# vcam

A small Rust beauty-camera bridge for OBS on Linux. It captures a physical USB webcam through V4L2, applies a lightweight skin-smoothing pass, and publishes a standard V4L2 output through `v4l2loopback`.

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
./target/release/vcam list
```

Start the bridge. With no `--source`, the first non-loopback physical capture device is selected:

```sh
./target/release/vcam run --output /dev/video20 --width 1280 --height 720 --strength 0.35
```

Useful options:

- `--source /dev/video1`: select a specific camera
- `--output /dev/video20`: select the loopback device consumed by OBS
- `--width 1280 --height 720`: request the camera capture size
- `--strength 0`: disable smoothing; `1` is the strongest smoothing

In OBS, add **Video Capture Device** and choose the loopback card labeled `vcam`. Set its resolution/FPS to match the bridge output.

## Current scope

The MVP supports MJPEG and YUYV USB camera input and publishes YUYV output. The current Rust processing pass smooths luma using a small neighborhood filter. Face detection, skin segmentation, landmark tracking, makeup overlays, camera hot-plug recovery, and a native OBS UI are intentionally deferred until the basic capture/output path is stable.

## Verification

```sh
cargo fmt -- --check
cargo test
```

`cargo test` requires `clang` and a usable system linker. The development host used to bootstrap this project currently lacks `clang` and has a global Cargo linker configuration pointing at an unavailable `mold` binary; use the documented package installation and a working linker before running it.
