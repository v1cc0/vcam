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

List V4L2 nodes and capabilities:

```sh
cargo run --release -- list
```

Run the deterministic CPU benchmark without a camera or loopback device:

```sh
cargo run --release -- bench
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
- `--background R,G,B`: replacement background color when `--mask` is active (default `245,245,245`)
- `--light-wrap N`: feathered edge light wrap strength, `0..1` (default `0.12`)
- `--stats`: print a rolling 30-frame pipeline timing while running
- `--compare`: raw camera on the left, processed `/dev/video20` on the right. Press `q` in either window to stop vcam
- `--no-preview`: do not start ffplay (useful when viewing only in OBS). Press `q` in the preview to stop vcam; `--no-preview` keeps the bridge running for OBS

`--vf` transforms the actual loopback frames, so ffplay and OBS show the same result. `ffplay` opens the processed loopback output automatically by default. The USB camera cannot be opened by a second `ffplay` while vcam is capturing, so `--compare` feeds the left window from the frame vcam just decoded:

```sh
cargo run --release -- run --output /dev/video20 --width 1280 --height 720 \
  --preset first-take --strength 0.25 --compare --stats
```

The current `--strength` effect remains a small luma blur/noise reduction pass. `--preset first-take` adds the first SIMD color pass. `--mask` adds the first background-compositing path with mask feathering and Light Wrap, but it is intentionally not a segmentation model or face-aware skin retouching system. In OBS, add **Video Capture Device** and choose the loopback card labeled `vcam`. Set its resolution/FPS to match the bridge output.

For the mask path, create a grayscale PGM at the capture resolution. White (`255`) keeps the subject; black (`0`) becomes the background; gray values provide transparency. The mask is softened with a 3x3 neighborhood before compositing:

```sh
# Example invocation after producing mask.pgm with a segmentation tool
cargo run --release -- run --source /dev/video0 --output /dev/video20 \
  --width 1280 --height 720 --preset first-take \
  --mask mask.pgm --background 245,245,245 --light-wrap 0.12 \
  --compare --stats
```

The path is resolved relative to the directory where `vcam` is launched; use an absolute path when the mask lives elsewhere.

## Current scope

The MVP supports MJPEG and YUYV USB camera input and publishes YUYV output. The default Rust processing pass smooths luma using a single 3x3 neighborhood filter. The `first-take` preset uses `fearless_simd` runtime dispatch for its luma tone pass and scalar-safe tail handling, then adjusts YUV chroma. The optional mask path accepts P2/P5 PGM, applies a 3x3 feather, composites a configurable background, and adds bounded edge Light Wrap. Face detection, automatic skin segmentation, landmark tracking, makeup overlays, camera hot-plug recovery, and a native OBS UI remain future work.

## Verification

```sh
cargo fmt -- --check
cargo test
```

`cargo test` requires `clang` and a usable system linker. The current development host has both available; if a fresh Linux setup fails, install the packages listed above and check the active Cargo linker configuration.
