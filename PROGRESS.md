# Progress

## Current round

- Fixed ambiguous V4L2 `enum_formats` method resolution by explicitly using `Capture`.
- Added a Rust `vcam` binary for Linux V4L2 capture and `v4l2loopback` output.
- Added automatic selection of the first physical non-loopback capture device.
- Added MJPEG and YUYV input handling, RGB-to-YUYV conversion, and a configurable luma smoothing pass.
- Added `list` and `run` CLI commands plus focused unit tests.
- Added README with Arch Linux setup and OBS configuration.

## Verification

- `cargo fmt -- --check` and `git diff --check` completed.
- `cargo test` and `cargo test --no-run` passed (2 unit tests).

## Next steps

- Run a live `/dev/video*` to OBS smoke test when V4L2 hardware is available.
- Replace the placeholder smoothing pass with face/skin-region processing once the V4L2 bridge is stable.
- Add hot-plug recovery and persistent camera selection after the single-camera path is validated.
