use std::env;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use jpeg_decoder::{Decoder, PixelFormat};
use v4l::buffer::Type;
use v4l::capability::Flags;
use v4l::format::{Format, FourCC};
use v4l::io::traits::{CaptureStream, OutputStream};
use v4l::prelude::MmapStream;
use v4l::video::{Capture, Output};
use v4l::Device;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_STRENGTH: f32 = 0.35;

#[derive(Debug)]
struct Config {
    source: Option<PathBuf>,
    output: PathBuf,
    width: u32,
    height: u32,
    strength: f32,
    preview: bool,
    compare: bool,
    framerate: u32,
    rotation: Rotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rotation {
    None,
    Clockwise,
    CounterClockwise,
    HalfTurn,
    HorizontalFlip,
    VerticalFlip,
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("list") => list_devices(),
        Some("run") => match parse_config(args) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(()),
            Ok(config) => run(config),
            Err(error) => Err(error),
        },
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown command: {command}"),
        )),
    }
}

fn parse_config(mut args: impl Iterator<Item = String>) -> io::Result<Config> {
    let mut config = Config {
        source: None,
        output: PathBuf::from("/dev/video20"),
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
        strength: DEFAULT_STRENGTH,
        preview: true,
        compare: false,
        framerate: 30,
        rotation: Rotation::None,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => config.source = Some(PathBuf::from(next_arg("--source", &mut args)?)),
            "--output" => config.output = PathBuf::from(next_arg("--output", &mut args)?),
            "--width" => config.width = parse_u32("--width", &next_arg("--width", &mut args)?)?,
            "--height" => config.height = parse_u32("--height", &next_arg("--height", &mut args)?)?,
            "--strength" => {
                config.strength =
                    next_arg("--strength", &mut args)?
                        .parse::<f32>()
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "--strength must be a number",
                            )
                        })?;
            }
            "--no-preview" => config.preview = false,
            "--compare" => config.compare = true,
            "--framerate" | "-framerate" => {
                config.framerate = parse_u32("--framerate", &next_arg("--framerate", &mut args)?)?;
                if config.framerate == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--framerate must be greater than zero",
                    ));
                }
            }
            "--vf" | "-vf" => config.rotation = parse_video_filter(&next_arg("--vf", &mut args)?)?,
            "--help" | "-h" => {
                print_help();
                return Err(io::Error::new(io::ErrorKind::Interrupted, "help requested"));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown option: {arg}"),
                ));
            }
        }
    }

    if config.width == 0 || config.height == 0 || config.width % 2 != 0 || config.height % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "width and height must be positive even integers",
        ));
    }
    if !(0.0..=1.0).contains(&config.strength) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "strength must be between 0 and 1",
        ));
    }
    if config.compare {
        config.preview = true;
    }
    Ok(config)
}

fn parse_video_filter(filter: &str) -> io::Result<Rotation> {
    match filter {
        "transpose=1" | "transpose=clock" => Ok(Rotation::Clockwise),
        "transpose=2" | "transpose=cclock" => Ok(Rotation::CounterClockwise),
        "transpose=2,transpose=2" | "transpose=clock,transpose=clock" => Ok(Rotation::HalfTurn),
        "transpose=1,transpose=1" => Ok(Rotation::HalfTurn),
        "hflip" => Ok(Rotation::HorizontalFlip),
        "vflip" => Ok(Rotation::VerticalFlip),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported --vf filter: {filter}"),
        )),
    }
}

fn next_arg(name: &str, args: &mut impl Iterator<Item = String>) -> io::Result<String> {
    args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing value for {name}"),
        )
    })
}

fn parse_u32(name: &str, value: &str) -> io::Result<u32> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be a positive integer"),
        )
    })
}

