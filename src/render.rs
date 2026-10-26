//! Preview and UI rendering, preserving the original framebuffer layout.

use std::time::Instant;

use crate::draw::{Canvas, rgb};
use crate::settings::{
    ASPECT_RATIOS, CameraSettings, CaptureFormat, DENOISE, ISOS, METERING, Picker, SHUTTERS,
    ev_picker_label,
};
use crate::state::{CompositionGrid, UiState};

const INK: u16 = 0x0000;
const PAPER: u16 = 0xffff;
const SOFT_PAPER: u16 = 0x9cf3;
const QUIET: u16 = 0x7bef;
const VALUE_LABELS: [&str; 5] = ["0.0", "0.5", "1.0", "1.5", "2.0"];
const BRIGHTNESS_LABELS: [&str; 5] = ["-1.0", "-0.5", "0.0", "+0.5", "+1.0"];
const WB_PICKER: [&str; 6] = ["AUTO", "DAY", "CLOUDY", "INDOOR", "FLUOR", "TUNG"];
const WB_READOUT: [&str; 6] = ["AUTO", "DAY", "CLDY", "INDR", "FLOR", "TUNG"];
const METERING_READOUT: [&str; 3] = ["CTR", "SPOT", "AVG"];
pub const TOP_WIDTHS: [i32; 8] = [42, 38, 52, 64, 76, 112, 108, 48];
pub const TOP_GAP: i32 = 20;
// Start below the centered controls' 64px hit area.
const TOP_PICKER_LIST_TOP: i32 = 72;
pub const SATURATION_SLIDER_TOUCH_TOP: i32 = 78;
pub const SATURATION_SLIDER_TOUCH_BOTTOM: i32 = 174;

pub fn saturation_slider_bounds(width: usize) -> (i32, i32) {
    let width = width as i32;
    let margin = (width / 12).clamp(12, 60).min((width - 2).max(0) / 2);
    (margin, (width - margin - 1).max(margin + 1))
}

pub fn saturation_slider_value(x: i32, width: usize) -> u8 {
    let (left, right) = saturation_slider_bounds(width);
    let span = right - left;
    (((x - left).clamp(0, span) * 20 + span / 2) / span) as u8
}

#[derive(Clone, Copy, Debug)]
pub struct PreviewFrameLayout {
    pub width: usize,
    pub height: usize,
    pub luma_stride: usize,
    pub chroma_stride: usize,
}

/// Centered image bounds, shared by the preview and composition guides.
pub fn preview_viewport(
    width: usize,
    height: usize,
    settings: &CameraSettings,
) -> (i32, i32, i32, i32) {
    let ar = ASPECT_RATIOS[settings.aspect];
    let ratio = ar.ratio_width as f64 / ar.ratio_height as f64;
    let mut w = width as i32;
    let mut h = (width as f64 / ratio) as i32;
    if h > height as i32 {
        h = height as i32;
        w = (height as f64 * ratio) as i32;
    }
    ((width as i32 - w) / 2, (height as i32 - h) / 2, w, h)
}

