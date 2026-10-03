use std::env;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant};

use fearless_simd::{dispatch, prelude::*, Level};
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
const DEFAULT_SLIM: f32 = 0.0;

#[derive(Debug)]
struct Config {
    source: Option<PathBuf>,
    output: PathBuf,
    width: u32,
    height: u32,
    strength: f32,
    slim: f32,
    preview: bool,
    compare: bool,
    framerate: u32,
    rotation: Rotation,
    preset: Preset,
    stats: bool,
    mask: Option<PathBuf>,
    background: [u8; 3],
    light_wrap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preset {
    Natural,
    FirstTake,
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
        Some("bench") => benchmark(),
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

fn benchmark() -> io::Result<()> {
    const WIDTH: usize = 1280;
    const HEIGHT: usize = 720;
    const WARMUP: usize = 10;
    const FRAMES: usize = 120;

    let mut frame = vec![0u8; WIDTH * HEIGHT * 2];
    for (index, value) in frame.iter_mut().enumerate() {
        *value = ((index * 37 + 19) % 256) as u8;
    }
    let mut luma = Vec::with_capacity(WIDTH * HEIGHT);
    let level = Level::new();
    for _ in 0..WARMUP {
        apply_first_take(&mut frame, &mut luma, level);
    }

    let started = Instant::now();
    for _ in 0..FRAMES {
        apply_first_take(&mut frame, &mut luma, level);
        std::hint::black_box(&frame);
    }
    let average_ms = started.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64;
    println!(
        "bench: {}x{} frames={} avg_first_take_ms={average_ms:.3} simd_fallback={}",
        WIDTH,
        HEIGHT,
        FRAMES,
        level.is_fallback()
    );
    Ok(())
}

fn parse_config(mut args: impl Iterator<Item = String>) -> io::Result<Config> {
    let mut config = Config {
        source: None,
        output: PathBuf::from("/dev/video20"),
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
        strength: DEFAULT_STRENGTH,
        slim: DEFAULT_SLIM,
        preview: true,
        compare: false,
        framerate: 30,
        rotation: Rotation::None,
        preset: Preset::Natural,
        stats: false,
        mask: None,
        background: [245, 245, 245],
        light_wrap: 0.12,
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
            "--slim" => {
                config.slim = next_arg("--slim", &mut args)?.parse::<f32>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--slim must be a number")
                })?;
            }
            "--no-preview" => config.preview = false,
            "--compare" => config.compare = true,
            "--stats" => config.stats = true,
            "--mask" => config.mask = Some(PathBuf::from(next_arg("--mask", &mut args)?)),
            "--background" => {
                config.background = parse_rgb(&next_arg("--background", &mut args)?)?;
            }
            "--light-wrap" => {
                config.light_wrap = next_arg("--light-wrap", &mut args)?
                    .parse::<f32>()
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "--light-wrap must be a number")
                    })?;
            }
            "--preset" => {
                config.preset = match next_arg("--preset", &mut args)?.as_str() {
                    "natural" => Preset::Natural,
                    "first-take" => Preset::FirstTake,
                    value => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("unsupported --preset: {value}"),
                        ));
                    }
                };
            }
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
    if !(0.0..=1.0).contains(&config.strength)
        || !(0.0..=1.0).contains(&config.slim)
        || !(0.0..=1.0).contains(&config.light_wrap)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "strength and slim must be between 0 and 1",
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

fn parse_rgb(value: &str) -> io::Result<[u8; 3]> {
    let channels = value
        .split(',')
        .map(|channel| channel.trim().parse::<u8>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "--background must be R,G,B with channels from 0 to 255",
            )
        })?;
    channels.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "--background must contain exactly three channels",
        )
    })
}

