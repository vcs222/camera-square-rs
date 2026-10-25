//! Test the UI's native preview/capture bridge without opening the touchscreen.

#[cfg(target_os = "linux")]
#[path = "../native_camera.rs"]
mod native_camera;
#[cfg(target_os = "linux")]
#[path = "../native_yuv.rs"]
mod native_yuv;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("native-ui-smoke requires Linux");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io;
    use std::os::fd::RawFd;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: native-ui-smoke /tmp/native-ui-smoke.jpg")?;
    let mut preview = native_camera::NativePreview::start(768 * 540 * 3 / 2)?;
    let mut frame = vec![0; 768 * 540 * 3 / 2];
    let mut capture = None;
    let mut previous = None;
    for index in 0..25 {
        let mut descriptor = libc::pollfd {
            fd: preview.fd() as RawFd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor is an initialized pollfd and remains valid for this call.
        let ready = unsafe { libc::poll(&mut descriptor, 1, 5_000) };
        if ready <= 0 {
            return Err(io::Error::other(format!("viewfinder timeout at frame {index}")).into());
        }
        preview.read_frame(&mut frame)?;
        let now = Instant::now();
        if let Some(previous) = previous.replace(now) {
            let elapsed: Duration = now.duration_since(previous);
            eprintln!(
                "UI preview frame {index} interval={}ms",
                elapsed.as_millis()
            );
        }
        if index == 12 {
            capture = Some((now, preview.capture(path.clone())?));
        }
    }
    if let Some((started, reply)) = capture {
        reply.recv_timeout(Duration::from_secs(20))??;
        eprintln!(
            "UI native JPEG saved after {}ms",
            started.elapsed().as_millis()
        );
    }
    preview.stop()?;
    Ok(())
}
