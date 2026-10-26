//! Camera access with a persistent native session and rpicam compatibility path.

use crate::settings::{
    ASPECT_RATIOS, BULB_INDEX, CameraSettings, DENOISE, ISOS, METERING, SHUTTERS, WHITE_BALANCES,
};
use std::fs;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

static ZSL_AVAILABLE: OnceLock<bool> = OnceLock::new();

pub fn prepare_still_capabilities() {
    let _ = ZSL_AVAILABLE.get_or_init(|| {
        Command::new("/usr/bin/rpicam-still")
            .arg("--help")
            .output()
            .ok()
            .is_some_and(|output| {
                let mut help = output.stdout;
                help.extend(output.stderr);
                help.windows(5).any(|part| part == b"--zsl")
            })
    });
}

fn validate(settings: &CameraSettings) -> io::Result<()> {
    if settings.aspect >= ASPECT_RATIOS.len()
        || settings.shutter >= SHUTTERS.len()
        || settings.shutter == BULB_INDEX
        || settings.iso >= ISOS.len()
        || settings.white_balance >= WHITE_BALANCES.len()
        || settings.metering >= METERING.len()
        || settings.denoise >= DENOISE.len()
        || settings.sharpness_tenths > 20
        || settings.contrast_tenths > 20
        || !(-10..=10).contains(&settings.brightness_tenths)
        || settings.saturation_tenths > 20
        || !(-30..=30).contains(&settings.ev_thirds)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid camera setting",
        ));
    }
    Ok(())
}

fn apply_settings(command: &mut Command, settings: &CameraSettings) {
    command
        .args(["--ev", &format!("{:.2}", settings.ev_value())])
        .args(["--metering", METERING[settings.metering].value])
        .args([
            "--sharpness",
            &settings.slider_label(crate::settings::Picker::Sharpness),
        ])
        .args([
            "--contrast",
            &settings.slider_label(crate::settings::Picker::Contrast),
        ])
        .args([
            "--brightness",
            &settings.slider_label(crate::settings::Picker::Brightness),
        ])
        .args(["--saturation", &settings.saturation_label()])
        .args(["--denoise", DENOISE[settings.denoise].value])
        .args(["--awb", WHITE_BALANCES[settings.white_balance].value]);
    if SHUTTERS[settings.shutter].value > 0 {
        command.args(["--shutter", &SHUTTERS[settings.shutter].value.to_string()]);
    }
    if ISOS[settings.iso].value > 0.0 {
        command.args(["--gain", &format!("{:.1}", ISOS[settings.iso].value)]);
    }
}

/// Ask rpicam to shut down cleanly, then force termination after four seconds.
/// Reaping is mandatory even after forced termination, so camera ownership cannot leak.
fn stop_child(child: &mut Child) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    // SAFETY: The PID belongs to our unreaped child. No memory is accessed.
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    let signal_error = if result < 0 {
        let error = io::Error::last_os_error();
        (error.raw_os_error() != Some(libc::ESRCH)).then_some(error)
    } else {
        None
    };
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if child.try_wait()?.is_some() {
            return signal_error.map_or(Ok(()), Err);
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    // A child may exit between try_wait and kill; always wait to collect its status.
    let kill_result = child.kill();
    let wait_result = child.wait();
    wait_result?;
    if let Err(error) = kill_result {
        if error.kind() != io::ErrorKind::InvalidInput && error.raw_os_error() != Some(libc::ESRCH)
        {
            return Err(error);
        }
    }
    signal_error.map_or(Ok(()), Err)
}

pub struct Preview {
    child: Option<Child>,
    output: Option<ChildStdout>,
    #[cfg(all(target_os = "linux", feature = "native-camera"))]
    native: Option<crate::native_camera::NativePreview>,
    pub width: usize,
    pub height: usize,
    pub luma_stride: usize,
    pub chroma_stride: usize,
    pub frame_size: usize,
}