fn print_help() {
    println!(
        "vcam - Rust beauty camera bridge for OBS\n\n\
         Usage:\n  vcam list\n  vcam bench\n  vcam run [options]\n\n\
         Options:\n  --source PATH       Input V4L2 camera; omitted selects the first USB capture device\n  --output PATH       v4l2loopback output (default: /dev/video20)\n  --width N           Capture width (default: 1280)\n  --height N          Capture height (default: 720)\n  --strength N        Smoothing strength, 0..1 (default: 0.35)\n  --slim N            Squeeze height. 0.33, 0.67, 1 map 720 to 600, 480, 360 (default: 0)\n  --preset NAME       natural or first-take (default: natural)\n  --mask PATH         PGM mask: white keeps the subject, black replaces the background\n  --background R,G,B  Replacement background color (default: 245,245,245)\n  --light-wrap N      Edge light wrap strength, 0..1 (default: 0.12)\n  --stats             Print rolling frame processing timings\n  --framerate N       Capture/output frame rate (default: 30)\n  --vf FILTER         transpose=1|2|clock|cclock, hflip, or vflip\n  --compare           Show the raw camera on the left and the processed output on the right\n  --no-preview        Do not launch ffplay to preview the output"
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
        "source={} output={} requested={}x{} fps={} strength={:.2} slim={:.2} preset={:?}",
        source_path.display(),
        config.output.display(),
        config.width,
        config.height,
        config.framerate,
        config.strength,
        config.slim,
        config.preset
    );

    let source = Device::with_path(&source_path)?;
    let source_format = configure_source(&source, config.width, config.height)?;
    Capture::set_params(
        &source,
        &v4l::video::capture::Parameters::with_fps(config.framerate),
    )?;
    let output = Device::with_path(&config.output)?;
    let mask = config
        .mask
        .as_deref()
        .map(|path| load_pgm_mask(path, source_format.width, source_format.height))
        .transpose()?;
    let background = rgb_to_yuv(
        config.background[0],
        config.background[1],
        config.background[2],
    );
    let squeezed = squeezed_height(source_format.height, config.slim);
    let (output_width, output_height) = if matches!(
        config.rotation,
        Rotation::Clockwise | Rotation::CounterClockwise
    ) {
        (squeezed, source_format.width)
    } else {
        (source_format.width, squeezed)
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
    let mut scratch = Vec::with_capacity((source_format.width * source_format.height * 2) as usize);
    let mut luma = Vec::with_capacity((source_format.width * source_format.height) as usize);
    let simd_level = Level::new();
    let mut stats_frames = 0u32;
    let mut stats_work = Duration::ZERO;
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

        let processing_started = if config.stats {
            Some(Instant::now())
        } else {
            None
        };
        if let Some(mask) = mask.as_deref() {
            composite_mask_yuyv(
                &mut frame,
                source_format.width,
                source_format.height,
                mask,
                background,
                config.light_wrap,
                &mut scratch,
                &mut luma,
            );
        }
        let processed_height = squeeze_yuyv(
            &mut frame,
            source_format.width,
            source_format.height,
            config.slim,
            &mut scratch,
        );
        beautify_yuyv(
            &mut frame,
            source_format.width,
            processed_height,
            config.strength,
            &mut scratch,
        );
        if config.preset == Preset::FirstTake {
            apply_first_take(&mut frame, &mut luma, simd_level);
        }
        apply_rotation(
            &frame,
            source_format.width,
            processed_height,
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

        if let Some(started) = processing_started {
            stats_frames += 1;
            stats_work += started.elapsed();
            if stats_frames == 30 {
                println!(
                    "stats: frames=30 pipeline_avg_ms={:.2}",
                    stats_work.as_secs_f64() * 1000.0 / f64::from(stats_frames)
                );
                stats_frames = 0;
                stats_work = Duration::ZERO;
            }
        }

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

fn squeezed_height(height: u32, slim: f32) -> u32 {
    if slim == 0.0 {
        return height;
    }
    // ponytail: 0.33/0.67/1 are the 600/480/360 steps on a 720 frame. Other values stay linear.
    let scaled = match (slim * 100.0).round() as i32 {
        0 => height,
        33 => height * 5 / 6,
        67 => height * 2 / 3,
        100 => height / 2,
        _ => (height as f32 * (1.0 - 0.5 * slim)).round() as u32,
    };
    let even = if scaled % 2 == 0 { scaled } else { scaled + 1 };
    even.clamp(2, height)
}

fn squeeze_yuyv(
    frame: &mut Vec<u8>,
    width: u32,
    height: u32,
    slim: f32,
    scratch: &mut Vec<u8>,
) -> u32 {
    let new_height = squeezed_height(height, slim);
    if new_height == height {
        return height;
    }
    scratch.clear();
    scratch.extend_from_slice(frame);
    let src = scratch.as_slice();
    let width = width as usize;
    let old_height = height as usize;
    let new_height_us = new_height as usize;
    frame.clear();
    frame.resize(width * new_height_us * 2, 0);
    for y in 0..new_height_us {
        let sy = y as f32 * (old_height - 1) as f32 / (new_height_us - 1) as f32;
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(old_height - 1);
        let fy = sy - y0 as f32;
        for x in (0..width).step_by(2) {
            let left = lerp_pixel(
                yuyv_pixel(&src, width, x, y0),
                yuyv_pixel(&src, width, x, y1),
                fy,
            );
            let right = lerp_pixel(
                yuyv_pixel(&src, width, x + 1, y0),
                yuyv_pixel(&src, width, x + 1, y1),
                fy,
            );
            let pair = (y * width + x) * 2;
            frame[pair] = left[0];
            frame[pair + 1] = ((left[1] as u16 + right[1] as u16) / 2) as u8;
            frame[pair + 2] = right[0];
            frame[pair + 3] = ((left[2] as u16 + right[2] as u16) / 2) as u8;
        }
    }
    new_height
}

fn lerp_pixel(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let mut out = [0u8; 3];
    for channel in 0..3 {
        out[channel] = (a[channel] as f32 * (1.0 - t) + b[channel] as f32 * t).round() as u8;
    }
    out
}

fn yuyv_pixel(src: &[u8], width: usize, x: usize, y: usize) -> [u8; 3] {
    let pair = y * width * 2 + (x / 2) * 4;
    [
        src[pair + if x % 2 == 0 { 0 } else { 2 }],
        src[pair + 1],
        src[pair + 3],
    ]
}

fn load_pgm_mask(path: &Path, width: u32, height: u32) -> io::Result<Vec<u8>> {
    let data = fs::read(path)?;
    parse_pgm_mask(&data, width, height).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("invalid mask {}: {error}", path.display()),
        )
    })
}

