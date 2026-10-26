# Camera support

Camera Square is designed for the Raspberry Pi camera stack. With the `native-camera` build feature, it uses a persistent libcamera session for JPEG, DNG, and paired capture across the available aspect ratios and controls supported by the camera. Unavailable native controls, unsupported native stream configurations, and builds without the feature use `rpicam-vid` and `rpicam-still`. A control appearing in the UI does not guarantee that every sensor or Raspberry Pi model supports it.

## Controls exposed by the app

| Control | Support in Camera Square | Notes |
| --- | --- | --- |
| Shutter speed | Limited choices | Auto and fixed values up to 1/1000 s; custom speeds and long exposures are not exposed. |
| Gain / ISO | Limited choices | Discrete gain values with nominal ISO labels; no continuous control or measured ISO readout. |
| White balance | Modes | Auto, daylight, cloudy, indoor, fluorescent, and tungsten. Custom modes and fixed gains are not exposed. |
| Metering | Centre, spot, average | Each mode is passed to preview and still capture. Guides are visual aids only. |
| EV compensation | Full app range | −10.0 to +10.0 in one-third-stop steps. It adjusts automatic exposure/gain targets; it cannot change exposure when shutter and gain are both fixed. |
| Sharpness | Limited range | 0.0 to 2.0 slider in 0.1 steps. |
| Contrast, brightness, saturation | Limited ranges | Contrast and saturation use 0.0–2.0 sliders; brightness uses −1.0–+1.0. All move in 0.1 steps. `rpicam` accepts a wider numeric range for contrast and saturation. |
| Denoise | Selected modes | Auto, off, colour-denoise off, fast, and high quality. Throughput and still/video behavior can vary by hardware. |
| HDR | Not exposed | The tested Pi Zero 2 W with HQ Camera does not produce HDR with `auto` or `single-exp`. |
| Focus peaking | App feature | Preview-only edge overlay; it is not a camera control. |
| Region of interest, tuning files, exposure profiles | Not exposed | — |
| Autofocus and lens position | Not exposed | The HQ Camera lens is manually focused; there is no motorized focus control. |

The controls follow the [Raspberry Pi camera software documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html). See the [controls guide](controls.md) for the current picker values and user-facing behavior.

## Why HDR is absent

On the tested Pi Zero 2 W with the IMX477 HQ Camera, both `rpicam-still --hdr auto` and `--hdr single-exp` logged `HDR mode SingleExposure not supported`. In a controlled OFF → AUTO → S-EXP → OFF series, all four photos recorded the same 1/15 s exposure and nominal ISO 792. The visual differences between HDR selections were smaller than between the two OFF captures. The earlier version of Camera Square sent the same options to `rpicam`, so keeping an HDR picker would suggest an effect that this hardware did not produce.

HDR may be possible with another camera or Pi. The [Raspberry Pi camera software documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html#hdr) describes sensor HDR on supported cameras such as Camera Module 3 (IMX708), and on-board single-exposure HDR on Raspberry Pi 5 and later. Support depends on the sensor, Pi model, camera stack, and selected mode. A future HDR control should check the actual hardware capability before appearing in the UI.