/// Convert an entire planar YUV420 frame. Invalid or incomplete frames are ignored.
pub fn render_preview(
    canvas: &mut Canvas<'_>,
    frame: &[u8],
    layout: PreviewFrameLayout,
    settings: &CameraSettings,
    focus_peaking: bool,
) {
    let PreviewFrameLayout {
        width: frame_width,
        height: frame_height,
        luma_stride,
        chroma_stride,
    } = layout;
    if frame_width == 0
        || frame_height == 0
        || frame_width > luma_stride
        || frame_width.div_ceil(2) > chroma_stride
    {
        return;
    }
    let Some(y_bytes) = luma_stride.checked_mul(frame_height) else {
        return;
    };
    let Some(chroma_bytes) = chroma_stride.checked_mul(frame_height.div_ceil(2)) else {
        return;
    };
    let Some(total) = chroma_bytes
        .checked_mul(2)
        .and_then(|chroma| y_bytes.checked_add(chroma))
    else {
        return;
    };
    if frame.len() < total {
        return;
    }
    let yp = &frame[..y_bytes];
    let up = &frame[y_bytes..y_bytes + chroma_bytes];
    let vp = &frame[y_bytes + chroma_bytes..total];
    let (left, top, w, h) = preview_viewport(canvas.width, canvas.height, settings);
    canvas.fill_rect(0, 0, canvas.width as i32, canvas.height as i32, INK);
    if w <= 0 || h <= 0 {
        return;
    }
    for y in 0..h {
        let sy = y as usize * frame_height / h as usize;
        for x in 0..w {
            let sx = x as usize * frame_width / w as usize;
            let luma = i32::from(yp[sy * luma_stride + sx]);
            let u = i32::from(up[(sy / 2) * chroma_stride + sx / 2]) - 128;
            let v = i32::from(vp[(sy / 2) * chroma_stride + sx / 2]) - 128;
            let c = (luma - 16).max(0);
            let r = ((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8;
            let mut color = rgb(r, g, b);
            if focus_peaking && sx > 0 && sy > 0 && sx + 1 < frame_width && sy + 1 < frame_height {
                let gx = i32::from(yp[sy * luma_stride + sx + 1])
                    - i32::from(yp[sy * luma_stride + sx - 1]);
                let gy = i32::from(yp[(sy + 1) * luma_stride + sx])
                    - i32::from(yp[(sy - 1) * luma_stride + sx]);
                if gx.abs() + gy.abs() >= 32 {
                    color = rgb(220, 20, 60);
                }
            }
            canvas.pixel(left + x, top + y, color);
        }
    }
}

pub fn picker_labels(_settings: &CameraSettings, picker: Picker) -> Vec<String> {
    match picker {
        Picker::None => Vec::new(),
        Picker::Shutter => SHUTTERS.iter().map(|v| v.label.to_string()).collect(),
        Picker::Iso => ISOS.iter().map(|v| v.label.to_string()).collect(),
        Picker::WhiteBalance => WB_PICKER.iter().map(|s| s.to_string()).collect(),
        Picker::Metering => METERING.iter().map(|v| v.label.to_string()).collect(),
        Picker::Sharpness | Picker::Contrast => {
            VALUE_LABELS.iter().map(|s| s.to_string()).collect()
        }
        Picker::Saturation => Vec::new(),
        Picker::Brightness => BRIGHTNESS_LABELS.iter().map(|s| s.to_string()).collect(),
        Picker::Ev => (-30..=30).map(ev_picker_label).collect(),
        Picker::Aspect => ASPECT_RATIOS.iter().map(|v| v.label.to_string()).collect(),
        Picker::CaptureFormat => CaptureFormat::ALL
            .iter()
            .map(|v| v.label().to_string())
            .collect(),
        Picker::Denoise => DENOISE.iter().map(|v| v.label.to_string()).collect(),
    }
}

pub fn picker_selected(settings: &CameraSettings, picker: Picker) -> usize {
    match picker {
        Picker::None => 0,
        Picker::Shutter => settings.shutter,
        Picker::Iso => settings.iso,
        Picker::WhiteBalance => settings.white_balance,
        Picker::Metering => settings.metering,
        Picker::Sharpness => settings.sharpness,
        Picker::Contrast => settings.contrast,
        Picker::Brightness => settings.brightness,
        Picker::Saturation => usize::from(settings.saturation_tenths),
        Picker::Ev => (settings.ev_thirds + 30) as usize,
        Picker::Aspect => settings.aspect,
        Picker::CaptureFormat => CaptureFormat::ALL
            .iter()
            .position(|&v| v == settings.capture_format)
            .unwrap_or(0),
        Picker::Denoise => settings.denoise,
    }
}

pub fn picker_scale(width: usize, settings: &CameraSettings, picker: Picker) -> i32 {
    let labels = picker_labels(settings, picker);
    if labels.is_empty() {
        return 1;
    }
    let longest = labels.iter().map(String::len).max().unwrap_or(1).max(1);
    (width as i32 / labels.len() as i32 / (6 * longest as i32)).clamp(1, 4)
}

pub const fn picker_is_top_row(picker: Picker) -> bool {
    matches!(
        picker,
        Picker::Contrast | Picker::Brightness | Picker::Saturation | Picker::Denoise
    )
}

pub fn picker_list_top(
    width: usize,
    height: usize,
    settings: &CameraSettings,
    picker: Picker,
) -> i32 {
    if picker_is_top_row(picker) {
        TOP_PICKER_LIST_TOP
    } else if picker == Picker::Ev {
        height as i32 - 152
    } else {
        height as i32 - 112 - 7 * picker_scale(width, settings, picker)
    }
}

pub fn shutter_display_y(width: usize, height: usize, state: &UiState, now: Instant) -> i32 {
    if state.picker_is_open(now) {
        let list_top = picker_list_top(width, height, &state.camera, state.picker);
        let list_height = if state.picker == Picker::Saturation {
            SATURATION_SLIDER_TOUCH_BOTTOM - TOP_PICKER_LIST_TOP
        } else if state.picker == Picker::Ev {
            35
        } else {
            7 * picker_scale(width, &state.camera, state.picker)
        };
        if state.shutter_y + 36 >= list_top && state.shutter_y - 36 <= list_top + list_height {
            if picker_is_top_row(state.picker) {
                return (list_top + list_height + 48)
                    .min(height as i32 - 36)
                    .max(36);
            }
            return (list_top - 48).max(36);
        }
    }
    state.shutter_y
}

fn outline(canvas: &mut Canvas<'_>, x0: i32, y0: i32, x1: i32, y1: i32, color: u16) {
    canvas.line(x0, y0, x1, y0, color);
    canvas.line(x1, y0, x1, y1, color);
    canvas.line(x1, y1, x0, y1, color);
    canvas.line(x0, y1, x0, y0, color);
}

fn draw_grid(canvas: &mut Canvas<'_>, state: &UiState) {
    let (left, top, w, h) = preview_viewport(canvas.width, canvas.height, &state.camera);
    if w <= 0 || h <= 0 {
        return;
    }
    match state.composition_grid {
        CompositionGrid::Nine | CompositionGrid::TwentyFour => {
            let (columns, rows) = if state.composition_grid == CompositionGrid::Nine {
                (3, 3)
            } else {
                (6, 4)
            };
            for i in 1..columns {
                canvas.line(
                    left + i * w / columns,
                    top,
                    left + i * w / columns,
                    top + h - 1,
                    QUIET,
                );
            }
            for i in 1..rows {
                canvas.line(
                    left,
                    top + i * h / rows,
                    left + w - 1,
                    top + i * h / rows,
                    QUIET,
                );
            }
        }
        CompositionGrid::Hd => {
            let ratio = 16.0 / 9.0;
            let (fw, fh) = if w as f64 / h as f64 >= ratio {
                ((h as f64 * ratio) as i32, h)
            } else {
                (w, (w as f64 / ratio) as i32)
            };
            let x = left + (w - fw) / 2;
            let y = top + (h - fh) / 2;
            outline(canvas, x, y, x + fw - 1, y + fh - 1, SOFT_PAPER);
        }
        CompositionGrid::Off => {}
    }
}

fn draw_metering(canvas: &mut Canvas<'_>, state: &UiState) {
    let (cx, cy) = (canvas.width as i32 / 2, canvas.height as i32 / 2);
    match state.camera.metering {
        1 => {
            canvas.line(cx - 18, cy, cx + 18, cy, PAPER);
            canvas.line(cx, cy - 18, cx, cy + 18, PAPER);
        }
        0 => {
            outline(canvas, cx - 68, cy - 68, cx + 68, cy + 68, QUIET);
            outline(canvas, cx - 32, cy - 32, cx + 32, cy + 32, SOFT_PAPER);
        }
        _ => {
            let (left, top, w, h) = preview_viewport(canvas.width, canvas.height, &state.camera);
            for row in 1..=3 {
                for col in 1..=3 {
                    let (x, y) = (left + col * w / 4, top + row * h / 4);
                    canvas.line(x - 5, y, x + 5, y, QUIET);
                    canvas.line(x, y - 5, x, y + 5, QUIET);
                }
            }
        }
    }
}

fn gallery_icon(canvas: &mut Canvas<'_>, x: i32) {
    let y = 18;
    outline(canvas, x + 1, y + 1, x + 43, y + 35, INK);
    outline(canvas, x, y, x + 42, y + 34, PAPER);
    canvas.circle(x + 12, y + 10, 3, PAPER);
    for (x0, y0, x1, y1) in [
        (5, 28, 17, 16),
        (17, 16, 24, 23),
        (24, 23, 31, 17),
        (31, 17, 38, 27),
    ] {
        canvas.line(x + x0, y + y0, x + x1, y + y1, PAPER);
    }
}

fn screenshot_icon(canvas: &mut Canvas<'_>, x: i32) {
    let y = 17;
    outline(canvas, x + 1, y + 6, x + 39, y + 31, INK);
    outline(canvas, x, y + 5, x + 38, y + 30, PAPER);
    canvas.line(x + 7, y + 4, x + 14, y + 4, PAPER);
    canvas.line(x + 7, y + 3, x + 14, y + 3, PAPER);
    canvas.circle(x + 20, y + 18, 7, PAPER);
    canvas.circle(x + 20, y + 18, 4, INK);
}

pub fn top_item_x(width: usize, item: usize) -> i32 {
    let total: i32 = TOP_WIDTHS.iter().sum::<i32>() + TOP_GAP * (TOP_WIDTHS.len() as i32 - 1);
    (width as i32 - total) / 2 + TOP_WIDTHS.iter().take(item).sum::<i32>() + TOP_GAP * item as i32
}

fn overlay_text(
    canvas: &mut Canvas<'_>,
    fullscreen: bool,
    x: i32,
    y: i32,
    text: &str,
    scale: i32,
    color: u16,
) {
    if fullscreen {
        canvas.text(x + 1, y + 1, text, scale, INK);
    }
    canvas.text(x, y, text, scale, color);
}

fn translucent_shutter(canvas: &mut Canvas<'_>, cx: i32, cy: i32) {
    for y in -36..=36 {
        for x in -36..=36 {
            if x * x + y * y > 36 * 36 {
                continue;
            }
            let (px, py) = (cx + x, cy + y);
            if px < 0
                || py < 0
                || px as usize >= canvas.width
                || px as usize >= canvas.stride
                || py as usize >= canvas.height
            {
                continue;
            }
            let Some(&under) = canvas.pixels.get(py as usize * canvas.stride + px as usize) else {
                continue;
            };
            let (r, g, b) = ((under >> 11) & 31, (under >> 5) & 63, under & 31);
            canvas.pixel(
                px,
                py,
                (((r + 32) / 2) << 11) | (((g + 64) / 2) << 5) | ((b + 32) / 2),
            );
        }
    }
}

fn render_gallery(canvas: &mut Canvas<'_>, state: &UiState) {
    canvas.fill_rect(0, 0, canvas.width as i32, canvas.height as i32, INK);
    let (w, h) = state.gallery_dimensions;
    if w > 0
        && h > 0
        && w.checked_mul(h)
            .and_then(|n| n.checked_mul(3))
            .is_some_and(|n| n <= state.gallery_rgb.len())
    {
        let mut out_w = canvas.width;
        let mut out_h = h * out_w / w;
        if out_h > canvas.height {
            out_h = canvas.height;
            out_w = w * out_h / h;
        }
        if out_w > 0 && out_h > 0 {
            let (left, top) = ((canvas.width - out_w) / 2, (canvas.height - out_h) / 2);
            for y in 0..out_h {
                for x in 0..out_w {
                    let offset = ((y * h / out_h) * w + x * w / out_w) * 3;
                    canvas.pixel(
                        (left + x) as i32,
                        (top + y) as i32,
                        rgb(
                            state.gallery_rgb[offset],
                            state.gallery_rgb[offset + 1],
                            state.gallery_rgb[offset + 2],
                        ),
                    );
                }
            }
        }
    } else {
        canvas.text(
            canvas.width as i32 / 2 - 54,
            canvas.height as i32 / 2,
            "NO PHOTOS",
            2,
            PAPER,
        );
    }
    if let Some(index) = state.gallery_index {
        if let Some(photo) = state.gallery.get(index) {
            let bar_top = canvas.height as i32 - 44;
            canvas.fill_rect(0, bar_top, canvas.width as i32, 44, INK);
            canvas.text(12, bar_top + 12, photo.format.label(), 2, PAPER);
            if let Some(stem) = photo.path.file_stem().and_then(|name| name.to_str()) {
                let name = stem.replace('_', " ").to_ascii_uppercase();
                canvas.text(160, bar_top + 15, &name, 1, PAPER);
            }
            let position = format!("{}/{}", index + 1, state.gallery.len());
            canvas.text(canvas.width as i32 - 92, bar_top + 15, &position, 1, PAPER);
        }
    }
    for (dx, dy, color) in [(1, 1, INK), (0, 0, PAPER)] {
        canvas.line(22 + dx, 35 + dy, 62 + dx, 35 + dy, color);
        canvas.line(22 + dx, 35 + dy, 36 + dx, 21 + dy, color);
        canvas.line(22 + dx, 35 + dy, 36 + dx, 49 + dy, color);
    }
}

pub fn render_overlays(canvas: &mut Canvas<'_>, state: &UiState, now: Instant) {
    if state.gallery_open {
        render_gallery(canvas, state);
        return;
    }
    let camera = &state.camera;
    let fullscreen = camera.aspect == 0;
    let width = canvas.width as i32;
    let height = canvas.height as i32;
    draw_grid(canvas, state);
    gallery_icon(canvas, top_item_x(canvas.width, 0));
    screenshot_icon(canvas, top_item_x(canvas.width, 1));
    let saturation_label = camera.saturation_label();
    for (item, label, value, offset) in [
        (2, "C", VALUE_LABELS[camera.contrast], 16),
        (3, "B", BRIGHTNESS_LABELS[camera.brightness], 16),
        (4, "SAT", saturation_label.as_str(), 40),
        (5, "DN", DENOISE[camera.denoise].label, 28),
        (6, "GRID", state.composition_grid.label(), 52),
    ] {
        let x = top_item_x(canvas.width, item);
        overlay_text(canvas, fullscreen, x, 28, label, 2, PAPER);
        overlay_text(canvas, fullscreen, x + offset, 28, value, 2, SOFT_PAPER);
    }
    overlay_text(
        canvas,
        fullscreen,
        top_item_x(canvas.width, 7),
        28,
        "PEAK",
        2,
        if state.focus_peaking { PAPER } else { QUIET },
    );
    if state.picker_is_open(now) {
        let top = picker_list_top(canvas.width, canvas.height, camera, state.picker);
        if state.picker == Picker::Saturation {
            let (left, right) = saturation_slider_bounds(canvas.width);
            let span = right - left;
            let thumb = left + span * i32::from(camera.saturation_tenths) / 20;
            let panel_left = (left - 36).max(0);
            canvas.darken_rect_half(
                panel_left,
                SATURATION_SLIDER_TOUCH_TOP,
                (width - panel_left * 2).max(1),
                SATURATION_SLIDER_TOUCH_BOTTOM - SATURATION_SLIDER_TOUCH_TOP,
            );
            canvas.fill_rect(left, 119, span, 4, QUIET);
            canvas.fill_rect(left, 119, thumb - left, 4, PAPER);
            canvas.circle(thumb, 121, 10, PAPER);
            canvas.circle(thumb, 121, 5, INK);
            canvas.text(left, 86, "SATURATION", 2, PAPER);
            canvas.text(right - 36, 86, &saturation_label, 2, PAPER);
            canvas.text(left, 147, "0.0", 2, SOFT_PAPER);
            canvas.text((left + right) / 2 - 18, 147, "1.0", 2, SOFT_PAPER);
            canvas.text(right - 36, 147, "2.0", 2, SOFT_PAPER);
        } else if state.picker == Picker::Ev {
            let cell_width = width / 5;
            for slot in 0..5 {
                let thirds = camera.ev_thirds as i32 + slot - 2;
                if !(-30..=30).contains(&thirds) {
                    continue;
                }
                let label = ev_picker_label(thirds as i8);
                let scale = match slot {
                    2 => 5,
                    1 | 3 => 4,
                    _ => 3,
                }
                .min(((cell_width - 2) / (label.len() as i32 * 6)).max(1));
                let left = slot * cell_width;
                let right = if slot == 4 { width } else { left + cell_width };
                let x = left + (right - left - label.len() as i32 * 6 * scale) / 2;
                overlay_text(
                    canvas,
                    fullscreen,
                    x,
                    top,
                    &label,
                    scale,
                    if slot == 2 { PAPER } else { QUIET },
                );
            }
        } else {
            let labels = picker_labels(camera, state.picker);
            let scale = picker_scale(canvas.width, camera, state.picker);
            let selected = picker_selected(camera, state.picker);
            for (i, label) in labels.iter().enumerate() {
                let left = i as i32 * width / labels.len() as i32;
                let right = (i + 1) as i32 * width / labels.len() as i32;
                let x = left + (right - left - label.len() as i32 * 6 * scale) / 2;
                overlay_text(
                    canvas,
                    fullscreen,
                    x,
                    top,
                    label,
                    scale,
                    if i == selected { PAPER } else { QUIET },
                );
            }
        }
    }
    draw_metering(canvas, state);
    let bottom = height - 100;
    let col_w = width / 8;
    if !fullscreen {
        canvas.fill_rect(0, bottom, width, 100, INK);
    }
    let ev_label = camera.ev_label();
    for (col, label, value, offset) in [
        (0, "SS", SHUTTERS[camera.shutter].label, 29),
        (1, "ISO", ISOS[camera.iso].label, 41),
        (2, "WB", WB_READOUT[camera.white_balance], 28),
        (3, "MTR", METERING_READOUT[camera.metering], 41),
        (4, "SH", VALUE_LABELS[camera.sharpness], 28),
        (5, "EV", ev_label.as_str(), 29),
    ] {
        overlay_text(
            canvas,
            fullscreen,
            col * col_w + 2,
            bottom + 43,
            label,
            2,
            PAPER,
        );
        overlay_text(
            canvas,
            fullscreen,
            col * col_w + offset,
            bottom + 43,
            value,
            2,
            SOFT_PAPER,
        );
    }
    overlay_text(
        canvas,
        fullscreen,
        6 * col_w + 2,
        bottom + 43,
        ASPECT_RATIOS[camera.aspect].label,
        2,
        PAPER,
    );
    overlay_text(
        canvas,
        fullscreen,
        7 * col_w + 2,
        bottom + 43,
        camera.capture_format.label(),
        2,
        PAPER,
    );
    translucent_shutter(
        canvas,
        state.shutter_x,
        shutter_display_y(canvas.width, canvas.height, state, now),
    );
    if state
        .message_deadline
        .is_some_and(|deadline| deadline > now)
    {
        let n = state.message.len() as i32 * 12;
        if !fullscreen {
            canvas.fill_rect((width - n - 28) / 2, 82, n + 28, 34, INK);
        }
        overlay_text(
            canvas,
            fullscreen,
            (width - n) / 2,
            92,
            &state.message,
            2,
            PAPER,
        );
    }
}

#[cfg(test)]
mod slider_tests {
    use super::*;

    #[test]
    fn saturation_slider_reaches_full_range() {
        for width in [480, 720] {
            let (left, right) = saturation_slider_bounds(width);
            assert_eq!(saturation_slider_value(left - 20, width), 0);
            assert_eq!(saturation_slider_value((left + right) / 2, width), 10);
            assert_eq!(saturation_slider_value(right + 20, width), 20);
        }
    }
}
