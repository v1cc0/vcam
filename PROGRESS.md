# Progress

## Current round

- Diagnosed the screenshot's noisy/ghosted background: a one-frame clean plate plus low threshold treated JPEG noise and lighting drift as foreground.
- Changed `vcam snapshot` to average 30 clean frames by default; added `--frames N` for control.
- Raised the default `--mask-threshold` from 18 to 28 while keeping `--mask-softness 32`.
- Added an accumulation unit test for averaged YUYV luma.

## Verification

- `cargo fmt -- --check` passes.
- `cargo test`: 15 tests passed.
- `cargo build --release` passes.
- `snapshot --frames 3` produced a valid P5 `1280x720` file of 921,616 bytes.
- The previous screenshot's ghost background remains expected when `background.pgm` was captured with the subject in frame; regenerate it from an empty room.
- `git diff --check` passes.

## Next steps

- Regenerate `background.pgm` with the room empty, fixed exposure/focus, and unchanged lighting.
- Run the difference path with defaults first; tune threshold only after inspecting a clean-room result.
- Add temporal mask smoothing if a valid clean plate still flickers around hair, glasses, hands, or the microphone.
- Consider a model-backed segmentation adapter only if background subtraction cannot meet the visual target.
