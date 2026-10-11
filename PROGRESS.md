# Progress

## Current round

- Added `gpui-kit 0.7.1` and `image 0.25.10` to support the first native preview UI.
- Added `vcam --preview [options]` without changing the existing `vcam run` OBS loopback path.
- Added `src/preview.rs`: V4L2 capture thread, MJPEG/YUYV decode reuse, latest-frame state, YUYV-to-BGRA conversion, GPUI Kit window and AMCap-style layout.
- Added clean shutdown signaling for the capture thread when the preview entity is dropped.
- Fixed the GPUI per-frame image atlas leak: pending camera frames are converted during render, and the previous `RenderImage` is explicitly dropped before the next one is uploaded.
- Updated `PLAN.md` and `README.md` with the preview scope and next stages.

## Verification

- `cargo fmt --all -- --check` passes.
- `cargo check` passes.
- `cargo build --release` passes.
- `cargo test`: 18 tests passed, including the preview color conversion regression test.
- `cargo run -- --help` lists `vcam --preview [options]`.
- Short GUI smoke test with `/dev/video1` at `640x480@15` kept the GPUI event loop alive for 12 seconds without capture or window errors; a Wayland screenshot confirmed live frames, frame counter updates, and the AMCap layout.
- Before the leak fix, the kernel logged `amdgpu ... Not enough memory for command submission`, app-slice OOM activity, and `ghostty`/`btm` aborts after a long preview run.
- After the fix, a 45-second Wayland run at `640x480@15` held vcam RSS at about `155–157 MB` with no new `amdgpu` or OOM log entries.
- A release build run at `1280x720@30` held RSS at about `115.8 MB` for 90 seconds, with no new `amdgpu` or OOM log entries; the process ended only because the test timeout sent TERM.

## Next steps

- Measure longer CPU/memory and frame delivery behavior at production resolution; keep the image-atlas eviction path covered before increasing preview rate.
- Wire real V4L2 controls and device/format selection into the UI.
- Add screenshot, recording, and loopback output actions.

## User files

- Keep the untracked `background.pgm` unchanged and out of commits.
- `PROGRESS.md.lastround` is the local backup for this round and remains ignored.
