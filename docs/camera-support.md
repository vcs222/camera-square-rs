# Camera support

Camera Square is designed for the Raspberry Pi camera stack. With the `native-camera` build feature, it uses a persistent libcamera session for default 4:3 JPEG preview and capture. Adjusted camera settings, other aspect ratios, and DNG output use `rpicam-vid` for preview and `rpicam-still` for capture. A control appearing in the UI does not guarantee that every sensor or Raspberry Pi model supports it.

## Controls exposed by the app

| Control | Support in Camera Square | Notes |
| --- | --- | --- |
| Shutter speed | Limited choices | Auto and fixed values up to 1/1000 s; custom speeds and long exposures are not exposed. |
| Gain / ISO | Limited choices | Discrete gain values with nominal ISO labels; no continuous control or measured ISO readout. |
| White balance | Modes | Auto, daylight, cloudy, indoor, fluorescent, and tungsten. Custom modes and fixed gains are not exposed. |
| Metering | Centre, spot, average | Each mode is passed to preview and still capture. Guides are visual aids only. |
| EV compensation | Full app range | −10.0 to +10.0 in one-third-stop steps. It adjusts automatic exposure/gain targets; it cannot change exposure when shutter and gain are both fixed. |
| Sharpness | Limited choices | 0.0 to 2.0 in 0.5 steps. |
| Contrast, brightness, saturation | Limited choices | The app offers common steps. `rpicam` accepts a wider numeric range for contrast and saturation than the app exposes. |
| Denoise | Selected modes | Auto, off, colour-denoise off, fast, and high quality. Throughput and still/video behavior can vary by hardware. |
| HDR | Selected modes | Off, auto, and single exposure. Sensor and platform support varies. |
| Focus peaking | App feature | Preview-only edge overlay; it is not a camera control. |
| Region of interest, tuning files, exposure profiles | Not exposed | — |
| Autofocus and lens position | Not exposed | The HQ Camera lens is manually focused; there is no motorized focus control. |

The controls follow the [Raspberry Pi camera software documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html). See the [controls guide](controls.md) for the current picker values and user-facing behavior.
