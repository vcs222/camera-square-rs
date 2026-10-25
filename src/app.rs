//! Camera UI event loop and user actions.

use std::fs::{self, File};
use std::io;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::camera::{Preview, capture_jpeg};
use crate::display::Display;
use crate::draw::Canvas;
use crate::gallery;
use crate::input::{TouchFrame, TouchInput};
use crate::render::{
    self, PreviewFrameLayout, picker_is_top_row, picker_labels, picker_list_top, picker_scale,
    picker_selected, shutter_display_y,
};
use crate::settings::{
    ASPECT_RATIOS, CaptureFormat, DENOISE, HDR, ISOS, METERING, Picker, SHUTTERS, WHITE_BALANCES,
};
use crate::state::UiState;

static RUNNING: AtomicBool = AtomicBool::new(true);
fn camera_directory() -> PathBuf {
    std::env::var_os("CAMERA_SQUARE_CAMERA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_pictures_directory().join("Camera"))
}

fn debug_directory() -> PathBuf {
    std::env::var_os("CAMERA_SQUARE_DEBUG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_pictures_directory().join("Debug"))
}

fn user_pictures_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures")
}

extern "C" fn stop_signal(_: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug)]
enum Hit {
    Gallery,
    Screenshot,
    Control(Picker),
    Grid,
    Peak,
    Shutter,
    PickerChoice(usize),
    Blank,
}

#[derive(Clone, Copy)]
struct Gesture {
    target: Hit,
    slot: usize,
    start: (i32, i32),
    last: (i32, i32),
    started: Instant,
    max_contacts: usize,
    dragged: bool,
    shutter_offset: (i32, i32),
}

fn top_item_at(x: i32, y: i32, width: usize) -> Option<usize> {
    // Touches against the left bezel often normalize to x=0, while the
    // gallery icon is drawn a few pixels in from the edge.
    if (0..54).contains(&x) && (0..96).contains(&y) {
        return Some(0);
    }
    if (0..96).contains(&y) && x >= width as i32 - 60 {
        return Some(8);
    }
    if !(0..64).contains(&y) {
        return None;
    }
    let widths = [42, 38, 52, 64, 76, 112, 100, 108, 48];
    let gap = 8;
    let total: i32 = widths.iter().sum::<i32>() + gap * (widths.len() as i32 - 1);
    let mut left = (width as i32 - total) / 2;
    for (index, item_width) in widths.into_iter().enumerate() {
        if x >= left - gap / 2 && x < left + item_width + gap / 2 {
            return Some(index);
        }
        left += item_width + gap;
    }
    None
}

fn gallery_back_at(x: i32, y: i32) -> bool {
    (0..72).contains(&x) && (0..72).contains(&y)
}

fn bottom_picker(x: i32, y: i32, width: usize, height: usize) -> Option<Picker> {
    if y < height as i32 - 100 || width == 0 {
        return None;
    }
    Some(match (x.max(0) as usize * 8 / width).min(7) {
        0 => Picker::Shutter,
        1 => Picker::Iso,
        2 => Picker::WhiteBalance,
        3 => Picker::Metering,
        4 => Picker::Sharpness,
        5 => Picker::Ev,
        6 => Picker::Aspect,
        _ => Picker::CaptureFormat,
    })
}

fn hit_test(state: &UiState, x: i32, y: i32, width: usize, height: usize, now: Instant) -> Hit {
    if state.gallery_open {
        return Hit::Gallery;
    }
    if state.picker_is_open(now) && picker_is_top_row(state.picker) {
        if let Some(choice) = picker_choice_at(state, x, y, width, height) {
            return Hit::PickerChoice(choice);
        }
    }
    if let Some(item) = top_item_at(x, y, width) {
        return match item {
            0 => Hit::Gallery,
            1 => Hit::Screenshot,
            2 => Hit::Control(Picker::Contrast),
            3 => Hit::Control(Picker::Brightness),
            4 => Hit::Control(Picker::Saturation),
            5 => Hit::Control(Picker::Denoise),
            6 => Hit::Control(Picker::Hdr),
            7 => Hit::Grid,
            _ => Hit::Peak,
        };
    }
    if let Some(picker) = bottom_picker(x, y, width, height) {
        return Hit::Control(picker);
    }
    let mut shutter_distance = None;
    if x >= 0 && y >= 0 {
        let shutter_y = shutter_display_y(width, height, state, now);
        let dx = x - state.shutter_x;
        let dy = y - shutter_y;
        let distance = dx * dx + dy * dy;
        shutter_distance = Some(distance);
        if distance <= 36 * 36 {
            return Hit::Shutter;
        }
    }
    if state.picker_is_open(now) {
        if let Some(choice) = picker_choice_at(state, x, y, width, height) {
            return Hit::PickerChoice(choice);
        }
    }
    // The glass has less precise touches near its right edge, where the
    // shutter starts. Keep the drawn circle small but give it a finger-sized
    // target; picker choices above this target keep their own priority.
    if shutter_distance.is_some_and(|distance| distance <= 64 * 64) {
        return Hit::Shutter;
    }
    Hit::Blank
}

