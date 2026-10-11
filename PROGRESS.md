# Progress

## Current round

- Added `gpui-kit 0.7.1` and `image 0.25.10` to support the first native preview UI.
- Added `vcam --preview [options]` without changing the existing `vcam run` OBS loopback path.
- Added `src/preview.rs`: V4L2 capture thread, MJPEG/YUYV decode reuse, latest-frame state, YUYV-to-BGRA conversion, GPUI Kit window and AMCap-style layout.
- Added clean shutdown signaling for the capture thread when the preview entity is dropped.
- Updated `PLAN.md` and `README.md` with the preview scope and next stages.

## Verification

- `cargo fmt --all -- --check` passes.
- `cargo check` passes.
- `cargo build --release` passes.
- `cargo test`: 18 tests passed, including the preview color conversion regression test.
- `cargo run -- --help` lists `vcam --preview [options]`.
- Short GUI smoke test with `/dev/video1` at `640x480@15` kept the GPUI event loop alive for 12 seconds without capture or window errors; a Wayland screenshot confirmed live frames, frame counter updates, and the AMCap layout.

## Next steps

- Wire real V4L2 controls and device/format selection into the UI.
- Add screenshot, recording, and loopback output actions.
- Measure 30-second CPU/memory and frame delivery behavior; then decide whether the GPUI image path needs a lower-rate preview or a separate video surface.

## User files

- Keep the untracked `background.pgm` unchanged and out of commits.
- `PROGRESS.md.lastround` is the local backup for this round and remains ignored.
