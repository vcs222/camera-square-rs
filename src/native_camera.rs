//! Opt-in persistent libcamera session for the default JPEG camera settings.
//! The camera stays on one thread; the UI receives the latest viewfinder frame
//! through an eventfd and requests a still with a channel.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use libcamera::{
    camera_manager::CameraManager,
    color_space::ColorSpace,
    formats,
    framebuffer_allocator::{FrameBuffer, FrameBufferAllocator},
    framebuffer_map::MemoryMappedFrameBuffer,
    geometry::Size,
    request::{RequestStatus, ReuseFlag},
    stream::StreamRole,
};
use turbojpeg::{Compressor, Subsamp, YuvPlanesImage};

use crate::native_yuv::{OwnedYuv420, Yuv420Layout, copy_yuv420};

const STILL_WIDTH: usize = 4056;
const STILL_HEIGHT: usize = 3040;
const VIEW_WIDTH: usize = 720;
const VIEW_HEIGHT: usize = 540;

enum Command {
    Capture {
        path: PathBuf,
        reply: mpsc::Sender<io::Result<()>>,
    },
    Stop,
}

pub struct NativePreview {
    event: Arc<OwnedFd>,
    latest: Arc<Mutex<Vec<u8>>>,
    failure: Arc<Mutex<Option<String>>>,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl NativePreview {
    pub fn start(frame_size: usize) -> io::Result<Self> {
        let expected_size = VIEW_WIDTH * VIEW_HEIGHT * 3 / 2;
        // The viewfinder has 768-byte Y rows, 384-byte chroma rows.
        let expected_padded_size = 768 * VIEW_HEIGHT * 3 / 2;
        if frame_size != expected_padded_size || frame_size < expected_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native preview requires 720x540 YUV420 with 768-byte stride",
            ));
        }
        // SAFETY: eventfd returns a fresh descriptor; OwnedFd takes ownership once.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let event = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let latest = Arc::new(Mutex::new(vec![0; frame_size]));
        let failure = Arc::new(Mutex::new(None));
        let (commands, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_event = Arc::clone(&event);
        let thread_latest = Arc::clone(&latest);
        let thread_failure = Arc::clone(&failure);
        let worker = thread::spawn(move || {
            if let Err(error) = camera_loop(
                Arc::clone(&thread_event),
                thread_latest,
                receiver,
                &ready_tx,
            ) {
                if let Ok(mut failure) = thread_failure.lock() {
                    *failure = Some(error.to_string());
                }
                signal_frame(&thread_event);
                let _ = ready_tx.send(Err(io::Error::other(error.to_string())));
                eprintln!("camera-square-rs: native camera: {error}");
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = commands.send(Command::Stop);
                let _ = worker.join();
                return Err(error);
            }
            Err(error) => {
                let _ = commands.send(Command::Stop);
                return Err(io::Error::other(format!("native camera startup: {error}")));
            }
        }
        Ok(Self {
            event,
            latest,
            failure,
            commands,
            worker: Some(worker),
        })
    }

    pub fn fd(&self) -> RawFd {
        self.event.as_raw_fd()
    }

    pub fn read_frame(&self, frame: &mut [u8]) -> io::Result<()> {
        let mut counter = 0_u64;
        // SAFETY: counter is an initialized u64 and eventfd writes exactly 8 bytes.
        let read = unsafe {
            libc::read(
                self.event.as_raw_fd(),
                (&mut counter as *mut u64).cast(),
                size_of::<u64>(),
            )
        };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error);
            }
        }
        if let Some(error) = self
            .failure
            .lock()
            .map_err(|_| io::Error::other("native camera failure lock poisoned"))?
            .as_ref()
        {
            return Err(io::Error::other(error.clone()));
        }
        let latest = self
            .latest
            .lock()
            .map_err(|_| io::Error::other("native preview frame lock poisoned"))?;
        if frame.len() != latest.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "incorrect native preview buffer size",
            ));
        }
        frame.copy_from_slice(&latest);
        Ok(())
    }

    pub fn capture(&self, path: PathBuf) -> io::Result<mpsc::Receiver<io::Result<()>>> {
        let (reply, receiver) = mpsc::channel();
        self.commands
            .send(Command::Capture { path, reply })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "native camera stopped"))?;
        Ok(receiver)
    }

    pub fn stop(&mut self) -> io::Result<()> {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("native camera thread panicked"))?;
        }
        Ok(())
    }
}