fn print_help() {
    println!(
        "vcam - Rust beauty camera bridge for OBS\n\n\
         Usage:\n  vcam list\n  vcam run [options]\n\n\
         Options:\n  --source PATH       Input V4L2 camera; omitted selects the first USB capture device\n  --output PATH       v4l2loopback output (default: /dev/video20)\n  --width N           Capture width (default: 1280)\n  --height N          Capture height (default: 720)\n  --strength N        Smoothing strength, 0..1 (default: 0.35)\n  --framerate N       Capture/output frame rate (default: 30)\n  --vf FILTER         transpose=1|2|clock|cclock, hflip, or vflip\n  --compare           Show the raw camera on the left and the processed output on the right\n  --no-preview        Do not launch ffplay to preview the output"
    );
}

fn list_devices() -> io::Result<()> {
    for path in video_nodes()? {
        let Ok(device) = Device::with_path(&path) else {
            continue;
        };
        let Ok(caps) = device.query_caps() else {
            continue;
        };
        println!(
            "{}\t{}\t{}\t{}",
            path.display(),
            caps.driver,
            caps.card,
            caps.bus
        );
    }
    Ok(())
}

fn video_nodes() -> io::Result<Vec<PathBuf>> {
    let mut nodes = fs::read_dir("/dev")?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("video"))
        })
        .collect::<Vec<_>>();
    nodes.sort();
    Ok(nodes)
}

fn select_source(explicit: Option<&Path>, output: &Path) -> io::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    for path in video_nodes()? {
        if path == output {
            continue;
        }
        let Ok(device) = Device::with_path(&path) else {
            continue;
        };
        let Ok(caps) = device.query_caps() else {
            continue;
        };
        if caps.capabilities.contains(Flags::VIDEO_CAPTURE) && !caps.driver.contains("loopback") {
            return Ok(path);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no physical V4L2 capture device found",
    ))
}

