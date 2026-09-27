# Progress

## Current round

- Fixed normal ffplay exit handling so closing the preview does not report a `BrokenPipe` error or restart ffplay.
- Documented the actual processing scope: one 3x3 luma smoothing pass, plus the existing frame rotation/flip path.
- Added a three-terminal raw-versus-loopback comparison command.
- Added a Rust `vcam` binary for Linux V4L2 capture and `v4l2loopback` output.
- Added automatic selection of the first physical non-loopback capture device.
- Added MJPEG and YUYV input handling, RGB-to-YUYV conversion, and a configurable luma smoothing pass.
- Added `list` and `run` CLI commands plus focused unit tests.
- Added README with Arch Linux setup and OBS configuration.

## Verification

- `cargo fmt -- --check`, `git diff --check`, `cargo test` (5 tests), and `cargo build --release` passed.

## Next steps

- Verify the three-terminal raw-versus-processed comparison on the user's desktop.
- A face-aware skin retouching algorithm remains future work; current strength values only control luma blur.
- Add hot-plug recovery and persistent camera selection after the single-camera path is validated.
