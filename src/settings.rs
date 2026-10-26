//! Camera controls and the selection state shared by the renderer and backend.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Picker {
    None,
    Shutter,
    Iso,
    WhiteBalance,
    Metering,
    Sharpness,
    Ev,
    Aspect,
    CaptureFormat,
    Contrast,
    Brightness,
    Saturation,
    Denoise,
}

impl Picker {
    pub const fn is_slider(self) -> bool {
        matches!(
            self,
            Self::Sharpness | Self::Ev | Self::Contrast | Self::Brightness | Self::Saturation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureFormat {
    Jpeg,
    Dng,
    JpegAndDng,
}

impl CaptureFormat {
    pub const ALL: [Self; 3] = [Self::Jpeg, Self::Dng, Self::JpegAndDng];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPG",
            Self::Dng => "DNG",
            Self::JpegAndDng => "JPG/DNG",
        }
    }

    pub const fn captures_raw(self) -> bool {
        !matches!(self, Self::Jpeg)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AspectRatio {
    pub label: &'static str,
    pub still_width: u32,
    pub still_height: u32,
    pub ratio_width: u32,
    pub ratio_height: u32,
}

pub const ASPECT_RATIOS: [AspectRatio; 5] = [
    AspectRatio {
        label: "1:1",
        still_width: 3040,
        still_height: 3040,
        ratio_width: 1,
        ratio_height: 1,
    },
    AspectRatio {
        label: "4:3",
        still_width: 4056,
        still_height: 3040,
        ratio_width: 4,
        ratio_height: 3,
    },
    AspectRatio {
        label: "3:2",
        still_width: 4056,
        still_height: 2704,
        ratio_width: 3,
        ratio_height: 2,
    },
    AspectRatio {
        label: "16:9",
        still_width: 4056,
        still_height: 2282,
        ratio_width: 16,
        ratio_height: 9,
    },
    AspectRatio {
        label: "65:24",
        still_width: 4056,
        still_height: 1498,
        ratio_width: 65,
        ratio_height: 24,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct Choice<T> {
    pub label: &'static str,
    pub value: T,
}

/// Fuji-style dial order. B occupies a visual slot but is deliberately unselectable.
pub const SHUTTERS: [Choice<i32>; 17] = [
    Choice {
        label: "A",
        value: 0,
    },
    Choice {
        label: "B",
        value: 0,
    },
    Choice {
        label: "T",
        value: 2_000_000,
    },
    Choice {
        label: "1",
        value: 1_000_000,
    },
    Choice {
        label: "2",
        value: 500_000,
    },
    Choice {
        label: "4",
        value: 250_000,
    },
    Choice {
        label: "8",
        value: 125_000,
    },
    Choice {
        label: "15",
        value: 66_666,
    },
    Choice {
        label: "30",
        value: 33_333,
    },
    Choice {
        label: "60",
        value: 16_667,
    },
    Choice {
        label: "125",
        value: 8_000,
    },
    Choice {
        label: "250",
        value: 4_000,
    },
    Choice {
        label: "500",
        value: 2_000,
    },
    Choice {
        label: "1000",
        value: 1_000,
    },
    Choice {
        label: "2000",
        value: 500,
    },
    Choice {
        label: "4000",
        value: 250,
    },
    Choice {
        label: "8000",
        value: 125,
    },
];

pub const BULB_INDEX: usize = 1;

pub const ISOS: [Choice<f32>; 6] = [
    Choice {
        label: "A",
        value: 0.0,
    },
    Choice {
        label: "100",
        value: 1.0,
    },
    Choice {
        label: "200",
        value: 2.0,
    },
    Choice {
        label: "400",
        value: 4.0,
    },
    Choice {
        label: "800",
        value: 8.0,
    },
    Choice {
        label: "1600",
        value: 16.0,
    },
];

pub const WHITE_BALANCES: [Choice<&str>; 6] = [
    Choice {
        label: "AUTO",
        value: "auto",
    },
    Choice {
        label: "DAYLIGHT",
        value: "daylight",
    },
    Choice {
        label: "CLOUDY",
        value: "cloudy",
    },
    Choice {
        label: "INDOOR",
        value: "indoor",
    },
    Choice {
        label: "FLUORESCENT",
        value: "fluorescent",
    },
    Choice {
        label: "TUNGSTEN",
        value: "tungsten",
    },
];

pub const METERING: [Choice<&str>; 3] = [
    Choice {
        label: "CENTRE",
        value: "centre",
    },
    Choice {
        label: "SPOT",
        value: "spot",
    },
    Choice {
        label: "AVERAGE",
        value: "average",
    },
];

pub const DENOISE: [Choice<&str>; 5] = [
    Choice {
        label: "AUTO",
        value: "auto",
    },
    Choice {
        label: "OFF",
        value: "off",
    },
    Choice {
        label: "CDN-OFF",
        value: "cdn_off",
    },
    Choice {
        label: "FAST",
        value: "cdn_fast",
    },
    Choice {
        label: "HQ",
        value: "cdn_hq",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct CameraSettings {
    pub shutter: usize,
    pub iso: usize,
    pub white_balance: usize,
    pub metering: usize,
    /// Sharpness in tenths, from 0.0 to 2.0.
    pub sharpness_tenths: u8,
    /// EV is stored as thirds of a stop, from -30 through +30.
    pub ev_thirds: i8,
    pub aspect: usize,
    pub capture_format: CaptureFormat,
    /// Contrast in tenths, from 0.0 to 2.0.
    pub contrast_tenths: u8,
    /// Brightness in tenths, from -1.0 to +1.0.
    pub brightness_tenths: i8,
    /// Saturation in tenths, from 0.0 to 2.0.
    pub saturation_tenths: u8,
    pub denoise: usize,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            shutter: 0,
            iso: 0,
            white_balance: 0,
            metering: 0,
            sharpness_tenths: 10,
            ev_thirds: 0,
            aspect: 1,
            capture_format: CaptureFormat::Jpeg,
            contrast_tenths: 10,
            brightness_tenths: 0,
            saturation_tenths: 10,
            denoise: 0,
        }
    }
}

impl CameraSettings {
    pub fn slider_value(&self, picker: Picker) -> Option<i16> {
        Some(match picker {
            Picker::Sharpness => i16::from(self.sharpness_tenths),
            Picker::Ev => i16::from(self.ev_thirds),
            Picker::Contrast => i16::from(self.contrast_tenths),
            Picker::Brightness => i16::from(self.brightness_tenths),
            Picker::Saturation => i16::from(self.saturation_tenths),
            _ => return None,
        })
    }

    pub fn set_slider_value(&mut self, picker: Picker, value: i16) -> bool {
        match picker {
            Picker::Sharpness => replace_value(&mut self.sharpness_tenths, value as u8),
            Picker::Ev => replace_value(&mut self.ev_thirds, value as i8),
            Picker::Contrast => replace_value(&mut self.contrast_tenths, value as u8),
            Picker::Brightness => replace_value(&mut self.brightness_tenths, value as i8),
            Picker::Saturation => replace_value(&mut self.saturation_tenths, value as u8),
            _ => false,
        }
    }

    pub fn slider_label(&self, picker: Picker) -> String {
        match picker {
            Picker::Sharpness => format!("{:.1}", f32::from(self.sharpness_tenths) / 10.0),
            Picker::Ev => self.ev_label(),
            Picker::Contrast => format!("{:.1}", f32::from(self.contrast_tenths) / 10.0),
            Picker::Brightness if self.brightness_tenths == 0 => "0.0".to_string(),
            Picker::Brightness => format!("{:+.1}", f32::from(self.brightness_tenths) / 10.0),
            Picker::Saturation => self.saturation_label(),
            _ => String::new(),
        }
    }

    pub fn saturation_value(&self) -> f32 {
        f32::from(self.saturation_tenths) / 10.0
    }

    pub fn saturation_label(&self) -> String {
        format!("{:.1}", self.saturation_value())
    }

    pub fn preview_dimensions(&self, screen_width: usize) -> (usize, usize) {
        let ar = ASPECT_RATIOS[self.aspect];
        let w = screen_width & !1;
        // Match the working C stream dimensions: truncate after adding one,
        // then round down to an even YUV420 height (e.g. 720×16:9 => 720×406).
        let h = ((w as f64 * ar.ratio_height as f64 / ar.ratio_width as f64 + 1.0) as usize) & !1;
        (w, h.max(2))
    }

    pub fn ev_value(&self) -> f32 {
        self.ev_thirds as f32 / 3.0
    }

    pub fn ev_label(&self) -> String {
        if self.ev_thirds == 0 {
            "0".to_string()
        } else {
            format!("{:+.1}", self.ev_value())
        }
    }
}

fn replace_value<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}
