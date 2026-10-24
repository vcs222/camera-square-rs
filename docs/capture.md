# Capture and configuration

## Still capture

Still photos use the selected camera settings and aspect ratio. JPEG is the default format; DNG captures use the camera stack's raw Bayer output. Selecting DNG removes the accompanying temporary JPEG after a successful capture. JPEG+DNG keeps both files with the same base name. The gallery displays JPEGs only.

The app pauses the preview while `rpicam-still` owns the camera, then starts the preview again. Capture runs on a worker thread; touch input is ignored until it finishes. The service log reports preview-stop, still-process, preview-restart, total-capture, and first-preview-after-capture timings.

## Capture tuning

The still camera waits 350 ms by default before capture. Set `CAMERA_SQUARE_STILL_TIMEOUT_MS` to a value from 0 to 5000 to change that delay. An invalid value falls back to 350 ms. A longer delay can help when automatic exposure or white balance needs more time to settle.

Zero-shutter-lag capture is disabled by default. Set `CAMERA_SQUARE_ENABLE_ZSL=1` to request it; the app first checks whether the installed `rpicam-still` supports `--zsl`.

An informal comparison on a Pi Zero 2 W with an HQ Camera used two low-light captures per setting. Captures at 350 ms took 1.65–2.07 seconds; at 1200 ms, 2.50–3.24 seconds. Exposure time and ISO matched, and the images looked similar in that scene. `--zsl` took 4.20–5.40 seconds in the same comparison, so it remains opt-in. These small samples are observations, not general performance guarantees.

## Output directories and touch diagnostics

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `CAMERA_SQUARE_CAMERA_DIR` | `~/Pictures/Camera` | Photo and DNG output; the gallery scans this directory. |
| `CAMERA_SQUARE_DEBUG_DIR` | `~/Pictures/Debug` | JPEG screenshots of the display and overlays. |
| `CAMERA_SQUARE_TOUCH_DEVICE` | Auto-detected device | Select a specific touchscreen device, such as `/dev/input/event1`. |
| `CAMERA_SQUARE_TRACE_TOUCH` | Unset | Set to any value to log touch targets and gesture outcomes. |

For a systemd instance, add overrides with `sudo systemctl edit "camera-square-rust@${USER}.service"`:

```ini
[Service]
Environment=CAMERA_SQUARE_STILL_TIMEOUT_MS=1200
Environment=CAMERA_SQUARE_CAMERA_DIR=/path/to/photos
```

Then apply them with `sudo systemctl restart "camera-square-rust@${USER}.service"`. Settings selected in the UI itself are not persisted; they return to defaults after a restart.