fn parse_pgm_mask(data: &[u8], width: u32, height: u32) -> io::Result<Vec<u8>> {
    let mut cursor = 0;
    let magic = pgm_token(data, &mut cursor).ok_or_else(|| invalid_mask("missing PGM magic"))?;
    let expected_magic = if magic == b"P5" {
        5
    } else if magic == b"P2" {
        2
    } else {
        return Err(invalid_mask("only P2 and P5 PGM masks are supported"));
    };
    let parsed_width = parse_pgm_usize(&pgm_token(data, &mut cursor), "width")?;
    let parsed_height = parse_pgm_usize(&pgm_token(data, &mut cursor), "height")?;
    let max_value = parse_pgm_usize(&pgm_token(data, &mut cursor), "maximum value")?;
    let expected_len = width as usize * height as usize;
    if parsed_width != width as usize || parsed_height != height as usize || max_value == 0 {
        return Err(invalid_mask(
            "PGM dimensions or maximum value do not match the video",
        ));
    }

    let mut mask = Vec::with_capacity(expected_len);
    if expected_magic == 5 {
        if max_value > 255 {
            return Err(invalid_mask("P5 maximum value must be at most 255"));
        }
        if data.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
            if data.get(cursor.wrapping_sub(1)) == Some(&b'\r') && data.get(cursor) == Some(&b'\n')
            {
                cursor += 1;
            }
        }
        let payload = data.get(cursor..).unwrap_or_default();
        if payload.len() < expected_len {
            return Err(invalid_mask("P5 payload is shorter than the video frame"));
        }
        mask.extend_from_slice(&payload[..expected_len]);
    } else {
        for _ in 0..expected_len {
            let token = pgm_token(data, &mut cursor)
                .ok_or_else(|| invalid_mask("P2 payload is shorter than the video frame"))?;
            let value = parse_pgm_usize(&Some(token), "pixel")?;
            if value > max_value {
                return Err(invalid_mask("P2 pixel exceeds the maximum value"));
            }
            mask.push((value * 255 / max_value) as u8);
        }
    }
    Ok(mask)
}

fn pgm_token(data: &[u8], cursor: &mut usize) -> Option<Vec<u8>> {
    loop {
        while data.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
        if data.get(*cursor) != Some(&b'#') {
            break;
        }
        while let Some(byte) = data.get(*cursor) {
            *cursor += 1;
            if *byte == b'\n' {
                break;
            }
        }
    }
    let start = *cursor;
    while let Some(byte) = data.get(*cursor) {
        if byte.is_ascii_whitespace() {
            break;
        }
        *cursor += 1;
    }
    (start != *cursor).then(|| data[start..*cursor].to_vec())
}

fn parse_pgm_usize(token: &Option<Vec<u8>>, name: &str) -> io::Result<usize> {
    let token = token
        .as_deref()
        .ok_or_else(|| invalid_mask(&format!("missing PGM {name}")))?;
    std::str::from_utf8(token)
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| invalid_mask(&format!("invalid PGM {name}")))
}

