use std::env;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

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

    if config.width == 0 || config.height == 0 || config.width % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "width must be a positive even integer and height must be greater than zero",
        ));
    }
    if !(0.0..=1.0).contains(&config.strength) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "strength must be between 0 and 1",
        ));
    }
    Ok(config)
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
         Options:\n  --source PATH       Input V4L2 camera; omitted selects the first USB capture device\n  --output PATH       v4l2loopback output (default: /dev/video20)\n  --width N           Capture width (default: 1280)\n  --height N          Capture height (default: 720)\n  --strength N        Smoothing strength, 0..1 (default: 0.35)"
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
        "source={} output={} requested={}x{} strength={:.2}",
        source_path.display(),
        config.output.display(),
        config.width,
        config.height,
        config.strength
    );

    let source = Device::with_path(&source_path)?;
    let source_format = configure_source(&source, config.width, config.height)?;
    let output = Device::with_path(&config.output)?;
    let output_format = Output::set_format(
        &output,
        &Format::new(
            source_format.width,
            source_format.height,
            FourCC::new(b"YUYV"),
        ),
    )?;

    if output_format.fourcc != FourCC::new(b"YUYV")
        || output_format.width != source_format.width
        || output_format.height != source_format.height
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
    let mut frame = Vec::with_capacity((source_format.width * source_format.height * 2) as usize);

    loop {
        let (input, metadata) = CaptureStream::next(&mut capture)?;
        let input = &input[..(metadata.bytesused as usize).min(input.len())];
        let (output_buffer, output_metadata) = OutputStream::next(&mut output_stream)?;
        let needed = (source_format.width * source_format.height * 2) as usize;
        if output_buffer.len() < needed {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "loopback buffer is smaller than the requested frame",
            ));
        }
        frame.clear();
        frame.resize(needed, 0);

        match source_format.fourcc.str().unwrap_or("") {
            "MJPG" | "JPEG" => {
                jpeg_to_yuyv(input, &mut frame, source_format.width, source_format.height)?
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

        beautify_yuyv(
            &mut frame,
            source_format.width,
            source_format.height,
            config.strength,
        );
        output_buffer[..needed].copy_from_slice(&frame);
        output_metadata.bytesused = needed as u32;
        output_metadata.field = 0;
    }
}

fn configure_source(device: &Device, width: u32, height: u32) -> io::Result<Format> {
    let formats = device.enum_formats()?;
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
    use super::{beautify_yuyv, rgb_to_yuv};

    #[test]
    fn rgb_conversion_has_expected_black_and_white_values() {
        assert_eq!(rgb_to_yuv(0, 0, 0), (16, 128, 128));
        assert_eq!(rgb_to_yuv(255, 255, 255), (235, 128, 128));
    }

    #[test]
    fn strength_zero_leaves_frame_unchanged() {
        let mut frame = vec![10, 20, 100, 30, 200, 40, 40, 50];
        let original = frame.clone();
        beautify_yuyv(&mut frame, 2, 2, 0.0);
        assert_eq!(frame, original);
    }
}
