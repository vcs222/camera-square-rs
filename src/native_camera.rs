//! Persistent libcamera session for JPEG and raw camera settings.
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
    controls::{
        AeMeteringMode, AnalogueGain, AnalogueGainMode, AwbMode, Brightness,
        ColourCorrectionMatrix, ColourGains, Contrast, ControlId, ExposureTime, ExposureTimeMode,
        ExposureValue, NoiseReductionMode, Saturation, SensorBlackLevels, Sharpness,
    },
    formats,
    framebuffer_allocator::{FrameBuffer, FrameBufferAllocator},
    framebuffer_map::MemoryMappedFrameBuffer,
    geometry::Size,
    properties::Model,
    request::{Request, RequestStatus, ReuseFlag},
    stream::StreamRole,
};
use turbojpeg::{Compressor, Subsamp, YuvPlanesImage};

use crate::jpeg_exif::{PhotoMetadata, add_exif};
use crate::native_dng::{RawCapture, save_dng};
use crate::native_yuv::{OwnedYuv420, Yuv420Layout, copy_yuv420};
use crate::settings::{ASPECT_RATIOS, CameraSettings, CaptureFormat, ISOS, SHUTTERS};

#[derive(Clone, Copy)]
struct NativeSize {
    still_width: usize,
    still_height: usize,
    view_width: usize,
    view_height: usize,
    view_stride: usize,
}

struct CapturedFrames {
    still: Option<OwnedYuv420>,
    gallery: Option<Vec<u8>>,
    raw: Option<RawCapture>,
}

