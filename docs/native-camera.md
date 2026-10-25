# Native camera experiment

`native-camera-probe` is an isolated experiment for using libcamera from Rust without starting `rpicam-vid` or `rpicam-still`. It does not run in the touchscreen application and does not change the current camera service. The probe asks libcamera for a 4056×3040 YUV420 still stream and a 720×540 YUV420 viewfinder stream in one session, receives 20 requests, and encodes one still with TurboJPEG on a worker thread while preview requests continue. TurboJPEG is built into the probe, so the Pi needs no additional JPEG package.

The probe can answer three hardware questions before the UI is changed:

1. Does the HQ Camera and Pi Zero 2 W accept these two streams together at the requested sizes and format?
2. How much memory and preview frame rate does the full-resolution stream use?
3. Is software JPEG encoding fast enough to beat the current `rpicam-still` capture without interrupting preview?

The ordinary application continues to support JPEG, DNG, JPEG+DNG, and all its current controls through `rpicam`. The probe produces JPEG only and does not preserve EXIF metadata yet. The optional touchscreen backend described below handles default JPEG settings and falls back to the existing process path for other settings.

## Build without camera hardware

The optional feature needs libcamera development headers, a C++ compiler, libclang, pkg-config, and CMake. On Ubuntu 26.04 ARM64:

```sh
sudo apt-get install libcamera-dev libclang-dev pkg-config cmake
cargo check --bin native-camera-probe --features native-camera-probe
cargo clippy --bin native-camera-probe --features native-camera-probe -- -D warnings
```

The default build needs none of these packages. CI compiles the probe separately on Linux ARM64. The Rust `libcamera` crate supports several libcamera versions, but its development package must match the runtime library on the target device. Installing development packages on the Pi may upgrade its working camera stack; check package changes before doing so.

### Build for the current Pi from macOS

Docker can compile an ARM64 Linux binary against the Pi's installed Raspberry Pi OS libcamera `0.6.0+rpt20251202-1` without using the Pi. The [container recipe](../cross/Dockerfile.pi-libcamera-0.6) uses Debian Trixie, the matching Raspberry Pi camera packages, and a newer Rust compiler from Trixie backports. The optional `libcamera` Rust crate needs Rust newer than the application's default minimum of 1.85.

```sh
docker build --platform linux/arm64 -f cross/Dockerfile.pi-libcamera-0.6 \
  -t camera-square-pi06-build .
mkdir -p target
docker run --rm --platform linux/arm64 \
  -v "$PWD:/workspace" -v "$PWD/target:/output" \
  -v camera-square-pi06-target:/build-target \
  -e CARGO_TARGET_DIR=/build-target \
  -e TURBOJPEG_SOURCE=vendor -e TURBOJPEG_STATIC=1 camera-square-pi06-build \
  sh -c 'cargo build --release --bin native-camera-probe --features native-camera-probe && cp /build-target/release/native-camera-probe /output/native-camera-probe-aarch64'
```

The output is `target/native-camera-probe-aarch64`. It uses the Pi's existing runtime libcamera package; installing development headers on the Pi is unnecessary. Building and linking cannot show whether the camera accepts the requested stream configuration or how fast it runs.

## Opt-in touchscreen backend

The `native-camera` feature also builds an optional backend into the touchscreen app. It uses a persistent libcamera session, passes the latest viewfinder frame to the UI through an eventfd, and saves JPEGs on a worker thread. Set `CAMERA_SQUARE_NATIVE=1` to try it. It currently activates only for the default camera settings: 4:3 JPEG at 4056×3040, with a 720×540 viewfinder. Other settings and DNG formats keep using the established `rpicam` path. Native JPEGs do not yet include the EXIF metadata written by `rpicam-still`.

Build ARM64 test binaries with the same container:

```sh
docker run --rm --platform linux/arm64 \
  -v "$PWD:/workspace" -v "$PWD/target:/output" \
  -v camera-square-pi06-target:/build-target \
  -e CARGO_TARGET_DIR=/build-target \
  -e TURBOJPEG_SOURCE=vendor -e TURBOJPEG_STATIC=1 camera-square-pi06-build \
  sh -c 'cargo build --release --features native-camera --bin camera-square-rs --bin native-ui-smoke && cp /build-target/release/camera-square-rs /output/camera-ui-native-aarch64 && cp /build-target/release/native-ui-smoke /output/native-ui-smoke-aarch64'
```

Copy the app to a separate filename on the Pi and run it with `CAMERA_SQUARE_NATIVE=1` while the existing service is stopped. Restore the service afterward; do not overwrite its installed binary during evaluation. The `native-ui-smoke` binary tests the same backend without opening the display or touch device.

On the current Pi, the tested binary is installed as `/home/csaba/camera-ui-rust-native`. A drop-in at `/etc/systemd/system/camera-square-rust.service.d/native.conf` sets `CAMERA_SQUARE_NATIVE=1` and points `ExecStart` to that binary. The previous `/home/csaba/camera-ui-rust` binary remains available. To return to it, move the drop-in out of that directory, run `sudo systemctl daemon-reload`, and restart `camera-square-rust.service`.

## HQ Camera test on Pi Zero 2 W

On 25 October 2026, the Pi accepted both streams without adjusting their size or format. Preview completion intervals were about 85 ms before capture. Copying the still YUV planes took 162–164 ms, which made one preview interval 163–165 ms. Preview then returned to roughly 63–86 ms intervals while TurboJPEG encoded the still on a worker thread. The JPEG was saved 578–604 ms after the selected still frame completed in two runs. Both outputs decoded as 4056×3040 JPEGs, and one was visually checked for plane layout. The camera service was restarted after each test.

For comparison, a standalone `rpicam-still` command at 4056×3040, quality 95, and a 350 ms timeout took 1,744 ms to exit in the same session. These numbers are indicative: the native probe chooses a frame from an already-running camera, whereas the `rpicam-still` command includes camera startup. The application still uses the process-backed path; it has not yet been timed with a native preview and shutter integration.

The UI backend's smoke test also ran on the Pi. Its preview frames arrived about every 85 ms; copying the still took 181 ms and made one interval 186 ms. The JPEG was saved 605 ms after the chosen frame, while subsequent preview frames continued.

A physical touchscreen test then recorded three shutter taps in the opt-in native UI. The app reported `CAPTURING` and `SAVED`, and saved three decodable 4056×3040 JPEGs in the gallery directory. Touch-to-save times were 740, 763, and 747 ms; still-frame-to-save times were 603, 602, and 580 ms. The full set of camera controls and DNG output have not been implemented in the native path.

## Running the probe again

Build the probe on a compatible Linux ARM64 system, stop the camera service so it releases the camera, then run the probe with a new temporary JPEG path. Start the service again even if the probe exits with an error. Compare its preview intervals, JPEG save time, memory use, and image quality against the installed app. Do not install the probe as the service binary.

The next implementation step depends on those results. A useful result would justify connecting the native stream to the framebuffer UI, translating all camera controls, and adding JPEG metadata and raw DNG output or an explicit fallback for those formats.
