//! Linux multitouch event-device reader with screen-coordinate normalization.

use std::fs;
use std::io;
use std::mem;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

const EV_ABS: u16 = 0x03;
const EV_SYN: u16 = 0x00;
const SYN_REPORT: u16 = 0;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const CONTACTS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    time: libc::timeval,
    event_type: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Default)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    pub active: bool,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct TouchFrame {
    pub contacts: [Contact; CONTACTS],
    pub count: usize,
    pub primary: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct PositionRange {
    pub minimum: i32,
    pub maximum: i32,
}

impl PositionRange {
    pub fn normalize(self, raw: i32, pixels: usize) -> i32 {
        if self.maximum <= self.minimum || pixels <= 1 {
            return raw;
        }
        let clamped = i64::from(raw).clamp(i64::from(self.minimum), i64::from(self.maximum));
        (((clamped - i64::from(self.minimum)) * (pixels as i64 - 1))
            / (i64::from(self.maximum) - i64::from(self.minimum))) as i32
    }
}

pub struct TouchInput {
    fd: OwnedFd,
    x_range: PositionRange,
    y_range: PositionRange,
    current_slot: usize,
    contacts: [Contact; CONTACTS],
}

impl TouchInput {
    pub fn open() -> io::Result<Self> {
        let event_path: PathBuf = if let Some(path) = std::env::var_os("CAMERA_SQUARE_TOUCH_DEVICE")
        {
            path.into()
        } else {
            let devices = fs::read_to_string("/proc/bus/input/devices")?;
            let event = touch_event_name(&devices).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "touchscreen event device not found",
                )
            })?;
            format!("/dev/input/{event}").into()
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(event_path)?;
        let fd: OwnedFd = file.into();
        let x_range = query_abs_range(fd.as_raw_fd(), ABS_MT_POSITION_X)?;
        let y_range = query_abs_range(fd.as_raw_fd(), ABS_MT_POSITION_Y)?;
        Ok(Self {
            fd,
            x_range,
            y_range,
            current_slot: 0,
            contacts: [Contact::default(); CONTACTS],
        })
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn normalize(&self, x: i32, y: i32, width: usize, height: usize) -> (i32, i32) {
        (
            self.x_range.normalize(x, width),
            self.y_range.normalize(y, height),
        )
    }

    /// Drain all currently available input events and return complete SYN_REPORT frames.
    pub fn read_frames(&mut self) -> io::Result<Vec<TouchFrame>> {
        let mut frames = Vec::new();
        loop {
            let mut events = [InputEvent::default(); 64];
            // SAFETY: `events` is initialized writable storage for 64 properly
            // aligned input_event values, and read never writes past its byte size.
            let bytes = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    events.as_mut_ptr().cast::<libc::c_void>(),
                    mem::size_of::<[InputEvent; 64]>(),
                )
            };
            if bytes < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if bytes == 0 {
                break;
            }
            let bytes = bytes as usize;
            let event_size = mem::size_of::<InputEvent>();
            if bytes % event_size != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "partial input_event read",
                ));
            }
            for event in events.iter().take(bytes / event_size) {
                match (event.event_type, event.code) {
                    (EV_ABS, ABS_MT_SLOT) => {
                        self.current_slot = usize::try_from(event.value).unwrap_or(usize::MAX);
                    }
                    (EV_ABS, ABS_MT_TRACKING_ID) if self.current_slot < CONTACTS => {
                        self.contacts[self.current_slot].active = event.value >= 0;
                    }
                    (EV_ABS, ABS_MT_POSITION_X) if self.current_slot < CONTACTS => {
                        self.contacts[self.current_slot].x = event.value;
                    }
                    (EV_ABS, ABS_MT_POSITION_Y) if self.current_slot < CONTACTS => {
                        self.contacts[self.current_slot].y = event.value;
                    }
                    (EV_SYN, SYN_REPORT) => {
                        let count = self
                            .contacts
                            .iter()
                            .filter(|contact| contact.active)
                            .count();
                        let primary = if self.current_slot < CONTACTS
                            && self.contacts[self.current_slot].active
                        {
                            Some(self.current_slot)
                        } else {
                            self.contacts.iter().position(|contact| contact.active)
                        };
                        frames.push(TouchFrame {
                            contacts: self.contacts,
                            count,
                            primary,
                        });
                    }
                    _ => {}
                }
            }
        }
        Ok(frames)
    }
}

fn query_abs_range(fd: RawFd, axis: u16) -> io::Result<PositionRange> {
    let request = 0x8000_0000_u64
        | ((mem::size_of::<InputAbsInfo>() as u64) << 16)
        | (u64::from(b'E') << 8)
        | (0x40 + u64::from(axis));
    let mut info = InputAbsInfo::default();
    // SAFETY: request is EVIOCGABS(axis), and info has the matching UAPI layout.
    if unsafe { libc::ioctl(fd, request as libc::c_ulong, &mut info) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PositionRange {
        minimum: info.minimum,
        maximum: info.maximum,
    })
}

fn touch_event_name(devices: &str) -> Option<String> {
    for block in devices.split("\n\n") {
        let mut event_name = None;
        let mut has_touch_key = false;
        for line in block.lines() {
            if let Some(handlers) = line.strip_prefix("H: Handlers=") {
                event_name = handlers
                    .split_whitespace()
                    .find(|handler| handler.starts_with("event"))
                    .map(str::to_owned);
            } else if let Some(key_bits) = line.strip_prefix("B: KEY=") {
                let words: Vec<_> = key_bits.split_whitespace().collect();
                if words.len() > 5 {
                    has_touch_key = u64::from_str_radix(words[words.len() - 6], 16)
                        .is_ok_and(|bits| bits & 0x400 != 0);
                }
            }
        }
        if has_touch_key {
            if let Some(event_name) = event_name {
                return Some(event_name);
            }
        }
    }
    None
}

use std::os::unix::fs::OpenOptionsExt;
