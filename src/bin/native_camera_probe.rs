//! Experimental single-session libcamera probe. Run only with the camera UI stopped.
//! It never touches the framebuffer or the user's photo directory.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("native-camera-probe requires Linux and libcamera development files");
}

#[cfg(target_os = "linux")]
#[path = "../native_yuv.rs"]
mod native_yuv;

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::OpenOptions;
    use std::io::{self, Write};
    use std::thread;
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

    use native_yuv::{Yuv420Layout, copy_yuv420};
    use turbojpeg::{Compressor, Subsamp, YuvPlanesImage};

    let filename = std::env::args()
        .nth(1)
        .ok_or("usage: native-camera-probe /tmp/native-probe.jpg")?;
    let started = Instant::now();
    let manager = CameraManager::new()?;
    let cameras = manager.cameras();
    let camera = cameras
        .get(0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no camera found"))?;
    let mut camera = camera.acquire()?;

    // One configuration owns a full-resolution processed still and a small
    // viewfinder stream. It should stay active through the shutter event.
    let mut config = camera
        .generate_configuration(&[StreamRole::StillCapture, StreamRole::ViewFinder])
        .ok_or_else(|| io::Error::other("cannot generate two-stream camera configuration"))?;
    for (index, size) in [(0, Size::new(4056, 3040)), (1, Size::new(720, 540))] {
        let mut stream_config = config
            .get_mut(index)
            .ok_or_else(|| io::Error::other("missing requested stream"))?;
        stream_config.set_pixel_format(formats::YUV420);
        stream_config.set_size(size);
        stream_config.set_color_space(Some(ColorSpace::sycc()));
        stream_config.set_buffer_count(2);
    }
    let status = config.validate();
    if status.is_invalid() {
        return Err(io::Error::other("invalid still/viewfinder configuration").into());
    }
    if status.is_adjusted() {
        eprintln!("libcamera adjusted the requested configuration: {config:#?}");
    }
    for (index, expected) in [(0, Size::new(4056, 3040)), (1, Size::new(720, 540))] {
        let stream_config = config
            .get(index)
            .ok_or_else(|| io::Error::other("missing stream"))?;
        let actual = stream_config.get_size();
        if actual.width != expected.width
            || actual.height != expected.height
            || stream_config.get_pixel_format() != formats::YUV420
        {
            return Err(io::Error::other(format!(
                "stream {index} changed to {actual:?} / {:?}; test requires YUV420 at {expected:?}",
                stream_config.get_pixel_format()
            ))
            .into());
        }
    }
    camera.configure(&mut config)?;
    let still_config = config
        .get(0)
        .ok_or_else(|| io::Error::other("missing still stream"))?;
    let view_config = config
        .get(1)
        .ok_or_else(|| io::Error::other("missing viewfinder stream"))?;
    let still_stream = still_config
        .stream()
        .ok_or_else(|| io::Error::other("still stream unavailable"))?;
    let view_stream = view_config
        .stream()
        .ok_or_else(|| io::Error::other("viewfinder stream unavailable"))?;
    let still_stride = still_config.get_stride() as usize;
    let view_stride = view_config.get_stride() as usize;
    eprintln!(
        "configured still=4056x3040 stride={still_stride}, view=720x540 stride={view_stride} in {}ms",
        started.elapsed().as_millis()
    );

    let mut allocator = FrameBufferAllocator::new(&camera);
    let still_buffers = allocator.alloc(&still_stream)?;
    let view_buffers = allocator.alloc(&view_stream)?;
    if still_buffers.len() < 2 || view_buffers.len() < 2 {
        return Err(io::Error::other("need at least two buffers for each stream").into());
    }
    let mut requests = Vec::new();
    for (still, view) in still_buffers.into_iter().zip(view_buffers) {
        let mut request = camera
            .create_request(None)
            .ok_or_else(|| io::Error::other("cannot allocate camera request"))?;
        request.add_buffer(&still_stream, MemoryMappedFrameBuffer::new(still)?)?;
        request.add_buffer(&view_stream, MemoryMappedFrameBuffer::new(view)?)?;
        requests.push(request);
    }

    let completed = camera.subscribe_request_completed();
    camera.start(None)?;
    for request in requests {
        camera.queue_request(request).map_err(|(_, error)| error)?;
    }
    let mut previous_frame = None;
    let mut jpeg_job = None;
    for index in 0..20 {
        let mut request = completed.recv_timeout(Duration::from_secs(8))?;
        if request.status() != RequestStatus::Complete {
            return Err(io::Error::other("camera request cancelled").into());
        }
        let now = Instant::now();
        if let Some(previous) = previous_frame.replace(now) {
            let interval: Duration = now.duration_since(previous);
            eprintln!("view frame {index} interval={}ms", interval.as_millis());
        } else {
            eprintln!("first view frame={}ms", started.elapsed().as_millis());
        }

        // Count preview frames without copying them to the framebuffer. The
        // final UI backend will use this stream for live view.
        let view: &MemoryMappedFrameBuffer<FrameBuffer> = request
            .buffer(&view_stream)
            .ok_or_else(|| io::Error::other("viewfinder buffer missing"))?;
        if view.data().is_empty() {
            return Err(io::Error::other("empty viewfinder frame").into());
        }

        if index == 12 {
            let capture_started = Instant::now();
            let still: &MemoryMappedFrameBuffer<FrameBuffer> = request
                .buffer(&still_stream)
                .ok_or_else(|| io::Error::other("still buffer missing"))?;
            let planes = still.data();
            let layout = Yuv420Layout {
                width: 4056,
                height: 3040,
                y_stride: still_stride,
                uv_stride: still_stride / 2,
            };
            let frame = copy_yuv420(&planes, layout)?;
            let output_path = filename.clone();
            let copy_ms = capture_started.elapsed().as_millis();
            jpeg_job = Some(thread::spawn(move || -> io::Result<(u128, u128)> {
                let mut compressor =
                    Compressor::new().map_err(|e| io::Error::other(e.to_string()))?;
                compressor
                    .set_quality(95)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let image = YuvPlanesImage {
                    y_plane: frame.y.as_slice(),
                    u_plane: frame.u.as_slice(),
                    v_plane: frame.v.as_slice(),
                    width: layout.width,
                    height: layout.height,
                    y_stride: layout.y_stride,
                    u_stride: layout.uv_stride,
                    v_stride: layout.uv_stride,
                    subsamp: Subsamp::Sub2x2,
                };
                let jpeg = compressor
                    .compress_yuv_planes_to_owned(&image)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let encode_ms = capture_started.elapsed().as_millis();
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output_path)?;
                output.write_all(&jpeg)?;
                Ok((encode_ms, capture_started.elapsed().as_millis()))
            }));
            eprintln!(
                "single-session JPEG queued after YUV copy={copy_ms}ms; total since start={}ms",
                started.elapsed().as_millis()
            );
        }
        request.reuse(ReuseFlag::REUSE_BUFFERS);
        camera.queue_request(request).map_err(|(_, error)| error)?;
    }
    if let Some(job) = jpeg_job {
        let (encode_ms, save_ms) = job
            .join()
            .map_err(|_| io::Error::other("JPEG worker panicked"))??;
        eprintln!("single-session JPEG encoded after {encode_ms}ms, saved after {save_ms}ms");
    }
    camera.stop()?;
    eprintln!("probe finished in {}ms", started.elapsed().as_millis());
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::io::Cursor;

    use turbojpeg::{Compressor, Subsamp, YuvPlanesImage};

    #[test]
    fn planar_yuv_encodes_as_decodable_jpeg() {
        let y = [128; 64];
        let u = [128; 16];
        let v = [128; 16];
        let image = YuvPlanesImage {
            y_plane: &y[..],
            u_plane: &u[..],
            v_plane: &v[..],
            width: 8,
            height: 8,
            y_stride: 8,
            u_stride: 4,
            v_stride: 4,
            subsamp: Subsamp::Sub2x2,
        };
        let mut compressor = Compressor::new().unwrap();
        compressor.set_quality(95).unwrap();
        let jpeg = compressor.compress_yuv_planes_to_owned(&image).unwrap();
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(&jpeg));
        let rgb = decoder.decode().unwrap();
        let info = decoder.info().unwrap();
        assert_eq!((info.width, info.height), (8, 8));
        assert!(rgb.iter().all(|value| (120..=136).contains(value)));
    }
}