fn run(config: Config) -> io::Result<()> {
    let source_path = select_source(config.source.as_deref(), &config.output)?;
    println!(
        "source={} output={} requested={}x{} fps={} strength={:.2}",
        source_path.display(),
        config.output.display(),
        config.width,
        config.height,
        config.framerate,
        config.strength
    );

    let source = Device::with_path(&source_path)?;
    let source_format = configure_source(&source, config.width, config.height)?;
    Capture::set_params(
        &source,
        &v4l::video::capture::Parameters::with_fps(config.framerate),
    )?;
    let output = Device::with_path(&config.output)?;
    let (output_width, output_height) = if matches!(
        config.rotation,
        Rotation::Clockwise | Rotation::CounterClockwise
    ) {
        (source_format.height, source_format.width)
    } else {
        (source_format.width, source_format.height)
    };
    let output_format = Output::set_format(
        &output,
        &Format::new(output_width, output_height, FourCC::new(b"YUYV")),
    )?;
    Output::set_params(
        &output,
        &v4l::video::output::Parameters::with_fps(config.framerate),
    )?;

    if output_format.fourcc != FourCC::new(b"YUYV")
        || output_format.width != output_width
        || output_format.height != output_height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "loopback rejected YUYV {}x{}",
                source_format.width, source_format.height
            ),
        ));
    }

    println!(
        "capture={} {}x{} output={} {}x{}",
        source_format.fourcc,
        source_format.width,
        source_format.height,
        output_format.fourcc,
        output_format.width,
        output_format.height
    );

    let mut capture = MmapStream::with_buffers(&source, Type::VideoCapture, 4)?;
    let mut output_stream = MmapStream::with_buffers(&output, Type::VideoOutput, 4)?;
    // ponytail: 200ms poll so q can unblock a loopback with no reader; raise if drops show up
    if config.preview {
        output_stream.set_timeout(Duration::from_millis(200));
    }
    let (raw_tx, mut raw_preview) = if config.compare {
        open_raw_preview(source_format.width, source_format.height, config.framerate)?
    } else {
        (None, None)
    };
    let mut preview = None;
    let mut preview_started = false;
    let mut bad_jpegs = 0u32;
    let mut frame = Vec::with_capacity((source_format.width * source_format.height * 2) as usize);
    let mut rotated = Vec::with_capacity((output_format.width * output_format.height * 2) as usize);
    let mut pixels = Vec::with_capacity((output_format.width * output_format.height) as usize);
    let needed = (source_format.width * source_format.height * 2) as usize;
    let output_bytes = (output_format.width * output_format.height * 2) as usize;

    loop {
        if preview_stopped(&mut preview)? || preview_stopped(&mut raw_preview)? {
            close_child(&mut raw_preview);
            close_child(&mut preview);
            return Ok(());
        }

        let (input, metadata) = CaptureStream::next(&mut capture)?;
        let input = &input[..(metadata.bytesused as usize).min(input.len())];
        frame.clear();
        frame.resize(needed, 0);

        match source_format.fourcc.str().unwrap_or("") {
            "MJPG" | "JPEG" => {
                if let Err(error) =
                    jpeg_to_yuyv(input, &mut frame, source_format.width, source_format.height)
                {
                    if error.kind() != io::ErrorKind::InvalidData {
                        return Err(error);
                    }
                    // ponytail: skip torn USB JPEGs; surface the error after a full second of them
                    bad_jpegs += 1;
                    if bad_jpegs >= 30 {
                        return Err(error);
                    }
                    eprintln!("skipping bad JPEG frame ({bad_jpegs}): {error}");
                    continue;
                }
                bad_jpegs = 0;
            }
            "YUYV" => {
                if input.len() < needed {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "camera returned a short YUYV frame",
                    ));
                }
                frame.copy_from_slice(&input[..needed]);
            }
            format => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("unsupported capture format {format}"),
                ));
            }
        }

        if let Some(tx) = &raw_tx {
            if matches!(
                tx.try_send(frame.clone()),
                Err(mpsc::TrySendError::Disconnected(_))
            ) {
                close_child(&mut raw_preview);
                close_child(&mut preview);
                return Ok(());
            }
        }

        beautify_yuyv(
            &mut frame,
            source_format.width,
            source_format.height,
            config.strength,
        );
        apply_rotation(
            &frame,
            source_format.width,
            source_format.height,
            config.rotation,
            &mut rotated,
            &mut pixels,
        );

        let (output_buffer, output_metadata) = match OutputStream::next(&mut output_stream) {
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            result => result?,
        };
        if output_buffer.len() < output_bytes {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "loopback buffer is smaller than the requested frame",
            ));
        }
        output_buffer[..output_bytes].copy_from_slice(&rotated);
        output_metadata.bytesused = output_bytes as u32;
        output_metadata.field = 0;

        if config.preview && !preview_started {
            preview = Some(start_preview(
                &config.output,
                output_format.width,
                output_format.height,
                config.framerate,
                if config.compare {
                    source_format.width
                } else {
                    0
                },
            )?);
            preview_started = true;
        }
    }
}

fn preview_stopped(preview: &mut Option<Child>) -> io::Result<bool> {
    let Some(child) = preview.as_mut() else {
        return Ok(false);
    };
    Ok(child.try_wait()?.is_some())
}

fn close_child(preview: &mut Option<Child>) {
    if let Some(mut child) = preview.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn open_raw_preview(
    width: u32,
    height: u32,
    framerate: u32,
) -> io::Result<(Option<SyncSender<Vec<u8>>>, Option<Child>)> {
    let mut child = ffplay(raw_preview_args(width, height, framerate), true)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "raw preview stdin missing"))?;
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(1);
    // ponytail: one queued frame; drop the rest so a slow ffplay cannot stall capture
    std::thread::spawn(move || {
        while let Ok(frame) = rx.recv() {
            if stdin.write_all(&frame).is_err() {
                break;
            }
        }
    });
    Ok((Some(tx), Some(child)))
}