fn invalid_mask(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn composite_mask_yuyv(
    frame: &mut [u8],
    width: u32,
    height: u32,
    mask: &[u8],
    background: (u8, u8, u8),
    light_wrap: f32,
    scratch: &mut Vec<u8>,
    mask_scratch: &mut Vec<u8>,
) {
    let width = width as usize;
    let height = height as usize;
    debug_assert_eq!(mask.len(), width * height);
    scratch.clear();
    scratch.extend_from_slice(frame);
    mask_scratch.clear();
    mask_scratch.resize(width * height, 0);
    let (background_y, background_u, background_v) = background;

    // Separable integer blur: six mask reads per pixel instead of rebuilding a 3x3
    // neighborhood for every output pixel.
    for y in 0..height {
        for x in 0..width {
            let left = x.saturating_sub(1);
            let right = (x + 1).min(width - 1);
            mask_scratch[y * width + x] = ((u16::from(mask[y * width + left])
                + u16::from(mask[y * width + x])
                + u16::from(mask[y * width + right]))
                / 3) as u8;
        }
    }

    for y in 0..height {
        for x in (0..width).step_by(2) {
            let left = mask_alpha(mask, mask_scratch, width, height, x, y);
            let right = mask_alpha(mask, mask_scratch, width, height, x + 1, y);
            let left_y =
                composite_luma(scratch[(y * width + x) * 2], background_y, left, light_wrap);
            let right_y = composite_luma(
                scratch[(y * width + x + 1) * 2],
                background_y,
                right,
                light_wrap,
            );
            let pair = (y * width + x) * 2;
            frame[pair] = left_y;
            frame[pair + 2] = right_y;
            let alpha = (left.alpha + right.alpha) * 0.5;
            frame[pair + 1] = blend_u8(scratch[pair + 1], background_u, 1.0 - alpha);
            frame[pair + 3] = blend_u8(scratch[pair + 3], background_v, 1.0 - alpha);
        }
    }
}

#[derive(Clone, Copy)]
struct MaskSample {
    alpha: f32,
    center: f32,
}

fn mask_alpha(
    mask: &[u8],
    horizontal: &[u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
) -> MaskSample {
    let x = x.min(width - 1);
    let center = f32::from(mask[y * width + x]) / 255.0;
    let top = y.saturating_sub(1) * width + x;
    let middle = y * width + x;
    let bottom = (y + 1).min(height - 1) * width + x;
    let blurred = (u16::from(horizontal[top])
        + u16::from(horizontal[middle])
        + u16::from(horizontal[bottom]))
        / 3;
    MaskSample {
        alpha: f32::from(blurred) / 255.0,
        center,
    }
}

fn composite_luma(value: u8, background: u8, sample: MaskSample, light_wrap: f32) -> u8 {
    let base = blend_u8(value, background, 1.0 - sample.alpha);
    let edge =
        (4.0 * sample.alpha * (1.0 - sample.alpha) * sample.center * light_wrap).clamp(0.0, 1.0);
    blend_u8(base, background, edge)
}

fn blend_u8(foreground: u8, background: u8, background_weight: f32) -> u8 {
    (f32::from(foreground) * (1.0 - background_weight) + f32::from(background) * background_weight)
        .round()
        .clamp(0.0, 255.0) as u8
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

fn beautify_yuyv(frame: &mut [u8], width: u32, height: u32, strength: f32, source: &mut Vec<u8>) {
    if strength == 0.0 {
        return;
    }
    source.clear();
    source.extend_from_slice(frame);
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

fn apply_first_take(frame: &mut [u8], luma: &mut Vec<u8>, level: Level) {
    luma.clear();
    luma.reserve(frame.len() / 2);
    for pair in frame.chunks_exact(2) {
        luma.push(pair[0]);
    }
    dispatch!(level, simd => first_take_luma(simd, luma));

    for (pair, values) in frame.chunks_exact_mut(4).zip(luma.chunks_exact(2)) {
        pair[0] = values[0];
        pair[2] = values[1];
        pair[1] = soften_chroma(pair[1], -2);
        pair[3] = soften_chroma(pair[3], 2);
    }
}

#[inline(always)]
fn first_take_luma<S: Simd>(simd: S, values: &mut [u8]) {
    let lanes = S::u8s::LEN;
    let (vectors, remainder) = values.split_at_mut(values.len() / lanes * lanes);
    let scale = S::u16s::splat(simd, 7);
    let lift = S::u16s::splat(simd, 24);

    for chunk in vectors.chunks_exact_mut(lanes) {
        let input = S::u8s::from_slice(simd, chunk);
        let (low, high) = input.widen();
        let low = ((low * scale) >> 3) + lift;
        let high = ((high * scale) >> 3) + lift;
        low.saturating_narrow(high).store_slice(chunk);
    }
    for value in remainder {
        *value = first_take_luma_scalar(*value);
    }
}

#[inline]
fn first_take_luma_scalar(value: u8) -> u8 {
    (24 + u16::from(value) * 7 / 8).min(u16::from(u8::MAX)) as u8
}

fn soften_chroma(value: u8, warmth: i16) -> u8 {
    let centered = i16::from(value) - 128;
    (128 + centered * 7 / 8 + warmth).clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::{
        apply_first_take, apply_rotation, beautify_yuyv, composite_mask_yuyv, parse_config,
        parse_pgm_mask, parse_video_filter, rgb_to_yuv, Level, Preset, Rotation,
    };

    #[test]
    fn pgm_masks_parse_ascii_and_binary_payloads() {
        let ascii = b"P2\n# subject mask\n2 2\n255\n0 255 255 0\n";
        assert_eq!(parse_pgm_mask(ascii, 2, 2).unwrap(), [0, 255, 255, 0]);

        let binary = b"P5\n2 2\n255\n\x00\xff\xff\x00";
        assert_eq!(parse_pgm_mask(binary, 2, 2).unwrap(), [0, 255, 255, 0]);
    }

    #[test]
    fn black_mask_replaces_frame_with_background() {
        let mut frame = vec![40, 100, 80, 120, 60, 140, 100, 160];
        let mut scratch = Vec::new();
        let mut mask_scratch = Vec::new();
        composite_mask_yuyv(
            &mut frame,
            2,
            2,
            &[0, 0, 0, 0],
            (235, 128, 128),
            0.12,
            &mut scratch,
            &mut mask_scratch,
        );
        assert_eq!(frame, [235, 128, 235, 128, 235, 128, 235, 128]);
    }

    #[test]
    fn rgb_conversion_has_expected_black_and_white_values() {
        assert_eq!(rgb_to_yuv(0, 0, 0), (16, 128, 128));
        assert_eq!(rgb_to_yuv(255, 255, 255), (235, 128, 128));
    }

    #[test]
    fn first_take_preset_lifts_and_softens_test_frame() {
        let mut frame = vec![0, 128, 255, 128, 128, 128, 16, 128];
        let mut luma = Vec::new();
        apply_first_take(&mut frame, &mut luma, Level::new());
        assert_eq!(frame, [24, 126, 247, 130, 136, 126, 38, 130]);
    }

    #[test]
    fn first_take_options_parse_without_changing_default() {
        assert_eq!(
            parse_config(
                ["--preset", "first-take", "--stats"]
                    .map(str::to_string)
                    .into_iter()
            )
            .unwrap()
            .preset,
            Preset::FirstTake
        );
        assert!(
            parse_config(["--stats"].map(str::to_string).into_iter())
                .unwrap()
                .stats
        );
        assert_eq!(
            parse_config(std::iter::empty::<String>()).unwrap().preset,
            Preset::Natural
        );
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
        let mut scratch = Vec::new();
        beautify_yuyv(&mut frame, 2, 2, 0.0, &mut scratch);
        assert_eq!(frame, original);
    }

    #[test]
    fn squeeze_maps_slim_onto_half_height() {
        assert_eq!(super::squeezed_height(720, 0.0), 720);
        assert_eq!(super::squeezed_height(720, 0.33), 600);
        assert_eq!(super::squeezed_height(720, 0.67), 480);
        assert_eq!(super::squeezed_height(720, 1.0), 360);

        let mut frame = vec![0u8; 2 * 4 * 2];
        for y in 0..4usize {
            frame[y * 4] = (y * 40) as u8;
            frame[y * 4 + 2] = (y * 40) as u8;
            frame[y * 4 + 1] = 128;
            frame[y * 4 + 3] = 128;
        }
        let mut scratch = Vec::new();
        let new_height = super::squeeze_yuyv(&mut frame, 2, 4, 1.0, &mut scratch);
        assert_eq!(new_height, 2);
        assert_eq!(frame.len(), 8);
        assert_eq!(frame[0], 0);
        assert_eq!(frame[4], 120);
    }

    #[test]
    fn truncated_jpeg_is_invalid_data() {
        let error = super::jpeg_to_yuyv(&[0xFF, 0xD8, 0xFF], &mut [0; 8], 2, 2).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
