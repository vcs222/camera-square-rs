# Native camera backend

The production app uses a persistent libcamera session for default 4:3 JPEG preview and capture when built with `native-camera`. A 720×540 YUV420 viewfinder and 4056×3040 YUV420 still stream run in the same session. The app encodes stills with TurboJPEG on a worker thread while preview continues. No environment flag is needed.

Adjusted camera settings, other aspect ratios, DNG, and JPEG+DNG continue through `rpicam-vid` and `rpicam-still` in the same app. If native startup fails, the app also falls back to `rpicam`. Native JPEGs currently lack the EXIF metadata written by `rpicam-still`.

## Build for the tested Pi Zero 2 W from macOS

The [ARM64 container recipe](../cross/Dockerfile.pi-libcamera-0.6) uses Debian Trixie and the Pi's libcamera `0.6.0+rpt20251202-1` package. The resulting binary links against the Pi's existing runtime library, so development packages are unnecessary on the device.

```sh
docker build --platform linux/arm64 -f cross/Dockerfile.pi-libcamera-0.6 \
  -t camera-square-pi06-build .
mkdir -p target
docker run --rm --platform linux/arm64 \
  -v "$PWD:/workspace" -v "$PWD/target:/output" \
  -v camera-square-pi06-target:/build-target \
  -e CARGO_TARGET_DIR=/build-target \
  -e TURBOJPEG_SOURCE=vendor -e TURBOJPEG_STATIC=1 camera-square-pi06-build \
  sh -c 'cargo build --release --features native-camera --bin camera-square-rs && cp /build-target/release/camera-square-rs /output/camera-ui-aarch64'
```

Install `target/camera-ui-aarch64` as `~/camera-ui-rust`, the single service binary. The app still requires the Raspberry Pi camera software for nondefault settings and raw capture. For another Linux ARM64 environment, install matching `libcamera-dev`, `libclang-dev`, `pkg-config`, and `cmake`, then build with `--features native-camera`.

## Hardware validation

On 25 October 2026, the HQ Camera on a Pi Zero 2 W accepted both native streams. In a physical touchscreen test, three shutter taps saved decodable 4056×3040 JPEGs. Touch-to-save times were 740, 763, and 747 ms, with preview continuing during encoding. A standalone `rpicam-still` command with a 350 ms timeout took 1,744 ms to exit in one comparison; that command includes camera startup, so the measurements are not directly equivalent.