impl Drop for NativePreview {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn camera_loop(
    event: Arc<OwnedFd>,
    latest: Arc<Mutex<Vec<u8>>>,
    commands: mpsc::Receiver<Command>,
    ready: &mpsc::SyncSender<io::Result<()>>,
) -> io::Result<()> {
    let manager = CameraManager::new().map_err(camera_error)?;
    let cameras = manager.cameras();
    let camera = cameras
        .get(0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no camera found"))?;
    let mut camera = camera.acquire().map_err(camera_error)?;
    let mut config = camera
        .generate_configuration(&[StreamRole::StillCapture, StreamRole::ViewFinder])
        .ok_or_else(|| io::Error::other("cannot generate native camera configuration"))?;
    for (index, size) in [
        (0, Size::new(STILL_WIDTH as u32, STILL_HEIGHT as u32)),
        (1, Size::new(VIEW_WIDTH as u32, VIEW_HEIGHT as u32)),
    ] {
        let mut stream = config
            .get_mut(index)
            .ok_or_else(|| io::Error::other("missing native camera stream"))?;
        stream.set_pixel_format(formats::YUV420);
        stream.set_size(size);
        stream.set_color_space(Some(ColorSpace::sycc()));
        stream.set_buffer_count(2);
    }
    let status = config.validate();
    if status.is_invalid() {
        return Err(io::Error::other("invalid native camera configuration"));
    }
    for (index, expected) in [
        (0, Size::new(STILL_WIDTH as u32, STILL_HEIGHT as u32)),
        (1, Size::new(VIEW_WIDTH as u32, VIEW_HEIGHT as u32)),
    ] {
        let stream = config
            .get(index)
            .ok_or_else(|| io::Error::other("missing validated native stream"))?;
        let actual = stream.get_size();
        if actual.width != expected.width
            || actual.height != expected.height
            || stream.get_pixel_format() != formats::YUV420
        {
            return Err(io::Error::other(format!(
                "native stream {index} adjusted to {:?} / {:?}",
                stream.get_size(),
                stream.get_pixel_format()
            )));
        }
    }
    camera.configure(&mut config).map_err(camera_error)?;
    let still_config = config
        .get(0)
        .ok_or_else(|| io::Error::other("missing still stream"))?;
    let view_config = config
        .get(1)
        .ok_or_else(|| io::Error::other("missing view stream"))?;
    let still_stride = still_config.get_stride() as usize;
    let view_stride = view_config.get_stride() as usize;
    if view_stride != 768 || still_stride % 2 != 0 {
        return Err(io::Error::other(format!(
            "unsupported native camera strides: still={still_stride}, view={view_stride}"
        )));
    }
    let still_stream = still_config
        .stream()
        .ok_or_else(|| io::Error::other("missing still stream handle"))?;
    let view_stream = view_config
        .stream()
        .ok_or_else(|| io::Error::other("missing view stream handle"))?;
    let mut allocator = FrameBufferAllocator::new(&camera);
    let still_buffers = allocator.alloc(&still_stream).map_err(camera_error)?;
    let view_buffers = allocator.alloc(&view_stream).map_err(camera_error)?;
    if still_buffers.len() < 2 || view_buffers.len() < 2 {
        return Err(io::Error::other(
            "native camera needs two buffers per stream",
        ));
    }
    let mut requests = Vec::new();
    for (still, view) in still_buffers.into_iter().zip(view_buffers) {
        let mut request = camera
            .create_request(None)
            .ok_or_else(|| io::Error::other("cannot create native request"))?;
        request
            .add_buffer(
                &still_stream,
                MemoryMappedFrameBuffer::new(still).map_err(camera_error)?,
            )
            .map_err(camera_error)?;
        request
            .add_buffer(
                &view_stream,
                MemoryMappedFrameBuffer::new(view).map_err(camera_error)?,
            )
            .map_err(camera_error)?;
        requests.push(request);
    }
    let completed = camera.subscribe_request_completed();
    camera.start(None).map_err(camera_error)?;
    for request in requests {
        camera
            .queue_request(request)
            .map_err(|(_, error)| camera_error(error))?;
    }
    ready.send(Ok(())).map_err(camera_error)?;
    eprintln!(
        "camera-square-rs: native camera ready; still stride={still_stride}, view stride={view_stride}"
    );
    let mut pending_capture: Option<(PathBuf, mpsc::Sender<io::Result<()>>)> = None;
    let mut jpeg_worker: Option<JoinHandle<()>> = None;
    'camera: loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => break 'camera,
                Ok(Command::Capture { path, reply }) => {
                    if pending_capture.is_some() || jpeg_worker.is_some() {
                        let _ = reply.send(Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "capture already in progress",
                        )));
                    } else {
                        pending_capture = Some((path, reply));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if jpeg_worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let _ = jpeg_worker.take().expect("finished JPEG worker").join();
        }
        let mut request = match completed.recv_timeout(Duration::from_millis(120)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(error) => return Err(camera_error(error)),
        };
        if request.status() != RequestStatus::Complete {
            continue;
        }
        let view: &MemoryMappedFrameBuffer<FrameBuffer> = request
            .buffer(&view_stream)
            .ok_or_else(|| io::Error::other("native view buffer missing"))?;
        let view_planes = view.data();
        {
            let mut slot = latest
                .lock()
                .map_err(|_| io::Error::other("native preview lock poisoned"))?;
            copy_view_frame(&view_planes, &mut slot, view_stride)?;
        }
        signal_frame(&event);
        if let Some((path, reply)) = pending_capture.take() {
            let started = Instant::now();
            let still: &MemoryMappedFrameBuffer<FrameBuffer> = request
                .buffer(&still_stream)
                .ok_or_else(|| io::Error::other("native still buffer missing"))?;
            let frame = copy_yuv420(
                &still.data(),
                Yuv420Layout {
                    width: STILL_WIDTH,
                    height: STILL_HEIGHT,
                    y_stride: still_stride,
                    uv_stride: still_stride / 2,
                },
            )?;
            eprintln!(
                "camera-square-rs: native still copy={}ms",
                started.elapsed().as_millis()
            );
            jpeg_worker = Some(thread::spawn(move || {
                let result = save_jpeg(path, frame, still_stride, started);
                let _ = reply.send(result);
            }));
        }
        request.reuse(ReuseFlag::REUSE_BUFFERS);
        camera
            .queue_request(request)
            .map_err(|(_, error)| camera_error(error))?;
    }
    camera.stop().map_err(camera_error)?;
    if let Some(worker) = jpeg_worker {
        let _ = worker.join();
    }
    Ok(())
}