fn raw_preview_args(width: u32, height: u32, framerate: u32) -> Vec<String> {
    [
        "-hide_banner",
        "-loglevel",
        "warning",
        "-fflags",
        "nobuffer",
        "-flags",
        "low_delay",
        "-framedrop",
        "-window_title",
        "raw",
        "-left",
        "0",
        "-top",
        "0",
        "-f",
        "rawvideo",
        "-pixel_format",
        "yuyv422",
        "-video_size",
        &format!("{width}x{height}"),
        "-framerate",
        &framerate.to_string(),
        "-i",
        "pipe:0",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn start_preview(
    path: &Path,
    width: u32,
    height: u32,
    framerate: u32,
    left: u32,
) -> io::Result<Child> {
    ffplay(
        [
            "-hide_banner",
            "-loglevel",
            "warning",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-framedrop",
            "-window_title",
            "vcam",
            "-left",
            &left.to_string(),
            "-top",
            "0",
            "-f",
            "v4l2",
            "-input_format",
            "yuyv422",
            "-framerate",
            &framerate.to_string(),
            "-video_size",
            &format!("{width}x{height}"),
            "-i",
            &path.display().to_string(),
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        false,
    )
}

fn ffplay(args: Vec<String>, piped: bool) -> io::Result<Child> {
    let mut command = Command::new("ffplay");
    command.args(&args);
    if piped {
        command.stdin(Stdio::piped());
    }
    command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to start ffplay preview (install ffmpeg): {error}"),
        )
    })
}

fn apply_rotation(
    source: &[u8],
    width: u32,
    height: u32,
    rotation: Rotation,
    output: &mut Vec<u8>,
    pixels: &mut Vec<[u8; 3]>,
) {
    let (out_width, out_height) =
        if matches!(rotation, Rotation::Clockwise | Rotation::CounterClockwise) {
            (height as usize, width as usize)
        } else {
            (width as usize, height as usize)
        };
    pixels.clear();
    pixels.resize(out_width * out_height, [0; 3]);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let pair = y * width as usize * 2 + (x / 2) * 4;
            let (dx, dy) = match rotation {
                Rotation::None => (x, y),
                Rotation::Clockwise => (height as usize - 1 - y, x),
                Rotation::CounterClockwise => (y, width as usize - 1 - x),
                Rotation::HalfTurn => (width as usize - 1 - x, height as usize - 1 - y),
                Rotation::HorizontalFlip => (width as usize - 1 - x, y),
                Rotation::VerticalFlip => (x, height as usize - 1 - y),
            };
            pixels[dy * out_width + dx] = [
                source[pair + if x % 2 == 0 { 0 } else { 2 }],
                source[pair + 1],
                source[pair + 3],
            ];
        }
    }

    output.clear();
    output.reserve(out_width * out_height * 2);
    for pair in pixels.chunks_exact(2) {
        output.extend_from_slice(&[
            pair[0][0],
            ((pair[0][1] as u16 + pair[1][1] as u16) / 2) as u8,
            pair[1][0],
            ((pair[0][2] as u16 + pair[1][2] as u16) / 2) as u8,
        ]);
    }
}

fn configure_source(device: &Device, width: u32, height: u32) -> io::Result<Format> {
    let formats = Capture::enum_formats(device)?;
    let requested = if formats
        .iter()
        .any(|format| format.fourcc == FourCC::new(b"MJPG"))
    {
        FourCC::new(b"MJPG")
    } else if formats
        .iter()
        .any(|format| format.fourcc == FourCC::new(b"YUYV"))
    {
        FourCC::new(b"YUYV")
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "camera supports neither MJPG nor YUYV",
        ));
    };
    Capture::set_format(device, &Format::new(width, height, requested))
}

