# Progress

## Current round

- `vcam --preview` now renders the original `/dev/video*` capture in the upper middle pane and the processed capture in the lower middle pane.
- Added live right-panel controls for brightness, contrast, and horizontal flip; controls affect only the lower processed pane.
- Added explicit `min_w_0`/`min_h_0` and non-shrinking side panels so the middle image cannot cover the right control panel.
- Kept separate original/processed GPUI `RenderImage` resources and explicitly evict each previous image before replacement.
- Updated `README.md` and `PLAN.md` to describe the split preview and live controls.

## Verification

- `cargo fmt --all -- --check` passes.
- `cargo check` passes.
- `cargo test`: 19 tests pass, including the effects regression test.
- Wayland smoke test with `/dev/video1` at `640x480@15` produced a screenshot showing both middle panes and an unobstructed right effects panel.
- niri continues to report the preview app ID as `com.v1cc0.vcam`.
- Test process and temporary screenshot/log files were removed.

## Next steps

- Measure longer CPU/memory and frame delivery behavior at production resolution.
- Wire real V4L2 controls and device/format selection into the UI.
- Add screenshot, recording, and loopback output actions.

## User files

- Keep the untracked `background.pgm` unchanged and out of commits.
- `PROGRESS.md.lastround` is the local backup for this round and remains ignored.
