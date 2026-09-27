# Progress

## Current round

- `vcam run` now launches ffplay to preview processed loopback output by default; `--no-preview` disables it for OBS-only use.
- Added a Rust `vcam` binary for Linux V4L2 capture and `v4l2loopback` output.
- Added automatic selection of the first physical non-loopback capture device.
- Added MJPEG and YUYV input handling, RGB-to-YUYV conversion, and a configurable luma smoothing pass.
- Added `list` and `run` CLI commands plus focused unit tests.
- Added README with Arch Linux setup and OBS configuration.

## Verification

- `cargo fmt -- --check`, `git diff --check`, `cargo test` (3 tests), and CLI help output passed.

## Next steps

- Verify live webcam-to-loopback output and ffplay preview on the user's V4L2 setup.
- Replace the placeholder smoothing pass with face/skin-region processing once the V4L2 bridge is stable.
- Add hot-plug recovery and persistent camera selection after the single-camera path is validated.