fn picker_choice_at(state: &UiState, x: i32, y: i32, width: usize, height: usize) -> Option<usize> {
    if x < 0 || x >= width as i32 {
        return None;
    }
    let labels = picker_labels(&state.camera, state.picker);
    if labels.is_empty() {
        return None;
    }
    let top = picker_list_top(width, height, &state.camera, state.picker);
    if state.picker == Picker::Ev {
        if y < top - 8 || y >= top + 40 {
            return None;
        }
        let slot = x as usize * 5 / width;
        let thirds = state.camera.ev_thirds as i32 + slot as i32 - 2;
        return (-30..=30)
            .contains(&thirds)
            .then_some((thirds + 30) as usize);
    }
    let scale = picker_scale(width, &state.camera, state.picker);
    if y < top - 8 || y >= top + 7 * scale + 8 {
        return None;
    }
    Some((x as usize * labels.len() / width).min(labels.len() - 1))
}

fn apply_picker(state: &mut UiState, index: usize) -> io::Result<bool> {
    let camera = &mut state.camera;
    let changed = match state.picker {
        Picker::Shutter if index < SHUTTERS.len() => replace(&mut camera.shutter, index),
        Picker::Iso if index < ISOS.len() => replace(&mut camera.iso, index),
        Picker::WhiteBalance if index < WHITE_BALANCES.len() => {
            replace(&mut camera.white_balance, index)
        }
        Picker::Metering if index < METERING.len() => replace(&mut camera.metering, index),
        Picker::Sharpness if index < 5 => replace(&mut camera.sharpness, index),
        Picker::Ev if index < 61 => replace(&mut camera.ev_thirds, index as i8 - 30),
        Picker::Aspect if index < ASPECT_RATIOS.len() => replace(&mut camera.aspect, index),
        Picker::CaptureFormat if index < CaptureFormat::ALL.len() => {
            replace(&mut camera.capture_format, CaptureFormat::ALL[index])
        }
        Picker::Contrast if index < 5 => replace(&mut camera.contrast, index),
        Picker::Brightness if index < 5 => replace(&mut camera.brightness, index),
        Picker::Saturation if index < 5 => replace(&mut camera.saturation, index),
        Picker::Denoise if index < DENOISE.len() => replace(&mut camera.denoise, index),
        Picker::Hdr if index < HDR.len() => replace(&mut camera.hdr, index),
        _ => return Ok(false),
    };
    state.refresh_picker_timeout();
    Ok(changed)
}

fn replace<T: Copy + PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}

fn trace_touch() -> bool {
    std::env::var_os("CAMERA_SQUARE_TRACE_TOUCH").is_some()
}

fn open_gallery(state: &mut UiState) {
    match gallery::scan(&camera_directory()) {
        Ok(photos) => {
            state.gallery = photos;
            state.gallery_index = state.gallery.len().checked_sub(1);
            if let Some(index) = state.gallery_index {
                match gallery::decode_rgb(&state.gallery[index].path) {
                    Ok((rgb, width, height)) => {
                        state.gallery_rgb = rgb;
                        state.gallery_dimensions = (width, height);
                    }
                    Err(error) => {
                        state.gallery_rgb.clear();
                        state.gallery_dimensions = (0, 0);
                        state.show_message(format!("GALLERY ERROR: {error}"), 2.0);
                    }
                }
            } else {
                state.gallery_rgb.clear();
                state.gallery_dimensions = (0, 0);
            }
            state.gallery_open = true;
        }
        Err(error) => state.show_message(format!("GALLERY ERROR: {error}"), 2.0),
    }
}

