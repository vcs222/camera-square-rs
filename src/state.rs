//! UI state kept separate from framebuffer rendering and camera subprocesses.

use std::path::PathBuf;
use std::time::Instant;

use crate::settings::{CameraSettings, Picker};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionGrid {
    Nine,
    TwentyFour,
    Hd,
    Off,
}

impl CompositionGrid {
    pub fn next(self) -> Self {
        match self {
            Self::Nine => Self::TwentyFour,
            Self::TwentyFour => Self::Hd,
            Self::Hd => Self::Off,
            Self::Off => Self::Nine,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Nine => "9",
            Self::TwentyFour => "24",
            Self::Hd => "HD",
            Self::Off => "OFF",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PhotoEntry {
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
}

pub struct UiState {
    pub camera: CameraSettings,
    pub picker: Picker,
    pub picker_deadline: Option<Instant>,
    pub composition_grid: CompositionGrid,
    pub focus_peaking: bool,
    pub shutter_x: i32,
    pub shutter_y: i32,
    pub gallery_open: bool,
    pub gallery: Vec<PhotoEntry>,
    pub gallery_index: Option<usize>,
    pub gallery_rgb: Vec<u8>,
    pub gallery_dimensions: (usize, usize),
    pub screenshot_index: u64,
    pub message: String,
    pub message_deadline: Option<Instant>,
}

impl UiState {
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        Self {
            camera: CameraSettings::default(),
            picker: Picker::None,
            picker_deadline: None,
            composition_grid: CompositionGrid::Nine,
            focus_peaking: false,
            shutter_x: screen_width as i32 - 60,
            shutter_y: screen_height as i32 / 2,
            gallery_open: false,
            gallery: Vec::new(),
            gallery_index: None,
            gallery_rgb: Vec::new(),
            gallery_dimensions: (0, 0),
            screenshot_index: 0,
            message: "LIVE".into(),
            message_deadline: None,
        }
    }

    pub fn picker_is_open(&self, now: Instant) -> bool {
        self.picker != Picker::None && self.picker_deadline.is_some_and(|deadline| deadline > now)
    }

    pub fn show_message(&mut self, text: impl Into<String>, seconds: f64) {
        self.message = text.into();
        self.message_deadline =
            Instant::now().checked_add(std::time::Duration::from_secs_f64(seconds));
    }

    pub fn close_picker(&mut self) {
        self.picker = Picker::None;
        self.picker_deadline = None;
    }

    pub fn open_picker(&mut self, picker: Picker) {
        let now = Instant::now();
        if self.picker != picker || !self.picker_is_open(now) {
            self.picker = picker;
        }
        self.picker_deadline = now.checked_add(std::time::Duration::from_secs(5));
    }

    pub fn refresh_picker_timeout(&mut self) {
        self.picker_deadline = Instant::now().checked_add(std::time::Duration::from_secs(5));
    }

    pub fn expire_picker(&mut self, now: Instant) {
        if self.picker != Picker::None && !self.picker_is_open(now) {
            self.close_picker();
        }
    }
}
