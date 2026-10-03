# Progress

## Current round

- Added the first mask compositing MVP: optional P2/P5 grayscale PGM input through `--mask`.
- White mask pixels keep the camera subject; black pixels become the configured `--background` color.
- Added a separable integer 3x3 mask feather and bounded `--light-wrap` edge blending.
- Added `--background R,G,B` and `--light-wrap 0..1` CLI controls.
- Improved missing/invalid mask diagnostics: relative paths are explicitly reported and documented as relative to the launch directory.

## Verification

- `cargo fmt -- --check` passes.
- `cargo test`: 12 tests passed, including P2/P5 parsing and black-mask replacement.
- `cargo build --release` passes.
- Real-device mask smoke test: `/dev/video0` MJPG 1280x720 -> `/dev/video20` YUYV 1280x720, five 30-frame windows at `21.68-21.79 ms` average pipeline time.
- The smoke test used a temporary all-black PGM to validate background replacement and cleaned it up after the run.
- Missing-mask smoke check now reports `cannot read mask 'mask.pgm': No such file or directory (os error 2)` instead of an ambiguous raw `ENOENT`; `git diff --check` passes.

## Next steps

- Produce or connect a real person segmentation mask and inspect hair, glasses, hands, and microphone edges with `--compare`.
- Tune `--light-wrap` and background brightness from actual night-room footage; keep the default conservative.
- Add temporal mask smoothing only after frame-to-frame segmentation jitter is observable.
- Keep ONNX/model inference, Rayon, native OBS FFI, and hot-plug recovery deferred until the mask contract and visual quality are stable.