fn gallery_step(state: &mut UiState, delta: isize) {
    let Some(current) = state.gallery_index else {
        return;
    };
    let next = current as isize + delta;
    if next < 0 || next >= state.gallery.len() as isize {
        return;
    }
    let next = next as usize;
    if let Ok((rgb, width, height)) = gallery::decode_rgb(&state.gallery[next].path) {
        state.gallery_rgb = rgb;
        state.gallery_dimensions = (width, height);
        state.gallery_index = Some(next);
    }
}

fn capture_path() -> io::Result<PathBuf> {
    let directory = camera_directory();
    fs::create_dir_all(&directory)?;
    let mut time: libc::time_t = 0;
    // SAFETY: time accepts a pointer to writable time_t storage.
    unsafe { libc::time(&mut time) };
    // SAFETY: zero is a valid starting state and localtime_r initializes `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    let format = b"CS_%Y%m%d_%H%M%S\0";
    let mut name = [0 as libc::c_char; 64];
    // SAFETY: format and destination are NUL-terminated buffers with valid lengths.
    let written =
        unsafe { libc::strftime(name.as_mut_ptr(), name.len(), format.as_ptr().cast(), &tm) };
    if written == 0 {
        return Err(io::Error::other("could not format capture timestamp"));
    }
    // SAFETY: strftime produced a NUL-terminated ASCII string.
    let base = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy();
    for suffix in 0..100 {
        let stem = if suffix == 0 {
            base.to_string()
        } else {
            format!("{base}_{suffix:02}")
        };
        let path = directory.join(format!("{stem}.jpg"));
        if !path.exists() && !path.with_extension("dng").exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free capture filename",
    ))
}

fn take_photo(
    state: &mut UiState,
    preview: &mut Preview,
    display: &mut Display,
    frame_ready: &mut bool,
    capture_job: &mut Option<JoinHandle<CaptureOutcome>>,
    native_capture_job: &mut Option<NativeCaptureJob>,
) {
    if capture_job.is_some() || native_capture_job.is_some() {
        return;
    }
    let started = Instant::now();
    state.close_picker();
    display.pixels.fill(0);
    {
        let mut canvas = Canvas {
            pixels: &mut display.pixels,
            width: display.width,
            height: display.height,
            stride: display.stride,
        };
        canvas.text(
            display.width as i32 / 2 - 60,
            display.height as i32 / 2 - 4,
            "CAPTURING",
            2,
            0xffff,
        );
    }
    let _ = display.present();
    if preview.is_native() {
        match capture_path().and_then(|path| preview.capture_native(path)) {
            Ok(reply) => {
                state.show_message("CAPTURING", 2.0);
                *native_capture_job = Some(NativeCaptureJob { reply, started });
            }
            Err(error) => {
                eprintln!("camera-square-rs: native capture request: {error}");
                state.show_message("CAPTURE FAILED", 2.0);
            }
        }
        return;
    }
    let owned_preview = std::mem::replace(
        preview,
        Preview::inactive(
            preview.width,
            preview.height,
            preview.luma_stride,
            preview.chroma_stride,
            preview.frame_size,
        ),
    );
    let settings = state.camera.clone();
    let screen_width = display.width;
    *frame_ready = false;
    *capture_job = Some(thread::spawn(move || {
        let mut owned_preview = owned_preview;
        let stop_error = owned_preview.stop().err();
        let stopped_at = Instant::now();
        let save_result = capture_path().and_then(|path| capture_jpeg(&settings, path));
        let saved_at = Instant::now();
        let next_preview = Preview::start(&settings, screen_width);
        let restarted_at = Instant::now();
        eprintln!(
            "camera-square-rs: capture timing stop={}ms still={}ms restart={}ms total={}ms",
            stopped_at.duration_since(started).as_millis(),
            saved_at.duration_since(stopped_at).as_millis(),
            restarted_at.duration_since(saved_at).as_millis(),
            restarted_at.duration_since(started).as_millis(),
        );
        CaptureOutcome {
            next_preview,
            save_result,
            stop_error,
            started,
        }
    }));
}

struct CaptureOutcome {
    next_preview: io::Result<Preview>,
    save_result: io::Result<()>,
    stop_error: Option<io::Error>,
    started: Instant,
}

struct NativeCaptureJob {
    reply: Receiver<io::Result<()>>,
    started: Instant,
}

