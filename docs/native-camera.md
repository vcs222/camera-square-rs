# Native camera backend

The production app uses a persistent libcamera session for JPEG, DNG, and paired capture when built with `native-camera`. It configures a YUV420 viewfinder and still stream at the selected aspect ratio. DNG modes add an unpacked 12-bit Bayer stream. JPEGs are encoded with TurboJPEG and DNGs are written as TIFF-based raw files on a worker thread while preview continues. No environment flag is needed.

Shutter, gain, white balance, metering, EV, sharpness, contrast, brightness, saturation, denoise, and single-exposure HDR are applied to native requests when the camera supports those controls. The tested IMX477 HQ Camera does not support HDR despite advertising the control, so HDR settings use the `rpicam` path on that sensor. Sensor HDR auto mode on the IMX708, unavailable controls, and rejected native stream configurations also use `rpicam`. Native JPEGs include EXIF capture time, model, dimensions, exposure, and nominal ISO. Native DNGs include Bayer layout, black and white levels, colour matrix, white balance, and capture metadata.

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

Install `target/camera-ui-aarch64` as `~/camera-ui-rust`, the single service binary. The app still requires the Raspberry Pi camera software for compatibility fallback. For another Linux ARM64 environment, install matching `libcamera-dev`, `libclang-dev`, `pkg-config`, and `cmake`, then build with `--features native-camera`.

## Hardware validation

On 25 October 2026, the HQ Camera on a Pi Zero 2 W accepted both native streams. In a physical touchscreen test, three shutter taps saved decodable 4056×3040 JPEGs. Touch-to-save times were 740, 763, and 747 ms, with preview continuing during encoding. A standalone `rpicam-still` command with a 350 ms timeout took 1,744 ms to exit in one comparison; that command includes camera startup, so the measurements are not directly equivalent.

Further touchscreen tests accepted 16:9 YUV streams, white-balance changes, and a third full-resolution Bayer stream. A 16:9 JPEG saved at 4056×2282 in about 0.6 seconds. DNG-only and paired captures saved native 4056×3040 raw files; `dcraw` identified and decoded a native DNG. DNG-only capture also writes a 720×540 processed JPEG preview to `.gallery`, so the gallery can display it without placing a second full photo beside the raw file. A DNG-only touchscreen capture with this preview took 771 ms from shutter to save.