impl Preview {
    /// Placeholder while a capture worker owns the camera process.
    pub fn inactive(
        width: usize,
        height: usize,
        luma_stride: usize,
        chroma_stride: usize,
        frame_size: usize,
    ) -> Self {
        Self {
            child: None,
            output: None,
            #[cfg(all(target_os = "linux", feature = "native-camera"))]
            native: None,
            width,
            height,
            luma_stride,
            chroma_stride,
            frame_size,
        }
    }

    pub fn start(settings: &CameraSettings, screen_width: usize) -> io::Result<Self> {
        validate(settings)?;
        if !(2..=16_384).contains(&screen_width) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid preview width",
            ));
        }
        let (width, height) = settings.preview_dimensions(screen_width);
        let luma_stride = (width + 63) & !63;
        let chroma_stride = luma_stride / 2;
        let frame_size = luma_stride * height * 3 / 2;
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if screen_width == 720 {
            match crate::native_camera::NativePreview::start(
                settings.clone(),
                width,
                height,
                luma_stride,
            ) {
                Ok(native) => {
                    return Ok(Self {
                        child: None,
                        output: None,
                        native: Some(native),
                        width,
                        height,
                        luma_stride,
                        chroma_stride,
                        frame_size,
                    });
                }
                Err(error) => {
                    eprintln!("camera-square-rs: native startup failed, using rpicam: {error}")
                }
            }
        }
        let mut command = Command::new("/usr/bin/rpicam-vid");
        command
            .args(["--timeout", "0", "--nopreview", "--codec", "yuv420"])
            .args([
                "--width",
                &width.to_string(),
                "--height",
                &height.to_string(),
            ])
            .args([
                "--framerate",
                &preview_framerate(settings).to_string(),
                "--flush",
                "--output",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        apply_settings(&mut command, settings);
        let mut child = command.spawn()?;
        let output = child.stdout.take();
        Ok(Self {
            child: Some(child),
            output,
            #[cfg(all(target_os = "linux", feature = "native-camera"))]
            native: None,
            width,
            height,
            luma_stride,
            chroma_stride,
            frame_size,
        })
    }

    /// The stdout descriptor can be polled together with the touch input descriptor.
    pub fn fd(&self) -> RawFd {
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(native) = &self.native {
            return native.fd();
        }
        self.output.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    pub fn is_native(&self) -> bool {
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        {
            self.native.is_some()
        }
        #[cfg(not(all(target_os = "linux", feature = "native-camera")))]
        {
            false
        }
    }

    pub fn capture_native(&self, path: PathBuf) -> io::Result<mpsc::Receiver<io::Result<()>>> {
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(native) = &self.native {
            return native.capture(path);
        }
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native camera is inactive",
        ))
    }

    /// Update an active native session without interrupting its preview.
    /// Returns false when the compatibility preview needs a restart instead.
    pub fn set_adjustments_live(&self, settings: &CameraSettings) -> io::Result<bool> {
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(native) = &self.native {
            native.set_adjustments(settings.clone())?;
            return Ok(true);
        }
        let _ = settings;
        Ok(false)
    }

    /// Update the native shutter control without restarting its preview.
    /// Returns false when the compatibility preview needs a restart instead.
    pub fn set_shutter_live(&self, index: usize) -> io::Result<bool> {
        if index >= SHUTTERS.len() || index == BULB_INDEX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid shutter speed",
            ));
        }
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(native) = &self.native {
            native.set_shutter(index)?;
            return Ok(true);
        }
        let _ = index;
        Ok(false)
    }

    /// Read one complete, stride-padded YUV420 frame after stdout becomes readable.
    pub fn read_frame(&mut self, frame: &mut [u8]) -> io::Result<()> {
        if frame.len() != self.frame_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "incorrect preview buffer size",
            ));
        }
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(native) = &self.native {
            return native.read_frame(frame);
        }
        self.output
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "preview stopped"))?
            .read_exact(frame)
    }

    pub fn stop(&mut self) -> io::Result<()> {
        #[cfg(all(target_os = "linux", feature = "native-camera"))]
        if let Some(mut native) = self.native.take() {
            return native.stop();
        }
        // Close stdout first so the camera process can release its pipe resources.
        self.output.take();
        match self.child.take() {
            Some(mut child) => stop_child(&mut child),
            None => Ok(()),
        }
    }
}

