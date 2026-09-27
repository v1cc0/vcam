# Progress

## Current round

- Added `--framerate`/`-framerate` to set physical capture and loopback output rates.
- Added `--vf`/`-vf` support for 90-degree transpose, 180-degree rotation, and horizontal/vertical flips applied to the loopback frames.
- Added a Rust `vcam` binary for Linux V4L2 capture and `v4l2loopback` output.
- Added automatic selection of the first physical non-loopback capture device.
- Added MJPEG and YUYV input handling, RGB-to-YUYV conversion, and a configurable luma smoothing pass.
- Added `list` and `run` CLI commands plus focused unit tests.
- Added README with Arch Linux setup and OBS configuration.

## Verification

- `cargo fmt -- --check`, `git diff --check`, `cargo test` (5 tests), and CLI help output passed.

## Next steps

- Verify live webcam-to-loopback output, frame rate, and filter orientation on the user's V4L2 setup.
- Arbitrary FFmpeg filter graphs remain unsupported; add filters only when implemented on the shared output path.
- Replace the placeholder smoothing pass with face/skin-region processing once the V4L2 bridge is stable.
