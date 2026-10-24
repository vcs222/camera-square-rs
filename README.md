# camera-square-rs

[Source and issue tracker](https://github.com/vcs222/camera-square-rs) · [MIT license](LICENSE)

Minimal touchscreen camera UI for the Raspberry Pi Zero 2 W, HyperPixel 4 Square, and HQ Camera.

The application is written in Rust. Its crate, source, and systemd unit template live at the repository root. It targets Raspberry Pi OS/Debian AArch64 with the Raspberry Pi camera stack and a compatible RGB565 framebuffer touchscreen.

## Touch controls

The top line contains the gallery and screenshot buttons, then **C**, **B**, **SAT**, **DN**, **HDR**, **GRID**, and **PEAK**. The eight primary camera controls remain in the bottom row: **SS**, **ISO**, **WB**, **MTR**, **SH**, **EV**, aspect ratio, and capture format. Primary control labels are white and selected values are softer gray; ratio and format are self-labeled.

Tap a control to show its picker as a floating row over the live view. Choices are spread across the full screen width, sized to fit their slots, and drawn without a background panel. The current choice is highlighted; the other choices are dimmer. Tap a choice to apply it. The selection updates while the picker stays open.

The picker closes when the shutter button is pressed or after five seconds without selecting another value. Selecting a value restarts that timer. Tapping the same setting control while its picker is open leaves the picker open; tapping another setting control switches to that picker.

Tap **GRID** to cycle through **9**, **24**, **HD**, and **OFF**; the current choice appears beside the control. The default is **9**. Grid 9 divides the selected image area into 3×3 cells. Grid 24 divides it into 6 columns and 4 rows. HD draws a centered 16:9 frame fitted inside the selected image area as a composition guide; it does not crop or resize the preview. All guides follow the selected aspect ratio, stay within the preview image, and do not appear in captured photos.

While a picker is open, the shutter stays at its current position unless it would overlap the value list. If it overlaps, it temporarily moves just above the list and returns to its last position when the picker closes.

Changing a camera setting restarts the current `rpicam-vid` preview process because its controls are supplied at startup. The preview may briefly pause; the picker remains visible. Still photos use the selected settings as well.

Shutter choices are **A**, **15**, **30**, **60**, **125**, **250**, **500**, and **1000**, in that order. Numeric labels represent the denominator of the shutter fraction in seconds; for example, **60** means 1/60 s. **A** selects automatic exposure time.

ISO choices are **A**, **100**, **200**, **400**, **800**, and **1600**. These are nominal labels mapped to Raspberry Pi camera gain settings: automatic gain, then 1×, 2×, 4×, 8×, and 16×. They are not calibrated ISO measurements.

White-balance choices are **AUTO**, **DAY**, **CLOUDY**, **INDOOR**, **FLUOR**, and **TUNG**. The compact picker labels map to the corresponding **AUTO**, **DAYLIGHT**, **CLOUDY**, **INDOOR**, **FLUORESCENT**, and **TUNGSTEN** `rpicam` AWB modes. The bottom-row readout shortens the longer values to **CLDY**, **INDR**, and **FLOR**.

Sharpness choices are **0.0**, **0.5**, **1.0**, **1.5**, and **2.0**, with **1.0** as the default amount. **0.0** disables sharpening, values between **0.0** and **1.0** reduce it, and values above **1.0** add sharpening. Tap **SH** to open the picker. A selection applies to the preview and still captures and restarts the preview. See the [Raspberry Pi sharpness documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html).

Metering choices are **CENTRE**, **SPOT**, and **AVERAGE**. Tap **MTR** to open the picker; the selected mode is highlighted. Its guide stays visible over the live preview after the picker closes: **SPOT** uses a small crosshair, **CENTRE** uses nested squares to suggest center weighting, and **AVERAGE** uses evenly spaced marks across the frame to suggest whole-frame weighting. Choosing a mode applies it to both preview and still capture and restarts the preview. The guides do not affect the captured image.

EV choices range from **-10.0** to **+10.0** in one-third-stop steps, with **0** as the default. Opening **EV** puts the current value in the center and shows two neighboring values on each side. The size hierarchy is fixed: the center value is large, the adjacent values are medium, and the two outer values are small. The active value is white; the others are dimmed. Tap any visible value to apply it immediately. The new value moves to the center, the live preview restarts with the updated EV, and the selector stays open. Each selection resets the five-second idle timeout. To move farther through the range, keep tapping the outermost visible value; there are no separate **− / +** buttons. The selector closes when the shutter is tapped or when no value is selected for five seconds. The shutter button stays at its saved position unless it would overlap the EV row; if so, it moves above the row and returns when the picker closes. EV compensation adjusts the camera's automatic exposure/gain targets; if both shutter and ISO/gain are fixed manually, the camera has no automatic exposure variable to adjust, so EV compensation cannot change the exposure. See the [Raspberry Pi rpicam EV documentation](https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/camera/rpicam_options_common.adoc).

Aspect-ratio choices appear in increasing order: **1:1**, **4:3**, **3:2**, **16:9**, and **65:24**. The default is **4:3**. Choosing a ratio changes the live-view framing and the still-photo output dimensions. The preview stream uses the display's full width and a matching height: on a 720-pixel-wide panel, **720×720**, **720×540**, **720×480**, **720×406**, and **720×266**, respectively. Heights are rounded to even pixels for YUV420. The view is fitted inside the square display, leaving black space above and below for wider views. In **1:1** mode the live view fills the entire screen; the controls, picker, and status messages overlay it with a dark text shadow for legibility. Still output sizes are **3040×3040**, **4056×3040**, **4056×2704**, **4056×2282**, and **4056×1498**, respectively; the latter two are the nearest practical integer dimensions to their labeled ratios while retaining the sensor's full width.

The shutter is a **50% transparent white button**. Tap it to capture in the selected format in `~/Pictures/Camera` on the Pi; drag it to reposition it below the top controls. Dragging moves the button without taking a photo.

Captures run on a worker thread while the UI loop continues polling. Taps during capture are ignored. The app briefly pauses live view during capture, then starts it again. The still camera defaults to a 350 ms warm-up (previously 1200 ms). On a Pi Zero 2 W with the HQ Camera, two direct captures at 350 ms took 1.65–2.07 seconds; two at 1200 ms took 2.50–3.24 seconds. Their exposure time and ISO matched, and the images looked similar in the tested low-light scene. `rpicam-still --zsl` took 4.20–5.40 seconds in the same test, so it is disabled by default; set `CAMERA_SQUARE_ENABLE_ZSL=1` to try it on other hardware when supported. `CAMERA_SQUARE_STILL_TIMEOUT_MS=1200` restores the longer warm-up if automatic exposure or white balance is unreliable; values from 0 through 5000 are accepted. The service log prints preview-stop, still-process, and preview-return timings for each shot.

The top line runs left to right: gallery, screenshot, **C**, **B**, **SAT**, **DN**, **HDR**, **GRID**, and **PEAK**. Tap the screenshot icon to save a JPEG of the current display, including the live view and overlays, to `~/Pictures/Debug`. Set `CAMERA_SQUARE_CAMERA_DIR` or `CAMERA_SQUARE_DEBUG_DIR` to override either directory. The gallery icon opens the photo gallery; **PEAK** toggles focus peaking.

The **C**, **B**, **SAT**, **DN**, and **HDR** readouts control contrast, brightness, saturation, denoise, and high dynamic range. Labels are bright white and current values are softer white. Tap a readout to open its full-width picker over the preview. The selected value is highlighted; tap another value to apply it immediately. The picker stays open after the change and closes when the shutter is pressed or five seconds pass without another selection. Changing a value restarts the live preview briefly. Selected modes are passed to both `rpicam-vid` and `rpicam-still`.

Contrast defaults to **1.0** and offers **0.0**, **0.5**, **1.0**, **1.5**, and **2.0**. **0.0** is the minimum contrast, **1.0** is standard, and values above **1.0** increase contrast. Brightness defaults to **0.0** and offers **-1.0**, **-0.5**, **0.0**, **+0.5**, and **+1.0**; **-1.0** is minimum brightness, **0.0** is standard, and **+1.0** is maximum brightness. Saturation defaults to **1.0** and offers **0.0**, **0.5**, **1.0**, **1.5**, and **2.0**; **0.0** produces grayscale, **1.0** is standard, and values above **1.0** increase saturation. These are app picker steps rather than the full numeric input range; contrast and saturation also accept values beyond the current picker choices. See the [Raspberry Pi camera software documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html).

Denoise defaults to **AUTO**. Its picker choices are **AUTO** (`auto`), **OFF** (`off`), **CDN-OFF** (`cdn_off`, spatial denoise only), **FAST** (`cdn_fast`), and **HQ** (`cdn_hq`). Auto uses standard spatial denoise; the camera stack chooses its colour-denoise behavior for the capture mode. **OFF** disables spatial and colour denoise, while **CDN-OFF** disables only colour denoise. **FAST** enables fast colour denoise and **HQ** enables high-quality colour denoise. Colour denoise can reduce video throughput, especially **HQ**; use it only when the slower preview is acceptable. See the [Raspberry Pi denoise option reference](https://www.raspberrypi.com/documentation/computers/camera_software.html).

HDR defaults to **OFF** and offers **OFF**, **AUTO**, and **S-EXP** (`single-exp`). **AUTO** enables a supported sensor HDR mode or the platform's on-board HDR when available. **S-EXP** requests on-board single-exposure HDR where available. HDR support depends on the camera sensor and Raspberry Pi model; unsupported combinations may leave HDR disabled. The Pi 5 and newer provide on-board HDR. Camera Square exposes these modes, but actual HDR output needs to be confirmed on the connected hardware. See the [Raspberry Pi HDR and denoise option reference](https://www.raspberrypi.com/documentation/computers/camera_software.html).

The capture-format control selects **JPG**, **DNG**, or **JPG/DNG**. DNG uses the camera stack's raw Bayer capture and DNG output. In **DNG** mode the temporary JPEG is removed after a successful capture; **JPG/DNG** keeps both files with the same base name. The gallery currently displays JPEGs, so DNG-only captures do not appear there.

## Camera control progress tracker

Statuses describe controls exposed by Camera Square, not every option supported by `rpicam`. Hardware support can also vary. The control names follow the [Raspberry Pi camera software documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html).

| Camera control | Status | Missing / note |
|---|---|---|
| Sharpness | In progress | 0.0 to 2.0 selectable in 0.5 steps; arbitrary numeric values are not exposed |
| Contrast | In progress | 0.0 to 2.0 selectable in 0.5 steps; wider numeric range is not exposed |
| Brightness | In progress | −1.0 to +1.0 selectable in 0.5 steps; arbitrary numeric values are not exposed |
| Saturation | In progress | 0.0 to 2.0 selectable in 0.5 steps; wider numeric range is not exposed |
| EV compensation | OK | Full documented −10.0 to +10.0 range is selectable in one-third-stop steps |
| Shutter speed | In progress | Custom speeds and long exposures; app has Auto and fixed choices up to 1/1000 s |
| Gain / ISO | In progress | Continuous control and actual gain readout; current ISO labels map to discrete gain values |
| Metering mode | OK | Centre, spot, and average modes selectable for preview and still capture; persistent preview guides |
| Exposure profile | NOK | — |
| White balance mode | In progress | Incandescent and custom modes |
| Fixed white-balance gains | NOK | — |
| Denoise mode | In progress | Auto, off, colour-denoise off, fast, and high-quality choices; throughput and still/video behavior need hardware review |
| Tuning file | NOK | — |
| Autofocus controls / lens position | NOK | HQ Camera lens is manually focused; no motorized focus |
| Region of interest (ROI) | NOK | — |
| HDR | In progress | Off, auto, and single-exposure modes exposed; support depends on camera and Raspberry Pi model |
| Focus peaking (app feature) | OK | Preview-only cherry-red edge overlay |

## Gallery

Tap the gallery icon at the left of the top line to open the gallery at the newest saved JPEG. Swipe left for older photos and right for newer photos. Tap the back arrow in the gallery to return to the live camera view. Photos are scaled to fit the display while keeping their original aspect ratio.

## Focus peaking

Tap **PEAK** at the right end of the top line to toggle focus peaking on the live view. High-contrast edges are tinted cherry red to help judge focus. The overlay is preview-only; saved photos are unchanged.

## Build and run

The app targets Raspberry Pi OS/Debian AArch64. It uses `rpicam-vid` and `rpicam-still` from `/usr/bin`, the `/dev/fb0` framebuffer, and a multitouch event device. Install Rust/Cargo and the Raspberry Pi camera software first. Build on the Pi with `cargo build --release --jobs 1`; the release profile avoids link-time optimization to fit the Pi's memory. `cargo fmt -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo check --all-targets --all-features` are also supported on a development host.

Install the executable and systemd unit, then start the service for your Linux account:

```sh
install -m 0755 target/release/camera-square-rs "$HOME/camera-ui-rust"
sudo install -m 0644 camera-square-rust@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now "camera-square-rust@${USER}.service"
```

The account must belong to the `video` and `input` groups. Photos and UI screenshots default to `~/Pictures/Camera` and `~/Pictures/Debug`.

For touch diagnostics, `CAMERA_SQUARE_TOUCH_DEVICE=/dev/input/eventN` selects a specific input device and `CAMERA_SQUARE_TRACE_TOUCH=1` logs touch targets and gesture outcomes. Camera access uses the `rpicam` subprocesses. Settings return to defaults when the app restarts.

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
