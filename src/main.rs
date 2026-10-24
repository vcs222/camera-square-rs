#![forbid(unsafe_op_in_unsafe_fn)]

mod app;
mod camera;
mod display;
mod draw;
mod gallery;
mod input;
mod render;
mod settings;
mod state;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("camera-square-rs: {error}");
        std::process::exit(1);
    }
}
