# Progress

## Current round

- Added a Rust `vcam` binary for Linux V4L2 capture and `v4l2loopback` output.
- Added automatic selection of the first physical non-loopback capture device.
- Added MJPEG and YUYV input handling, RGB-to-YUYV conversion, and a configurable luma smoothing pass.
- Added `list` and `run` CLI commands plus focused unit tests.
- Added README with Arch Linux setup and OBS configuration.

## Verification

- `cargo fmt` and `cargo fmt -- --check` completed.
- `git diff --check` completed.
- Direct `rustc` parsing reached only expected unresolved external-crate errors.
- `cargo test` is blocked on this host by missing `clang` for `v4l2-sys-mit` and an unavailable global `mold` linker configuration. This is an environment prerequisite, not a reported runtime result.

## Next steps

- Install `clang` and use an available linker, then run `cargo test` and a live `/dev/video*` to OBS smoke test.
- Replace the placeholder smoothing pass with face/skin-region processing once the V4L2 bridge is stable.
- Add hot-plug recovery and persistent camera selection after the single-camera path is validated.