fn signal_frame(event: &OwnedFd) {
    let one = 1_u64;
    // SAFETY: eventfd reads one initialized u64; a full counter merely drops a wakeup.
    let _ = unsafe {
        libc::write(
            event.as_raw_fd(),
            (&one as *const u64).cast(),
            size_of::<u64>(),
        )
    };
}

fn copy_view_frame(planes: &[&[u8]], output: &mut [u8], stride: usize) -> io::Result<()> {
    let y_len = stride * VIEW_HEIGHT;
    let uv_len = stride / 2 * VIEW_HEIGHT / 2;
    if output.len() != y_len + 2 * uv_len {
        return Err(io::Error::other("native preview size mismatch"));
    }
    match planes {
        [combined] if combined.len() >= output.len() => {
            output.copy_from_slice(&combined[..output.len()])
        }
        [y, u, v] if y.len() >= y_len && u.len() >= uv_len && v.len() >= uv_len => {
            output[..y_len].copy_from_slice(&y[..y_len]);
            output[y_len..y_len + uv_len].copy_from_slice(&u[..uv_len]);
            output[y_len + uv_len..].copy_from_slice(&v[..uv_len]);
        }
        _ => return Err(io::Error::other("unsupported native preview plane layout")),
    }
    Ok(())
}

fn save_jpeg(path: PathBuf, frame: OwnedYuv420, stride: usize, started: Instant) -> io::Result<()> {
    let mut compressor = Compressor::new().map_err(camera_error)?;
    compressor.set_quality(95).map_err(camera_error)?;
    let image = YuvPlanesImage {
        y_plane: frame.y.as_slice(),
        u_plane: frame.u.as_slice(),
        v_plane: frame.v.as_slice(),
        width: STILL_WIDTH,
        height: STILL_HEIGHT,
        y_stride: stride,
        u_stride: stride / 2,
        v_stride: stride / 2,
        subsamp: Subsamp::Sub2x2,
    };
    let jpeg = compressor
        .compress_yuv_planes_to_owned(&image)
        .map_err(camera_error)?;
    let encoded_ms = started.elapsed().as_millis();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    output.write_all(&jpeg)?;
    eprintln!(
        "camera-square-rs: native JPEG encoded={}ms saved={}ms path={}",
        encoded_ms,
        started.elapsed().as_millis(),
        path.display()
    );
    Ok(())
}

fn camera_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