fn jpeg_to_yuyv(input: &[u8], output: &mut [u8], width: u32, height: u32) -> io::Result<()> {
    let mut decoder = Decoder::new(Cursor::new(input));
    let rgb = decoder.decode().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("JPEG decode failed: {error}"),
        )
    })?;
    let info = decoder.info().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "JPEG has no image information")
    })?;
    if info.pixel_format != PixelFormat::RGB24
        || info.width as u32 != width
        || info.height as u32 != height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unexpected JPEG frame {}x{} {:?}",
                info.width, info.height, info.pixel_format
            ),
        ));
    }

    for (pair, pixels) in rgb.chunks_exact(6).zip(output.chunks_exact_mut(4)) {
        let (y0, u0, v0) = rgb_to_yuv(pair[0], pair[1], pair[2]);
        let (y1, u1, v1) = rgb_to_yuv(pair[3], pair[4], pair[5]);
        pixels[0] = y0;
        pixels[1] = ((u0 as u16 + u1 as u16) / 2) as u8;
        pixels[2] = y1;
        pixels[3] = ((v0 as u16 + v1 as u16) / 2) as u8;
    }
    Ok(())
}

fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;
    (
        (16 + (66 * r + 129 * g + 25 * b + 128) / 256).clamp(0, 255) as u8,
        (128 + (-38 * r - 74 * g + 112 * b + 128) / 256).clamp(0, 255) as u8,
        (128 + (112 * r - 94 * g - 18 * b + 128) / 256).clamp(0, 255) as u8,
    )
}

fn beautify_yuyv(frame: &mut [u8], width: u32, height: u32, strength: f32) {
    if strength == 0.0 {
        return;
    }
    let source = frame.to_vec();
    let width = width as usize;
    let height = height as usize;
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            let mut count = 0u32;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    sum += source[(ny * width + nx) * 2] as u32;
                    count += 1;
                }
            }
            let index = (y * width + x) * 2;
            let original = source[index] as f32;
            let average = sum as f32 / count as f32;
            frame[index] = (original + (average - original) * strength)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_rotation, beautify_yuyv, parse_config, parse_video_filter, rgb_to_yuv, Rotation,
    };

    #[test]
    fn rgb_conversion_has_expected_black_and_white_values() {
        assert_eq!(rgb_to_yuv(0, 0, 0), (16, 128, 128));
        assert_eq!(rgb_to_yuv(255, 255, 255), (235, 128, 128));
    }

    #[test]
    fn no_preview_option_disables_ffplay() {
        let config = parse_config(["--no-preview".to_string()].into_iter()).unwrap();
        assert!(!config.preview);
    }

    #[test]
    fn compare_opens_side_by_side_preview() {
        let config = parse_config(["--compare".to_string()].into_iter()).unwrap();
        assert!(config.compare);
        assert!(config.preview);
        let args = super::raw_preview_args(1280, 720, 30);
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-window_title" && pair[1] == "raw"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-left" && pair[1] == "0"));
    }

    #[test]
    fn filters_parse_and_reject_unhandled_graphs() {
        assert_eq!(
            parse_video_filter("transpose=1").unwrap(),
            Rotation::Clockwise
        );
        assert_eq!(parse_video_filter("vflip").unwrap(), Rotation::VerticalFlip);
        assert!(parse_video_filter("transpose=1,hflip").is_err());
        let config = parse_config(
            ["-framerate", "30", "-vf", "transpose=1"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(config.framerate, 30);
        assert_eq!(config.rotation, Rotation::Clockwise);
    }

    #[test]
    fn clockwise_rotation_reorders_yuyv_pixels() {
        let source = [10, 128, 20, 128, 30, 128, 40, 128];
        let mut output = Vec::new();
        let mut pixels = Vec::new();
        apply_rotation(&source, 2, 2, Rotation::Clockwise, &mut output, &mut pixels);
        assert_eq!(output, [30, 128, 10, 128, 40, 128, 20, 128]);
    }

    #[test]
    fn strength_zero_leaves_frame_unchanged() {
        let mut frame = vec![10, 20, 100, 30, 200, 40, 40, 50];
        let original = frame.clone();
        beautify_yuyv(&mut frame, 2, 2, 0.0);
        assert_eq!(frame, original);
    }

    #[test]
    fn truncated_jpeg_is_invalid_data() {
        let error = super::jpeg_to_yuyv(&[0xFF, 0xD8, 0xFF], &mut [0; 8], 2, 2).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
