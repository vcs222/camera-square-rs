//! Owns the Linux framebuffer mapping and an independently allocated RGB565 canvas.

#[cfg(not(target_os = "linux"))]
use std::io;

#[cfg(target_os = "linux")]
mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::mem;
    use std::os::fd::AsRawFd;
    use std::ptr;

    const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
    const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

    #[repr(C)]
    #[derive(Default)]
    struct Bitfield {
        offset: u32,
        length: u32,
        msb_right: u32,
    }

    // These structures mirror Linux uapi/linux/fb.h. Address fields in the fixed
    // info are unsigned long, which differs between 32-bit and 64-bit Linux.
    #[repr(C)]
    #[derive(Default)]
    struct VariableInfo {
        xres: u32,
        yres: u32,
        xres_virtual: u32,
        yres_virtual: u32,
        xoffset: u32,
        yoffset: u32,
        bits_per_pixel: u32,
        grayscale: u32,
        red: Bitfield,
        green: Bitfield,
        blue: Bitfield,
        transp: Bitfield,
        nonstd: u32,
        activate: u32,
        height: u32,
        width: u32,
        accel_flags: u32,
        pixclock: u32,
        left_margin: u32,
        right_margin: u32,
        upper_margin: u32,
        lower_margin: u32,
        hsync_len: u32,
        vsync_len: u32,
        sync: u32,
        vmode: u32,
        rotate: u32,
        colorspace: u32,
        reserved: [u32; 4],
    }

    #[repr(C)]
    #[derive(Default)]
    struct FixedInfo {
        id: [u8; 16],
        smem_start: libc::c_ulong,
        smem_len: u32,
        kind: u32,
        type_aux: u32,
        visual: u32,
        xpanstep: u16,
        ypanstep: u16,
        ywrapstep: u16,
        line_length: u32,
        mmio_start: libc::c_ulong,
        mmio_len: u32,
        accel: u32,
        capabilities: u16,
        reserved: [u16; 2],
    }

    pub struct Display {
        pub width: usize,
        pub height: usize,
        /// Pixels per row, including any kernel framebuffer padding.
        pub stride: usize,
        pub pixels: Vec<u16>,
        mapping: *mut libc::c_void,
        mapping_len: usize,
        visible_offset: usize,
        // Retain the descriptor for the lifetime of the mapping.
        _file: File,
    }

    fn invalid(message: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    impl Display {
        pub fn open() -> io::Result<Self> {
            let file = OpenOptions::new().read(true).write(true).open("/dev/fb0")?;
            let mut variable = VariableInfo::default();
            let mut fixed = FixedInfo::default();
            // SAFETY: the writable structures have the layout and size expected
            // by the framebuffer ioctl ABI, and file owns a valid descriptor.
            if unsafe { libc::ioctl(file.as_raw_fd(), FBIOGET_VSCREENINFO, &mut variable) } < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: same ABI and descriptor requirements as above.
            if unsafe { libc::ioctl(file.as_raw_fd(), FBIOGET_FSCREENINFO, &mut fixed) } < 0 {
                return Err(io::Error::last_os_error());
            }
            if variable.bits_per_pixel != 16
                || variable.grayscale != 0
                || variable.nonstd != 0
                || variable.red.offset != 11
                || variable.red.length != 5
                || variable.green.offset != 5
                || variable.green.length != 6
                || variable.blue.offset != 0
                || variable.blue.length != 5
                || variable.red.msb_right != 0
                || variable.green.msb_right != 0
                || variable.blue.msb_right != 0
                || variable.transp.length != 0
                || fixed.kind != 0
                || fixed.visual != 2
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "framebuffer must use packed true-color RGB565 pixels",
                ));
            }
            let width = variable.xres as usize;
            let height = variable.yres as usize;
            let line_bytes = fixed.line_length as usize;
            if width == 0 || height == 0 || line_bytes % mem::size_of::<u16>() != 0 {
                return Err(invalid("invalid framebuffer dimensions or row stride"));
            }
            let stride = line_bytes / mem::size_of::<u16>();
            let xoffset = variable.xoffset as usize;
            let yoffset = variable.yoffset as usize;
            if xoffset
                .checked_add(width)
                .is_none_or(|right| right > stride)
                || xoffset
                    .checked_add(width)
                    .is_none_or(|right| right > variable.xres_virtual as usize)
                || yoffset
                    .checked_add(height)
                    .is_none_or(|bottom| bottom > variable.yres_virtual as usize)
            {
                return Err(invalid(
                    "visible framebuffer extends outside virtual dimensions",
                ));
            }
            let visible_offset = yoffset
                .checked_mul(line_bytes)
                .and_then(|row| row.checked_add(xoffset * mem::size_of::<u16>()))
                .ok_or_else(|| invalid("framebuffer offset overflow"))?;
            let visible_end = (height - 1)
                .checked_mul(line_bytes)
                .and_then(|rows| rows.checked_add(visible_offset))
                .and_then(|last_row| {
                    width
                        .checked_mul(2)
                        .and_then(|bytes| last_row.checked_add(bytes))
                })
                .ok_or_else(|| invalid("framebuffer dimensions overflow"))?;
            let mapping_len = fixed.smem_len as usize;
            if visible_end > mapping_len {
                return Err(invalid("visible framebuffer exceeds mapped video memory"));
            }
            let length = stride
                .checked_mul(height)
                .ok_or_else(|| invalid("framebuffer allocation size overflow"))?;
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(length).map_err(|error| {
                io::Error::other(format!("allocate framebuffer canvas: {error}"))
            })?;
            pixels.resize(length, 0);
            // SAFETY: the driver provides the validated memory length; mmap
            // receives a live descriptor and returns a mapping owned by Display.
            let mapping = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    mapping_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    file.as_raw_fd(),
                    0,
                )
            };
            if mapping == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                width,
                height,
                stride,
                pixels,
                mapping,
                mapping_len,
                visible_offset,
                _file: file,
            })
        }

        pub fn present(&self) -> io::Result<()> {
            // The public canvas can be replaced by callers, so recheck its size
            // before accessing it. Only visible pixels are copied, never padding.
            let required = self
                .stride
                .checked_mul(self.height)
                .ok_or_else(|| invalid("framebuffer canvas dimensions overflow"))?;
            if self.pixels.len() < required || self.width > self.stride {
                return Err(invalid("framebuffer canvas is too small"));
            }
            let line_bytes = self
                .stride
                .checked_mul(2)
                .ok_or_else(|| invalid("framebuffer canvas stride overflow"))?;
            for row in 0..self.height {
                let source = row
                    .checked_mul(self.stride)
                    .ok_or_else(|| invalid("framebuffer source offset overflow"))?;
                let target = row
                    .checked_mul(line_bytes)
                    .and_then(|offset| offset.checked_add(self.visible_offset))
                    .ok_or_else(|| invalid("framebuffer target offset overflow"))?;
                let row_bytes = self
                    .width
                    .checked_mul(2)
                    .ok_or_else(|| invalid("framebuffer row size overflow"))?;
                if target
                    .checked_add(row_bytes)
                    .is_none_or(|end| end > self.mapping_len)
                {
                    return Err(invalid("framebuffer copy exceeds mapped memory"));
                }
                // SAFETY: source and destination row bounds are checked above,
                // the canvas and mmap are separate allocations, and u8 pointers
                // avoid relying on framebuffer address alignment.
                unsafe {
                    ptr::copy_nonoverlapping(
                        self.pixels.as_ptr().add(source).cast::<u8>(),
                        self.mapping.cast::<u8>().add(target),
                        row_bytes,
                    );
                }
            }
            Ok(())
        }
    }

    impl Drop for Display {
        fn drop(&mut self) {
            // SAFETY: this object owns exactly the mapping returned by mmap.
            unsafe {
                libc::munmap(self.mapping, self.mapping_len);
            }
            // _file closes automatically after this drop method returns.
        }
    }
}

#[cfg(target_os = "linux")]
pub use platform::Display;

#[cfg(not(target_os = "linux"))]
pub struct Display {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixels: Vec<u16>,
}

#[cfg(not(target_os = "linux"))]
impl Display {
    pub fn open() -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux framebuffer required",
        ))
    }

    pub fn present(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux framebuffer required",
        ))
    }
}
