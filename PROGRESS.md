# Progress

## Current round

- Added `fearless_simd = 1.0` and runtime-dispatched `first_take_luma` using native SIMD widths with scalar remainder handling.
- Added `vcam bench`, which processes a deterministic 1280x720 YUYV frame without camera/OBS dependencies.
- Added `--preset natural|first-take`; the MVP preset lifts luma, softens contrast, reduces chroma, and adds a small warm bias. Default behavior remains `natural`.
- Added `--stats` for rolling 30-frame pipeline timing.
- Reused scratch buffers for squeeze and luma smoothing instead of allocating a fresh source copy per frame.
- Updated README with the MVP test commands and explicit limits: no mask, background replacement, Light Wrap, or face-aware retouching yet.

## Verification

- `cargo fmt -- --check` passes.
- `cargo test`: 10 tests passed.
- `cargo build --release` passes.
- `cargo run --release -- bench`: 1280x720, 120 frames, `simd_fallback=false`, average `1.252 ms/frame` for the first-take kernel on this host.
- Real-device smoke test: `/dev/video0` MJPG 1280x720 -> `/dev/video20` YUYV 1280x720 with `first-take` processed two 30-frame windows at `11.68 ms` and `12.02 ms` average pipeline time; the 8-second controller timeout ended the run as expected.
- `git diff --check` passes.

## Next steps

- Test `cargo run --release -- run --output /dev/video20 --preset first-take --compare --stats` with the real camera and OBS.
- Compare `natural` versus `first-take` on the night bedroom setup and tune only after seeing the actual feed.
- Add a real mask input and edge compositing/Light Wrap after the CPU color pass is visually acceptable.
- Keep ONNX/model inference, Rayon, native OBS FFI, and hot-plug recovery deferred until the basic stream is measured.
