# vcam

A small Rust beauty-camera bridge for OBS on Linux. It captures a physical USB webcam through V4L2, applies a lightweight luma smoothing pass, and publishes a standard V4L2 output through `v4l2loopback`.

## Requirements

- Linux with V4L2
- Rust stable
- `clang` (required by `v4l2-sys-mit` during build)
- `v4l2loopback-dkms` and `v4l2loopback-utils`
- OBS Studio

Install the runtime pieces on Arch Linux:

```sh
sudo pacman -S clang v4l-utils v4l2loopback-dkms
sudo modprobe v4l2loopback video_nr=20 card_label=vcam exclusive_caps=1
```

The output device will normally be `/dev/video20`; use the actual device shown by `v4l2-ctl --list-devices` if the number differs.

## Build

```sh
cargo build --release
```

## Run

Open the GPUI Kit AMCap-style live preview without creating a loopback output:

```sh
cargo run --release -- --preview --source /dev/video1 --width 1280 --height 720 --framerate 30
```

The preview owns the physical V4L2 capture device and displays the latest decoded frame in a native window. Its control panels are intentionally read-only in this first pass; `vcam run` remains the OBS loopback path.

List V4L2 nodes and capabilities:

```sh
cargo run --release -- list
```

Run the deterministic CPU benchmark without a camera or loopback device:

```sh
cargo run --release -- bench
```

Capture a clean-plate grayscale reference while the room is empty and the camera/light setup is unchanged:

```sh
cargo run --release -- snapshot \
  --source /dev/video0 \
  --output background.pgm \
  --width 1280 --height 720 --framerate 30 --frames 30
```

Start the bridge. With no `--source`, the first non-loopback physical capture device is selected:

```sh
cargo run --release -- run --output /dev/video20 --width 1280 --height 720 --strength 0.35
```

Useful options:

- `--source /dev/video1`: select a specific camera
- `--output /dev/video20`: select the loopback device consumed by OBS
- `--width 1280 --height 720`: request the camera capture size
- `--framerate 30` or `-framerate 30`: request the capture and loopback frame rate
- `--vf transpose=1` or `-vf transpose=1`: rotate clockwise; `transpose=2` rotates counterclockwise; `hflip` and `vflip` are also supported
- `--strength 0`: disable smoothing; `1` is the strongest smoothing
- `--slim 0.33`: squash the whole frame vertically. From 720: `0.33` is 600, `0.67` is 480, `1` is 360. Width stays 1280
- `--preset first-take`: apply the CPU SIMD MVP look: lifted blacks, softer contrast, lower chroma, and a subtle warm bias. `natural` is the default
- `--mask PATH`: load a same-size PGM mask; white keeps the camera subject and black replaces the background
- `--dump-mask PATH`: save the first runtime difference mask as a PGM for diagnosis
- `--mask-threshold N`: luma difference required before a pixel becomes foreground (default `28`)
- `--mask-softness N`: luma difference ramp used for a soft foreground edge (default `32`)
- `--background R,G,B`: replacement background color when a mask is active (default `232,232,232`)
- `--light-wrap N`: feathered edge light wrap strength, `0..1` (default `0.12`)
- `--stats`: print a rolling 30-frame pipeline timing while running
- `--compare`: raw camera on the left, processed `/dev/video20` on the right. Press `q` in either window to stop vcam
- `--no-preview`: do not start ffplay (useful when viewing only in OBS). Press `q` in the preview to stop vcam; `--no-preview` keeps the bridge running for OBS

`--vf` transforms the actual loopback frames, so ffplay and OBS show the same result. `ffplay` opens the processed loopback output automatically by default. The USB camera cannot be opened by a second `ffplay` while vcam is capturing, so `--compare` feeds the left window from the frame vcam just decoded:

```sh
cargo run --release -- run --output /dev/video20 --width 1280 --height 720 \
  --preset first-take --strength 0.25 --compare --stats
```

The first-take luma pass is deliberately conservative: it compresses the upper range and applies only a small shadow lift. This keeps the high-key background without turning the face, hair, and shirt into clipped white. `--mask` adds the first background-compositing path with mask feathering and Light Wrap, but it is intentionally not a segmentation model or face-aware skin retouching system. In OBS, add **Video Capture Device** and choose the loopback card labeled `vcam`. Set its resolution/FPS to match the bridge output.

For the mask path, create a grayscale PGM at the capture resolution. White (`255`) keeps the subject; black (`0`) becomes the background; gray values provide transparency. The mask is softened with a 3x3 neighborhood before compositing:

```sh
# Example invocation after producing mask.pgm with a segmentation tool
cargo run --release -- run --source /dev/video0 --output /dev/video20 \
  --width 1280 --height 720 --preset first-take \
  --mask mask.pgm --background 232,232,232 --light-wrap 0.12 \
  --compare --stats
```

The difference-mask path is useful for a fixed bedroom camera without a model runtime:

```sh
cargo run --release -- run --source /dev/video0 --output /dev/video20 \
  --width 1280 --height 720 --preset first-take \
  --difference-from background.pgm \
  --dump-mask /tmp/vcam-mask.pgm \
  --mask-threshold 28 --mask-softness 32 \
  --background 232,232,232 --light-wrap 0.12 \
  --compare --stats
```

The difference mask first removes the frame-wide median luma shift, which prevents a small auto-exposure change from turning the entire room into foreground. It still requires a genuinely empty clean plate; if the plate contains the person, the algorithm cannot recover the missing subject pixels.

The path is resolved relative to the directory where `vcam` is launched; use an absolute path when the mask lives elsewhere.

The repository also contains `mask.pgm`, a rough `1280x720` fixed-camera soft ellipse generated for the supplied screenshot. It is useful for testing the compositing path immediately, but it is not semantic person segmentation: room pixels inside the ellipse remain visible, and the mask must be replaced when the camera framing changes.

The PGM mask is deliberately an input contract in this MVP. A segmentation model will be added only after this edge/compositing stage is visually validated.

## Current scope

The MVP supports MJPEG and YUYV USB camera input and publishes YUYV output. The default Rust processing pass smooths luma using a single 3x3 neighborhood filter. The `first-take` preset uses `fearless_simd` runtime dispatch for its luma tone pass and scalar-safe tail handling, then adjusts YUV chroma. The optional mask path accepts P2/P5 PGM, applies a 3x3 feather, composites a configurable background, and adds bounded edge Light Wrap. Face detection, automatic skin segmentation, landmark tracking, makeup overlays, camera hot-plug recovery, and a native OBS UI remain future work.

## Verification

```sh
cargo fmt -- --check
cargo test
```

`cargo test` requires `clang` and a usable system linker. The current development host has both available; if a fresh Linux setup fails, install the packages listed above and check the active Cargo linker configuration.
