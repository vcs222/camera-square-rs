#![forbid(unsafe_op_in_unsafe_fn)]

mod app;
mod camera;
mod display;
mod draw;
mod gallery;
mod input;
#[cfg(any(test, all(target_os = "linux", feature = "native-camera")))]
mod jpeg_exif;
#[cfg(all(target_os = "linux", feature = "native-camera"))]
mod native_camera;
#[cfg(all(target_os = "linux", feature = "native-camera"))]
mod native_dng;
mod render;
mod settings;
mod state;

#[cfg(any(test, all(target_os = "linux", feature = "native-camera")))]
mod native_yuv;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("camera-square-rs: {error}");
        std::process::exit(1);
    }
}
