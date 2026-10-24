# Camera Square

[![CI](https://github.com/vcs222/camera-square-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/vcs222/camera-square-rs/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A touchscreen camera for the Raspberry Pi Zero 2 W, HQ Camera, and HyperPixel 4 Square. Camera Square pairs a live view with direct controls for exposure, color, framing, and capture format—without a desktop environment.

## Highlights

- Live camera preview and still capture through Raspberry Pi's `rpicam` tools.
- On-screen controls for shutter, gain, white balance, metering, EV, and image adjustments.
- JPEG, raw DNG, and paired JPEG+DNG capture.
- Composition grids, aspect-ratio framing, focus peaking, and an on-device gallery.
- A small Rust application designed to run directly on Raspberry Pi OS.

## Hardware and software

Camera Square targets 64-bit Raspberry Pi OS or Debian on AArch64. It expects:

- A Raspberry Pi camera supported by the installed `rpicam` stack.
- `/usr/bin/rpicam-vid` and `/usr/bin/rpicam-still`.
- A Linux touchscreen with a 16-bit RGB565 `/dev/fb0` framebuffer and multitouch input.
- A service account in the `video` and `input` groups.

The project is designed around the Pi Zero 2 W, HQ Camera, and HyperPixel 4 Square. Camera controls and performance vary by sensor and Raspberry Pi model; see [camera support](docs/camera-support.md).

## Build and install

Install Rust 1.85 or newer and the Raspberry Pi camera software. Build on the Pi:

```sh
cargo build --release --jobs 1
install -m 0755 target/release/camera-square-rs "$HOME/camera-ui-rust"
sudo install -m 0644 camera-square-rust@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now "camera-square-rust@${USER}.service"
```

The account must be in the `video` and `input` groups. Log out and back in after adding it to either group. The service runs as that account and starts at boot.

## Using the camera

Tap a control to open its value picker. Pickers appear below the top-row controls and above the bottom-row controls. A selection applies immediately; the picker remains open for another choice and closes five seconds after the last setting-control tap or value selection, or when the shutter is pressed. Changing most camera settings briefly restarts the live preview.

Captures are saved under `~/Pictures/Camera`; display screenshots go to `~/Pictures/Debug`. Settings return to their defaults when the app restarts.

See the [controls guide](docs/controls.md) for setting ranges and behavior, and [capture notes](docs/capture.md) for output formats, directory overrides, and timing options.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
```

The GitHub Actions workflow also checks the declared minimum Rust version and audits dependencies. Host builds cannot verify camera, framebuffer, or touchscreen behavior; include your hardware and OS when reporting device-specific issues. See [contributing](CONTRIBUTING.md) and the [security policy](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