fn save_screenshot(state: &mut UiState, display: &Display) -> io::Result<()> {
    let directory = debug_directory();
    fs::create_dir_all(&directory)?;
    let stem = capture_timestamp("SCREEN")?;
    let path = unique_jpeg_path(&directory, &stem)?;
    let mut rgb_data = Vec::with_capacity(display.width * display.height * 3);
    for y in 0..display.height {
        let row = y * display.stride;
        for x in 0..display.width {
            let px = display.pixels[row + x];
            rgb_data.extend_from_slice(&[
                (((px >> 11) & 31) << 3) as u8,
                (((px >> 5) & 63) << 2) as u8,
                ((px & 31) << 3) as u8,
            ]);
        }
    }
    let file = File::create(path)?;
    jpeg_encoder::Encoder::new(file, 90)
        .encode(
            &rgb_data,
            display.width as u16,
            display.height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(io::Error::other)?;
    state.screenshot_index = state.screenshot_index.wrapping_add(1);
    Ok(())
}

fn capture_timestamp(prefix: &str) -> io::Result<String> {
    let mut time: libc::time_t = 0;
    // SAFETY: time accepts a pointer to writable time_t storage.
    unsafe { libc::time(&mut time) };
    // SAFETY: localtime_r initializes the provided tm value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    let format = b"%Y%m%d_%H%M%S\0";
    let mut name = [0 as libc::c_char; 64];
    // SAFETY: strftime writes into a bounded destination using the format string.
    let written =
        unsafe { libc::strftime(name.as_mut_ptr(), name.len(), format.as_ptr().cast(), &tm) };
    if written == 0 {
        return Err(io::Error::other("could not format timestamp"));
    }
    // SAFETY: strftime wrote a NUL-terminated ASCII string.
    Ok(format!(
        "{prefix}_{}",
        unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy()
    ))
}

fn unique_jpeg_path(directory: &Path, stem: &str) -> io::Result<PathBuf> {
    for suffix in 0..100 {
        let name = if suffix == 0 {
            format!("{stem}.jpg")
        } else {
            format!("{stem}_{suffix:02}.jpg")
        };
        let path = directory.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free screenshot filename",
    ))
}

fn render_current(
    display: &mut Display,
    state: &UiState,
    preview: &Preview,
    frame: &[u8],
    frame_ready: bool,
    background: &mut Vec<u16>,
) -> io::Result<()> {
    if !state.gallery_open {
        if frame_ready && frame.len() == preview.frame_size {
            let mut canvas = Canvas {
                pixels: &mut display.pixels,
                width: display.width,
                height: display.height,
                stride: display.stride,
            };
            render::render_preview(
                &mut canvas,
                frame,
                PreviewFrameLayout {
                    width: preview.width,
                    height: preview.height,
                    luma_stride: preview.luma_stride,
                    chroma_stride: preview.chroma_stride,
                },
                &state.camera,
                state.focus_peaking,
            );
            background.clone_from(&display.pixels);
        } else if background.len() == display.pixels.len() {
            display.pixels.copy_from_slice(background);
        } else {
            display.pixels.fill(0);
        }
    }
    {
        let mut canvas = Canvas {
            pixels: &mut display.pixels,
            width: display.width,
            height: display.height,
            stride: display.stride,
        };
        render::render_overlays(&mut canvas, state, Instant::now());
    }
    display.present()
}

fn restart_preview(
    state: &mut UiState,
    preview: &mut Preview,
    display: &Display,
    frame: &mut Vec<u8>,
    retry_at: &mut Instant,
    frame_ready: &mut bool,
) {
    *frame_ready = false;
    let _ = preview.stop();
    match Preview::start(&state.camera, display.width) {
        Ok(new_preview) => {
            *frame = vec![0; new_preview.frame_size];
            *preview = new_preview;
            *retry_at = Instant::now() + Duration::from_secs(1);
        }
        Err(error) => {
            eprintln!("camera-square-rs: restart preview: {error}");
            frame.fill(0);
            state.show_message("CAMERA RETRY", 2.0);
            *retry_at = Instant::now() + Duration::from_secs(1);
        }
    }
}

struct TouchContext<'a> {
    touch: &'a TouchInput,
    state: &'a mut UiState,
    preview: &'a mut Preview,
    display: &'a mut Display,
    camera_frame: &'a mut Vec<u8>,
    preview_retry_at: &'a mut Instant,
    frame_ready: &'a mut bool,
    background: &'a mut Vec<u16>,
    capture_job: &'a mut Option<JoinHandle<CaptureOutcome>>,
    native_capture_job: &'a mut Option<NativeCaptureJob>,
}