enum Command {
    Capture {
        path: PathBuf,
        reply: mpsc::Sender<io::Result<()>>,
    },
    SetSaturation(u8),
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
    pub fn start(
        settings: CameraSettings,
        view_width: usize,
        view_height: usize,
        view_stride: usize,
    ) -> io::Result<Self> {
        let aspect = ASPECT_RATIOS[settings.aspect];
        let size = NativeSize {
            still_width: aspect.still_width as usize,
            still_height: aspect.still_height as usize,
            view_width,
            view_height,
            view_stride,
        };
        let frame_size = view_stride * view_height * 3 / 2;
        if view_width % 2 != 0 || view_height % 2 != 0 || view_stride < view_width {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid native preview dimensions",
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
                settings,
                size,
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

    pub fn set_saturation(&self, tenths: u8) -> io::Result<()> {
        self.commands
            .send(Command::SetSaturation(tenths))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "native camera stopped"))
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

fn apply_controls(request: &mut Request, settings: &CameraSettings) -> io::Result<()> {
    let controls = request.controls_mut();
    let exposure_mode = if settings.shutter == 0 {
        ExposureTimeMode::Auto
    } else {
        ExposureTimeMode::Manual
    };
    let gain_mode = if settings.iso == 0 {
        AnalogueGainMode::Auto
    } else {
        AnalogueGainMode::Manual
    };
    controls.set(exposure_mode).map_err(camera_error)?;
    controls.set(gain_mode).map_err(camera_error)?;
    if settings.shutter != 0 {
        controls
            .set(ExposureTime(SHUTTERS[settings.shutter].value))
            .map_err(camera_error)?;
    }
    if settings.iso != 0 {
        controls
            .set(AnalogueGain(ISOS[settings.iso].value))
            .map_err(camera_error)?;
    }
    let awb = match settings.white_balance {
        0 => AwbMode::AwbAuto,
        1 => AwbMode::AwbDaylight,
        2 => AwbMode::AwbCloudy,
        3 => AwbMode::AwbIndoor,
        4 => AwbMode::AwbFluorescent,
        _ => AwbMode::AwbTungsten,
    };
    controls.set(awb).map_err(camera_error)?;
    let metering = match settings.metering {
        0 => AeMeteringMode::MeteringCentreWeighted,
        1 => AeMeteringMode::MeteringSpot,
        _ => AeMeteringMode::MeteringMatrix,
    };
    controls.set(metering).map_err(camera_error)?;
    controls
        .set(ExposureValue(settings.ev_value()))
        .map_err(camera_error)?;
    controls
        .set(Sharpness(settings.sharpness as f32 / 2.0))
        .map_err(camera_error)?;
    controls
        .set(Contrast(settings.contrast as f32 / 2.0))
        .map_err(camera_error)?;
    controls
        .set(Brightness(settings.brightness as f32 / 2.0 - 1.0))
        .map_err(camera_error)?;
    controls
        .set(Saturation(settings.saturation_value()))
        .map_err(camera_error)?;
    if settings.denoise != 0 {
        let mode = match settings.denoise {
            1 => NoiseReductionMode::Off,
            2 => NoiseReductionMode::Minimal,
            3 => NoiseReductionMode::Fast,
            _ => NoiseReductionMode::HighQuality,
        };
        controls.set(mode).map_err(camera_error)?;
    }
    Ok(())
}

fn camera_loop(
    event: Arc<OwnedFd>,
    latest: Arc<Mutex<Vec<u8>>>,
    commands: mpsc::Receiver<Command>,
    ready: &mpsc::SyncSender<io::Result<()>>,
    mut settings: CameraSettings,
    size: NativeSize,
) -> io::Result<()> {
    let manager = CameraManager::new().map_err(camera_error)?;
    let cameras = manager.cameras();
    let camera = cameras
        .get(0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no camera found"))?;
    let mut camera = camera.acquire().map_err(camera_error)?;
    let model = camera
        .properties()
        .get::<Model>()
        .map(|value| value.0)
        .unwrap_or_else(|_| "Raspberry Pi Camera".to_string());
    if settings.denoise != 0
        && camera
            .controls()
            .count(ControlId::NoiseReductionMode as u32)
            == 0
    {
        return Err(io::Error::other("native denoise control unavailable"));
    }
    let wants_raw = settings.capture_format.captures_raw();
    let roles = if wants_raw {
        vec![
            StreamRole::StillCapture,
            StreamRole::ViewFinder,
            StreamRole::Raw,
        ]
    } else {
        vec![StreamRole::StillCapture, StreamRole::ViewFinder]
    };
    let mut config = camera
        .generate_configuration(&roles)
        .ok_or_else(|| io::Error::other("cannot generate native camera configuration"))?;
    for (index, size) in [
        (
            0,
            Size::new(size.still_width as u32, size.still_height as u32),
        ),
        (
            1,
            Size::new(size.view_width as u32, size.view_height as u32),
        ),
    ] {
        let mut stream = config
            .get_mut(index)
            .ok_or_else(|| io::Error::other("missing native camera stream"))?;
        stream.set_pixel_format(formats::YUV420);
        stream.set_size(size);
        stream.set_color_space(Some(ColorSpace::sycc()));
        stream.set_buffer_count(2);
    }
    if wants_raw {
        let mut raw = config
            .get_mut(2)
            .ok_or_else(|| io::Error::other("missing native raw stream"))?;
        raw.set_pixel_format(formats::SBGGR12);
        raw.set_size(Size::new(4056, 3040));
        raw.set_buffer_count(2);
    }
    let status = config.validate();
    if status.is_invalid() {
        return Err(io::Error::other("invalid native camera configuration"));
    }
    for (index, expected) in [
        (
            0,
            Size::new(size.still_width as u32, size.still_height as u32),
        ),
        (
            1,
            Size::new(size.view_width as u32, size.view_height as u32),
        ),
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
    let raw_config = if wants_raw {
        Some(
            config
                .get(2)
                .ok_or_else(|| io::Error::other("missing validated raw stream"))?,
        )
    } else {
        None
    };
    let raw_layout = raw_config.and_then(|raw| {
        let format = raw.get_pixel_format();
        let cfa = if format == formats::SBGGR12 {
            Some([2, 1, 1, 0])
        } else if format == formats::SRGGB12 {
            Some([0, 1, 1, 2])
        } else if format == formats::SGRBG12 {
            Some([1, 0, 2, 1])
        } else if format == formats::SGBRG12 {
            Some([1, 2, 0, 1])
        } else {
            None
        };
        cfa.map(|cfa| (raw.get_size(), raw.get_stride() as usize, cfa))
    });
    if wants_raw && raw_layout.is_none() {
        return Err(io::Error::other(
            "native raw stream is not unpacked 12-bit Bayer",
        ));
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
    if view_stride != size.view_stride || still_stride % 2 != 0 {
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
    let raw_stream = if wants_raw {
        Some(
            config
                .get(2)
                .and_then(|raw| raw.stream())
                .ok_or_else(|| io::Error::other("missing raw stream handle"))?,
        )
    } else {
        None
    };
    let mut allocator = FrameBufferAllocator::new(&camera);
    let still_buffers = allocator.alloc(&still_stream).map_err(camera_error)?;
    let view_buffers = allocator.alloc(&view_stream).map_err(camera_error)?;
    let mut raw_buffers = if let Some(raw) = &raw_stream {
        allocator.alloc(raw).map_err(camera_error)?.into_iter()
    } else {
        Vec::new().into_iter()
    };
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
        if let Some(raw) = &raw_stream {
            let buffer = raw_buffers
                .next()
                .ok_or_else(|| io::Error::other("not enough raw buffers"))?;
            request
                .add_buffer(
                    raw,
                    MemoryMappedFrameBuffer::new(buffer).map_err(camera_error)?,
                )
                .map_err(camera_error)?;
        }
        requests.push(request);
    }
    let completed = camera.subscribe_request_completed();
    camera.start(None).map_err(camera_error)?;
    for mut request in requests {
        apply_controls(&mut request, &settings)?;
        camera
            .queue_request(request)
            .map_err(|(_, error)| camera_error(error))?;
    }
    ready.send(Ok(())).map_err(camera_error)?;
    eprintln!(
        "camera-square-rs: native camera ready; still stride={still_stride}, view stride={view_stride}"
    );
    let mut pending_capture: Option<(PathBuf, mpsc::Sender<io::Result<()>>)> = None;
    let mut capture_worker: Option<JoinHandle<()>> = None;
    'camera: loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => break 'camera,
                Ok(Command::Capture { path, reply }) => {
                    if pending_capture.is_some() || capture_worker.is_some() {
                        let _ = reply.send(Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "capture already in progress",
                        )));
                    } else {
                        pending_capture = Some((path, reply));
                    }
                }
                Ok(Command::SetSaturation(tenths)) => {
                    settings.saturation_tenths = tenths;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if capture_worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let _ = capture_worker
                .take()
                .expect("finished capture worker")
                .join();
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
            copy_view_frame(&view_planes, &mut slot, view_stride, size.view_height)?;
        }
        signal_frame(&event);
        if let Some((path, reply)) = pending_capture.take() {
            let started = Instant::now();
            let metadata = PhotoMetadata {
                width: size.still_width as u32,
                height: size.still_height as u32,
                model: model.clone(),
                exposure_us: request
                    .metadata()
                    .get::<ExposureTime>()
                    .ok()
                    .map(|value| value.0),
                analogue_gain: request
                    .metadata()
                    .get::<AnalogueGain>()
                    .ok()
                    .map(|value| value.0),
            };
            let frame = if settings.capture_format == CaptureFormat::Dng {
                None
            } else {
                let still: &MemoryMappedFrameBuffer<FrameBuffer> = request
                    .buffer(&still_stream)
                    .ok_or_else(|| io::Error::other("native still buffer missing"))?;
                Some(copy_yuv420(
                    &still.data(),
                    Yuv420Layout {
                        width: size.still_width,
                        height: size.still_height,
                        y_stride: still_stride,
                        uv_stride: still_stride / 2,
                    },
                )?)
            };
            let gallery_frame = if settings.capture_format == CaptureFormat::Dng {
                Some(
                    latest
                        .lock()
                        .map_err(|_| io::Error::other("native preview lock poisoned"))?
                        .clone(),
                )
            } else {
                None
            };
            let raw = if let (Some(stream), Some((dimensions, stride, cfa))) =
                (&raw_stream, raw_layout)
            {
                let buffer: &MemoryMappedFrameBuffer<FrameBuffer> = request
                    .buffer(stream)
                    .ok_or_else(|| io::Error::other("native raw buffer missing"))?;
                let planes = buffer.data();
                let bytes = planes
                    .first()
                    .ok_or_else(|| io::Error::other("missing raw plane"))?;
                let required = stride * dimensions.height as usize;
                if bytes.len() < required {
                    return Err(io::Error::other("short raw plane"));
                }
                Some(RawCapture {
                    data: bytes[..required].to_vec(),
                    width: dimensions.width as usize,
                    height: dimensions.height as usize,
                    stride,
                    cfa,
                    model: model.clone(),
                    black_levels: request
                        .metadata()
                        .get::<SensorBlackLevels>()
                        .ok()
                        .map(|value| value.0),
                    colour_gains: request
                        .metadata()
                        .get::<ColourGains>()
                        .ok()
                        .map(|value| value.0),
                    colour_matrix: request
                        .metadata()
                        .get::<ColourCorrectionMatrix>()
                        .ok()
                        .map(|value| value.0),
                    exposure_us: metadata.exposure_us,
                    analogue_gain: metadata.analogue_gain,
                })
            } else {
                None
            };
            eprintln!(
                "camera-square-rs: native frame copy={}ms",
                started.elapsed().as_millis()
            );
            let capture_format = settings.capture_format;
            capture_worker = Some(thread::spawn(move || {
                let result = save_capture(
                    path,
                    CapturedFrames {
                        still: frame,
                        gallery: gallery_frame,
                        raw,
                    },
                    capture_format,
                    still_stride,
                    size,
                    metadata,
                    started,
                );
                let _ = reply.send(result);
            }));
        }
        request.reuse(ReuseFlag::REUSE_BUFFERS);
        apply_controls(&mut request, &settings)?;
        camera
            .queue_request(request)
            .map_err(|(_, error)| camera_error(error))?;
    }
    camera.stop().map_err(camera_error)?;
    if let Some(worker) = capture_worker {
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

fn copy_view_frame(
    planes: &[&[u8]],
    output: &mut [u8],
    stride: usize,
    height: usize,
) -> io::Result<()> {
    let y_len = stride * height;
    let uv_len = stride / 2 * height / 2;
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

fn save_capture(
    path: PathBuf,
    frames: CapturedFrames,
    format: CaptureFormat,
    stride: usize,
    size: NativeSize,
    metadata: PhotoMetadata,
    started: Instant,
) -> io::Result<()> {
    let raw_path = path.with_extension("dng");
    if format.captures_raw() {
        save_dng(
            &raw_path,
            frames
                .raw
                .ok_or_else(|| io::Error::other("missing native raw capture"))?,
        )?;
        eprintln!(
            "camera-square-rs: native DNG saved={}ms path={}",
            started.elapsed().as_millis(),
            raw_path.display()
        );
    }
    if format == CaptureFormat::Dng {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("capture has no directory"))?
            .join(".gallery");
        std::fs::create_dir_all(&directory)?;
        let preview_path = directory.join(
            path.file_name()
                .ok_or_else(|| io::Error::other("capture has no filename"))?,
        );
        save_gallery_jpeg(
            preview_path,
            frames
                .gallery
                .ok_or_else(|| io::Error::other("missing gallery preview frame"))?,
            size,
            metadata,
            started,
        )?;
    } else {
        save_jpeg(
            path,
            frames
                .still
                .ok_or_else(|| io::Error::other("missing native still frame"))?,
            stride,
            size,
            metadata,
            started,
        )?;
    }
    Ok(())
}

fn save_gallery_jpeg(
    path: PathBuf,
    frame: Vec<u8>,
    size: NativeSize,
    mut metadata: PhotoMetadata,
    started: Instant,
) -> io::Result<()> {
    let y_len = size.view_stride * size.view_height;
    let uv_len = y_len / 4;
    if frame.len() != y_len + 2 * uv_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid gallery preview frame",
        ));
    }
    let mut compressor = Compressor::new().map_err(camera_error)?;
    compressor.set_quality(85).map_err(camera_error)?;
    let image = YuvPlanesImage {
        y_plane: &frame[..y_len],
        u_plane: &frame[y_len..y_len + uv_len],
        v_plane: &frame[y_len + uv_len..],
        width: size.view_width,
        height: size.view_height,
        y_stride: size.view_stride,
        u_stride: size.view_stride / 2,
        v_stride: size.view_stride / 2,
        subsamp: Subsamp::Sub2x2,
    };
    let jpeg = compressor
        .compress_yuv_planes_to_owned(&image)
        .map_err(camera_error)?;
    metadata.width = size.view_width as u32;
    metadata.height = size.view_height as u32;
    let jpeg = add_exif(&jpeg, &metadata)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    output.write_all(&jpeg)?;
    eprintln!(
        "camera-square-rs: native gallery preview saved={}ms path={}",
        started.elapsed().as_millis(),
        path.display()
    );
    Ok(())
}

fn save_jpeg(
    path: PathBuf,
    frame: OwnedYuv420,
    stride: usize,
    size: NativeSize,
    metadata: PhotoMetadata,
    started: Instant,
) -> io::Result<()> {
    let mut compressor = Compressor::new().map_err(camera_error)?;
    compressor.set_quality(95).map_err(camera_error)?;
    let image = YuvPlanesImage {
        y_plane: frame.y.as_slice(),
        u_plane: frame.u.as_slice(),
        v_plane: frame.v.as_slice(),
        width: size.still_width,
        height: size.still_height,
        y_stride: stride,
        u_stride: stride / 2,
        v_stride: stride / 2,
        subsamp: Subsamp::Sub2x2,
    };
    let jpeg = compressor
        .compress_yuv_planes_to_owned(&image)
        .map_err(camera_error)?;
    let jpeg = add_exif(&jpeg, &metadata)?;
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
