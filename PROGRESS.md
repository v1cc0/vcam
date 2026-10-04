# Progress

## Current round

- Reduced `first-take` luma lift from 24 to 10; the SIMD and scalar paths now preserve more facial, hair, and shirt detail.
- Added `--dump-mask PATH` to save the first generated difference mask for diagnosis.
- Added frame-wide median luma-shift compensation before difference thresholding, reducing false foreground from auto-exposure drift.
- Reduced the default replacement background from YUV-equivalent RGB `245,245,245` to `232,232,232` to keep high-key whites from clipping.
- Confirmed from the exported mask and grayscale `background.pgm` that the current local plate still contains the person; this cannot be repaired by threshold or tone settings.

## Verification

- `cargo fmt -- --check` passes.
- `cargo test`: 17 tests passed.
- `cargo build --release` passes.
- Difference smoke test with `--preset natural --light-wrap 0` ran at approximately `20.85-21.33 ms/frame` and wrote a valid `1280x720` P5 mask.
- Added a regression test proving a uniform exposure shift does not create a difference mask.
- `git diff --check` passes.

## Next steps

- Re-capture `background.pgm` while fully out of frame, with fixed exposure/focus and unchanged lighting.
- Inspect `--dump-mask` before judging final compositing.
- Run `--preset natural --light-wrap 0` first, then enable `first-take` and Light Wrap separately.
- Add temporal mask smoothing only after a valid clean plate still flickers around hair, glasses, hands, or the microphone.