fn apply_touch_frame(
    frame: TouchFrame,
    previous_count: &mut usize,
    gesture: &mut Option<Gesture>,
    max_contacts: &mut usize,
    context: TouchContext<'_>,
) {
    let TouchContext {
        touch,
        state,
        preview,
        display,
        camera_frame,
        preview_retry_at,
        frame_ready,
        background,
        capture_job,
        native_capture_job,
    } = context;
    let now = Instant::now();
    if frame.count > 0 && *previous_count == 0 {
        *max_contacts = frame.count;
        if let Some(index) = frame.primary {
            let raw = frame.contacts[index];
            let (x, y) = touch.normalize(raw.x, raw.y, display.width, display.height);
            let target = hit_test(state, x, y, display.width, display.height, now);
            let display_y = shutter_display_y(display.width, display.height, state, now);
            if trace_touch() {
                eprintln!(
                    "touch down at ({x},{y}) slot={index} target={target:?} shutter=({}, {})",
                    state.shutter_x, display_y
                );
            }
            let shutter_offset = (state.shutter_x - x, display_y - y);
            *gesture = Some(Gesture {
                target,
                slot: index,
                start: (x, y),
                last: (x, y),
                started: now,
                max_contacts: frame.count,
                dragged: false,
                shutter_offset,
            });
            if let Hit::Control(picker) = target {
                state.open_picker(picker);
                let _ = render_current(
                    display,
                    state,
                    preview,
                    camera_frame,
                    *frame_ready,
                    background,
                );
            }
        }
    } else if frame.count > 0 {
        *max_contacts = (*max_contacts).max(frame.count);
        if let Some(mut current) = *gesture {
            current.max_contacts = current.max_contacts.max(frame.count);
            if frame.contacts[current.slot].active {
                let raw = frame.contacts[current.slot];
                let (x, y) = touch.normalize(raw.x, raw.y, display.width, display.height);
                current.last = (x, y);
                if matches!(current.target, Hit::Shutter)
                    && (x - current.start.0).pow(2) + (y - current.start.1).pow(2) >= 64
                {
                    current.dragged = true;
                    state.close_picker();
                    state.shutter_x =
                        (x + current.shutter_offset.0).clamp(36, display.width as i32 - 36);
                    // Leave a 36px margin above the primary bottom control row.
                    let highest_shutter_y = (display.height as i32 - 136).max(100);
                    state.shutter_y = (y + current.shutter_offset.1).clamp(100, highest_shutter_y);
                    let _ = render_current(
                        display,
                        state,
                        preview,
                        camera_frame,
                        *frame_ready,
                        background,
                    );
                }
            }
            *gesture = Some(current);
        }
    } else if *previous_count > 0 {
        if let Some(current) = gesture.take() {
            let elapsed = now.saturating_duration_since(current.started);
            let (x, y) = current.last;
            if trace_touch() {
                eprintln!(
                    "touch up at ({x},{y}) target={:?} contacts={} dragged={} elapsed={}ms",
                    current.target,
                    current.max_contacts,
                    current.dragged,
                    elapsed.as_millis()
                );
            }
            if current.max_contacts == 1 && elapsed < Duration::from_secs(2) {
                match current.target {
                    Hit::Gallery if state.gallery_open => {
                        let dx = x - current.start.0;
                        let dy = y - current.start.1;
                        if gallery_back_at(current.start.0, current.start.1)
                            && dx.abs() < 36
                            && dy.abs() < 36
                        {
                            state.gallery_open = false;
                            state.gallery_rgb.clear();
                            state.gallery_dimensions = (0, 0);
                            state.close_picker();
                        } else if dx.abs() >= 64 && dx.abs() > dy.abs() * 3 / 2 {
                            gallery_step(state, if dx < 0 { -1 } else { 1 });
                        }
                    }
                    Hit::Gallery => open_gallery(state),
                    Hit::Screenshot => {
                        let result = save_screenshot(state, display);
                        state.show_message(
                            if result.is_ok() {
                                "SCREEN SAVED"
                            } else {
                                "SCREENSHOT FAILED"
                            },
                            2.0,
                        );
                        if let Err(error) = result {
                            eprintln!("camera-square-rs: screenshot: {error}");
                        }
                    }
                    Hit::Grid => {
                        state.composition_grid = state.composition_grid.next();
                        state.show_message(format!("GRID {}", state.composition_grid.label()), 1.2);
                    }
                    Hit::Peak => {
                        state.focus_peaking = !state.focus_peaking;
                        state.show_message(
                            if state.focus_peaking {
                                "PEAK ON"
                            } else {
                                "PEAK OFF"
                            },
                            1.2,
                        );
                    }
                    Hit::Shutter if !current.dragged => take_photo(
                        state,
                        preview,
                        display,
                        frame_ready,
                        capture_job,
                        native_capture_job,
                    ),
                    Hit::PickerChoice(index) if state.picker_is_open(now) => {
                        match apply_picker(state, index) {
                            Ok(true) => {
                                let labels = picker_labels(&state.camera, state.picker);
                                let selected = picker_selected(&state.camera, state.picker);
                                if let Some(value) = labels.get(selected) {
                                    state.show_message(
                                        format!("{} {}", picker_name(state.picker), value),
                                        1.2,
                                    );
                                }
                                if restart_needed(state.picker)
                                    || (state.picker == Picker::CaptureFormat
                                        && std::env::var_os("CAMERA_SQUARE_NATIVE").is_some())
                                {
                                    restart_preview(
                                        state,
                                        preview,
                                        display,
                                        camera_frame,
                                        preview_retry_at,
                                        frame_ready,
                                    );
                                }
                            }
                            Ok(false) => {}
                            Err(error) => eprintln!("camera-square-rs: picker: {error}"),
                        }
                    }
                    _ => {}
                }
                let _ = render_current(
                    display,
                    state,
                    preview,
                    camera_frame,
                    *frame_ready,
                    background,
                );
            }
        }
        *max_contacts = 0;
    } else if let Some(current) = gesture.as_mut() {
        current.max_contacts = current.max_contacts.max(frame.count);
    }
    *previous_count = frame.count;
}

