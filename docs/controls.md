# Controls and camera settings

Camera Square puts capture and framing controls on the bottom row, with image adjustments and utilities across the top. Pickers open below top-row controls and above bottom-row controls.

## Everyday controls

| Control | Choices and behavior |
| --- | --- |
| Shutter (**SS**) | A (auto), B (disabled), T (2 seconds), 1, 2, 4, 8, 15, 30, 60, 125, 250, 500, 1000, 2000, 4000, 8000. Numeric labels are denominators except **1**, which means one second. |
| Gain (**ISO**) | Auto, 100, 200, 400, 800, or 1600. These labels map to automatic gain, then fixed gains of 1×, 2×, 4×, 8×, and 16×; they are not calibrated ISO values. |
| White balance (**WB**) | Auto, daylight, cloudy, indoor, fluorescent, tungsten. The UI shortens some labels to fit. |
| Metering (**MTR**) | Centre, spot, or average. A small on-screen guide illustrates the selected area; guides do not affect the captured image. |
| Sharpness (**SH**) | 0.0 to 2.0 in 0.1 steps; default 1.0. |
| Exposure compensation (**EV**) | −10.0 to +10.0 in one-third-stop steps. Drag the slider to set it. |
| Aspect ratio | 1:1, 4:3, 3:2, 16:9, or 65:24; default 4:3. Changes both preview framing and still output size. |
| Capture format | JPEG, DNG, or JPEG+DNG. DNG-only photos have a JPEG gallery preview stored separately from the raw file. |

**SS** opens a half-transparent rotating wheel styled after a camera shutter dial. Its printed markings rotate with the wheel and become horizontal at the fixed white line on the left. Drag the wheel or tap a marking to bring it to that line. B is dimmed and skipped because hold-to-expose bulb capture is not implemented. T takes a two-second exposure; the preview updates slowly at T and other long speeds. The native camera applies speeds during a drag; the `rpicam` compatibility path applies the final speed on release. Very fast exposure times can be rounded to the sensor's available timing steps.

## Image adjustments

The top-row **C**, **B**, and **SAT** controls adjust contrast, brightness, and saturation:

| Setting | Choices | Default |
| --- | --- | --- |
| Contrast | 0.0 to 2.0 in 0.1 steps | 1.0 |
| Brightness | −1.0 to +1.0 in 0.1 steps | 0.0 |
| Saturation | 0.0 to 2.0 in 0.1 steps | 1.0 |

Contrast, brightness, saturation, sharpness, and EV use sliders. Their half-transparent dark panels keep the live view visible underneath, while the handle and readout follow your finger. On the native camera path, the preview updates as you drag; on the `rpicam` compatibility path, it updates when you release. Tap a track to choose a value directly. A saturation of 0.0 produces grayscale.

**DN** selects denoise: Auto, Off, colour-denoise off, Fast, or High quality. Off disables spatial and colour denoise; colour-denoise off disables only colour denoise. High-quality colour denoise can reduce preview throughput. HDR is not offered in this app; see [camera support](camera-support.md) for the hardware reason.

## Composition and focus

**GRID** cycles through 9 (3×3), 24 (6×4), a centered 16:9 guide, and Off. Guides follow the selected image area and are not included in photos.

**PEAK** toggles focus peaking in the live preview. High-contrast edges are tinted red; saved photos are unchanged.

## Gallery, screenshots, and shutter

The gallery button opens the newest photo preview. A bottom bar shows its format (`JPG`, `DNG`, or `JPG+DNG`), capture name, and position in the gallery. Swipe left for older photos and right for newer ones; tap the back arrow to return to the camera. Photos are fitted to the screen without changing their aspect ratio.

The screenshot button saves the current display, including overlays, as a JPEG. The translucent shutter button captures a photo in the selected format. Drag it to reposition it below the top controls; dragging does not take a photo.

## Picker behavior

Tap a control to open its picker. Tap a choice or use its slider or dial; the picker stays open so another value can be chosen. The same control leaves its picker open, while another setting control switches to its picker. It closes five seconds after the last setting-control tap or value selection, or when the shutter is pressed.

Changing non-slider camera settings restarts the preview, so it briefly pauses. The picker remains open, and the selected settings are also used for still captures. With the native build, unavailable native controls or unsupported stream configurations switch to the `rpicam` path. Settings return to their defaults when the app restarts.

## Aspect-ratio dimensions

On a 720-pixel-wide display, preview dimensions are rounded to even heights for YUV420. Still dimensions use the sensor's full width where possible.

| Ratio | Preview | Still |
| --- | ---: | ---: |
| 1:1 | 720×720 | 3040×3040 |
| 4:3 | 720×540 | 4056×3040 |
| 3:2 | 720×480 | 4056×2704 |
| 16:9 | 720×406 | 4056×2282 |
| 65:24 | 720×266 | 4056×1498 |

The last two still sizes are the nearest practical integer dimensions to their labeled ratios while retaining the sensor's full width. For more on sensor-dependent controls, see [camera support](camera-support.md).
