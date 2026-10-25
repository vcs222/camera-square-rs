# Contributing

Bug reports, hardware compatibility notes, and focused pull requests are welcome.

## Before opening a pull request

- Keep changes focused and explain any user-visible behavior change.
- Run `cargo fmt --all -- --check`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo check --all-targets`.
- For native camera changes, also build on Linux with `cargo check --all-targets --features native-camera` and the `libcamera-dev`, `libclang-dev`, `pkg-config`, and `cmake` packages installed.
- For camera, framebuffer, or touch changes, include the Raspberry Pi model, camera, display, and OS used for hardware validation. Host builds cannot verify device behavior.

Do not include personal photographs, DNG captures, or other private capture data in commits or issues.

## Reporting issues

Include the command or touch sequence, expected behavior, actual behavior, and relevant journal output. Remove personal paths and image data before sharing logs.