fn restart_needed(picker: Picker) -> bool {
    !matches!(picker, Picker::None | Picker::CaptureFormat)
}

fn picker_name(picker: Picker) -> &'static str {
    match picker {
        Picker::None => "",
        Picker::Shutter => "SS",
        Picker::Iso => "ISO",
        Picker::WhiteBalance => "WB",
        Picker::Metering => "MTR",
        Picker::Sharpness => "SH",
        Picker::Ev => "EV",
        Picker::Aspect => "AR",
        Picker::CaptureFormat => "FORMAT",
        Picker::Contrast => "C",
        Picker::Brightness => "B",
        Picker::Saturation => "SAT",
        Picker::Denoise => "DN",
        Picker::Hdr => "HDR",
    }
}

fn poll_descriptors(touch_fd: RawFd, preview_fd: RawFd) -> io::Result<[libc::pollfd; 2]> {
    let mut fds = [
        libc::pollfd {
            fd: touch_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: preview_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: both descriptors and the array are valid for poll.
    if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 120) } < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(fds)
}

pub fn run() -> io::Result<()> {
    RUNNING.store(true, Ordering::Relaxed);
    // SAFETY: signal handlers only update an atomic flag and perform no allocation.
    unsafe {
        libc::signal(
            libc::SIGTERM,
            stop_signal as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGINT, stop_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let mut display = Display::open()?;
    let mut touch = TouchInput::open()?;
    fs::create_dir_all(camera_directory())?;
    fs::create_dir_all(debug_directory())?;
    let mut state = UiState::new(display.width, display.height);
    let mut preview = Preview::start(&state.camera, display.width)?;
    let mut camera_frame = vec![0; preview.frame_size];
    let mut frame_ready = false;
    let mut background = vec![0; display.pixels.len()];
    let mut preview_retry_at = Instant::now() + Duration::from_secs(1);
    let mut previous_count = 0;
    let mut gesture = None;
    let mut max_contacts = 0;
    let mut capture_job: Option<JoinHandle<CaptureOutcome>> = None;
    let mut native_capture_job: Option<NativeCaptureJob> = None;
    let mut first_preview_after_capture: Option<Instant> = None;
    while RUNNING.load(Ordering::Relaxed) {
        let fds = poll_descriptors(touch.fd(), preview.fd())?;
        if fds[0].revents & libc::POLLIN != 0 {
            for touch_frame in touch.read_frames()? {
                if capture_job.is_some() || native_capture_job.is_some() {
                    previous_count = touch_frame.count;
                    gesture = None;
                    continue;
                }
                apply_touch_frame(
                    touch_frame,
                    &mut previous_count,
                    &mut gesture,
                    &mut max_contacts,
                    TouchContext {
                        touch: &touch,
                        state: &mut state,
                        preview: &mut preview,
                        display: &mut display,
                        camera_frame: &mut camera_frame,
                        preview_retry_at: &mut preview_retry_at,
                        frame_ready: &mut frame_ready,
                        background: &mut background,
                        capture_job: &mut capture_job,
                        native_capture_job: &mut native_capture_job,
                    },
                );
            }
        }
        // Touch wins when both descriptors are ready; rendering a full frame
        // before the shutter event would add avoidable input latency.
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 && preview.fd() >= 0
        {
            match preview.read_frame(&mut camera_frame) {
                Ok(()) => {
                    frame_ready = true;
                    render_current(
                        &mut display,
                        &state,
                        &preview,
                        &camera_frame,
                        frame_ready,
                        &mut background,
                    )?;
                    if let Some(started) = first_preview_after_capture.take() {
                        eprintln!(
                            "camera-square-rs: capture timing first preview={}ms",
                            started.elapsed().as_millis()
                        );
                    }
                }
                Err(error) => {
                    eprintln!("camera-square-rs: preview read: {error}");
                    let _ = preview.stop();
                    frame_ready = false;
                    state.show_message("CAMERA RETRY", 2.0);
                    preview_retry_at = Instant::now() + Duration::from_millis(250);
                }
            }
        }
        if capture_job.as_ref().is_some_and(JoinHandle::is_finished) {
            let outcome = capture_job.take().expect("finished capture job").join();
            match outcome {
                Ok(outcome) => {
                    if let Some(error) = outcome.stop_error {
                        eprintln!("camera-square-rs: preview stop: {error}");
                    }
                    state.show_message(
                        if outcome.save_result.is_ok() {
                            "SAVED"
                        } else {
                            "CAPTURE FAILED"
                        },
                        2.0,
                    );
                    if let Err(error) = outcome.save_result {
                        eprintln!("camera-square-rs: capture failed: {error}");
                    }
                    match outcome.next_preview {
                        Ok(next_preview) => {
                            first_preview_after_capture = Some(outcome.started);
                            camera_frame.resize(next_preview.frame_size, 0);
                            preview = next_preview;
                            preview_retry_at = Instant::now() + Duration::from_secs(1);
                        }
                        Err(error) => {
                            eprintln!("camera-square-rs: restart preview: {error}");
                            preview_retry_at = Instant::now() + Duration::from_secs(1);
                        }
                    }
                }
                Err(_) => {
                    eprintln!("camera-square-rs: capture worker panicked");
                    state.show_message("CAPTURE FAILED", 2.0);
                    preview_retry_at = Instant::now();
                }
            }
        }
        let native_result =
            native_capture_job
                .as_ref()
                .and_then(|job| match job.reply.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => Some(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "native JPEG worker disconnected",
                    ))),
                });
        if let Some(result) = native_result {
            let job = native_capture_job.take().expect("completed native capture");
            eprintln!(
                "camera-square-rs: native capture total={}ms",
                job.started.elapsed().as_millis()
            );
            state.show_message(
                if result.is_ok() {
                    "SAVED"
                } else {
                    "CAPTURE FAILED"
                },
                2.0,
            );
            if let Err(error) = result {
                eprintln!("camera-square-rs: native capture failed: {error}");
            }
        }
        if capture_job.is_none()
            && native_capture_job.is_none()
            && preview.fd() < 0
            && Instant::now() >= preview_retry_at
        {
            restart_preview(
                &mut state,
                &mut preview,
                &display,
                &mut camera_frame,
                &mut preview_retry_at,
                &mut frame_ready,
            );
        }
        let now = Instant::now();
        let picker_open = state.picker_is_open(now);
        state.expire_picker(now);
        if picker_open && state.picker == Picker::None {
            let _ = render_current(
                &mut display,
                &state,
                &preview,
                &camera_frame,
                frame_ready,
                &mut background,
            );
        }
    }
    if let Some(job) = capture_job {
        let _ = job.join();
    }
    let _ = preview.stop();
    Ok(())
}