fn preview_framerate(settings: &CameraSettings) -> f64 {
    let exposure = SHUTTERS[settings.shutter].value;
    if exposure > 0 {
        (1_000_000.0 / f64::from((exposure + 10_000).max(83_333))).min(12.0)
    } else {
        12.0
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Capture to a .jpg path and, when requested, its sibling .dng path.
/// DNG-only capture keeps rpicam's accompanying JPEG in the gallery preview directory.
pub fn capture_jpeg(settings: &CameraSettings, output_path: impl AsRef<Path>) -> io::Result<()> {
    validate(settings)?;
    let output_path = output_path.as_ref();
    if output_path.extension().and_then(|ext| ext.to_str()) != Some("jpg") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture output must end in .jpg",
        ));
    }
    let raw_path = output_path.with_extension("dng");
    // Never overwrite an existing photograph, regardless of the selected capture format.
    for path in [output_path, raw_path.as_path()] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "capture output exists",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let aspect = ASPECT_RATIOS[settings.aspect];
    let zsl_enabled = if std::env::var_os("CAMERA_SQUARE_ENABLE_ZSL").is_some() {
        prepare_still_capabilities();
        *ZSL_AVAILABLE.get().expect("still capabilities probed")
    } else {
        false
    };
    let mut command = Command::new("/usr/bin/rpicam-still");
    // This is a new camera process, so automatic exposure starts over. Keep a
    // short warm-up by default and allow a longer one in difficult lighting.
    let configured_timeout_ms = std::env::var("CAMERA_SQUARE_STILL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value <= 5_000)
        .unwrap_or(350);
    let exposure_us = SHUTTERS[settings.shutter].value.max(0) as u32;
    let timeout_ms = configured_timeout_ms.max(exposure_us.div_ceil(1_000).saturating_add(350));
    command
        .args([
            "--nopreview",
            "--timeout",
            &timeout_ms.to_string(),
            "--quality",
            "95",
        ])
        .args([
            "--width",
            &aspect.still_width.to_string(),
            "--height",
            &aspect.still_height.to_string(),
        ])
        .arg("--output")
        .arg(output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    apply_settings(&mut command, settings);
    if zsl_enabled {
        command.arg("--zsl");
    }
    if settings.capture_format.captures_raw() {
        command.arg("--raw");
    }
    let started = Instant::now();
    let mut child = command.spawn()?;
    let spawned_at = Instant::now();
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = stop_child(&mut child);
            return Err(error);
        }
    };
    eprintln!(
        "camera-square-rs: still timing zsl={} timeout={}ms spawn={}ms process={}ms",
        zsl_enabled,
        timeout_ms,
        spawned_at.duration_since(started).as_millis(),
        spawned_at.elapsed().as_millis(),
    );
    if !status.success() {
        return Err(io::Error::other(format!("rpicam-still failed: {status}")));
    }
    if !output_path.is_file() || (settings.capture_format.captures_raw() && !raw_path.is_file()) {
        return Err(io::Error::other(
            "camera did not save all requested outputs",
        ));
    }
    if settings.capture_format == crate::settings::CaptureFormat::Dng {
        let gallery_directory = output_path
            .parent()
            .ok_or_else(|| io::Error::other("capture has no directory"))?
            .join(".gallery");
        fs::create_dir_all(&gallery_directory)?;
        fs::rename(
            output_path,
            gallery_directory.join(
                output_path
                    .file_name()
                    .ok_or_else(|| io::Error::other("capture has no filename"))?,
            ),
        )?;
    }
    Ok(())
}
