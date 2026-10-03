# Progress

## Current round

- Added `vcam snapshot` to capture a same-size P5 grayscale clean plate from the physical camera.
- Added `--difference-from`, `--mask-threshold`, and `--mask-softness` for fixed-camera background subtraction without an external model runtime.
- Reused the existing mask feather, background compositing, and Light Wrap path for generated difference masks.
- Static `--mask` and dynamic `--difference-from` are mutually exclusive to keep the frame path unambiguous.

## Verification

- `cargo fmt -- --check` passes.
- `cargo test`: 14 tests passed, including difference threshold/softness and CLI exclusivity.
- `cargo build --release` passes.
- Real camera `snapshot` produced a valid P5 `1280x720` clean plate at 921,616 bytes.
- Real-device difference-mask smoke test: `/dev/video0` MJPG 1280x720 -> `/dev/video20` YUYV 1280x720, approximately `23.6-24.6 ms/frame` across four 30-frame windows.
- The difference smoke test used a same-scene reference for pipeline validation, not a clean-room quality claim.
- `git diff --check` passes.

## Next steps

- Capture a true empty-room clean plate, then test person silhouette, hair, glasses, hands, and microphone edges.
- Add temporal mask smoothing if a real clean plate shows frame-to-frame jitter.
- Consider a model-backed segmentation adapter only if background subtraction cannot meet the visual target.
- Keep Rayon, native OBS FFI, and hot-plug recovery deferred until mask quality is stable.
