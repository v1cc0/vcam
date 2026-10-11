use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui_kit::component::{
    checkbox::Checkbox,
    slider::{Slider, SliderEvent, SliderState},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, px, rgb, size, Context, Entity, ImageSource, ObjectFit, Render, RenderImage,
    SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use image::RgbaImage;
use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::Device;

const POLL_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone)]
struct CapturedFrame {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

struct SharedPreview {
    latest: Option<CapturedFrame>,
    status: String,
    captured_frames: u64,
    stop: bool,
}

pub(crate) fn run(config: super::Config) -> io::Result<()> {
    let source_path = super::select_source(config.source.as_deref(), Path::new(""))?;
    let width = config.width;
    let height = config.height;
    let framerate = config.framerate;
    let title = format!("vcam Preview - {}", source_path.display());

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.activate(true);

            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from(title.clone())),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                app_id: Some("com.v1cc0.vcam".to_string()),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| PreviewView::new(source_path, width, height, framerate, window, cx))
            })
            .expect("failed to open vcam preview window");
        });

    Ok(())
}

struct PreviewView {
    source: PathBuf,
    requested_width: u32,
    requested_height: u32,
    framerate: u32,
    original_image: Option<Arc<RenderImage>>,
    processed_image: Option<Arc<RenderImage>>,
    last_frame: Option<CapturedFrame>,
    pending_frame: Option<CapturedFrame>,
    brightness: Entity<SliderState>,
    contrast: Entity<SliderState>,
    flip_horizontal: bool,
    effects_dirty: bool,
    _subscriptions: Vec<Subscription>,
    status: String,
    captured_frames: u64,
    shared: Arc<Mutex<SharedPreview>>,
    _poll_task: Task<()>,
}

impl PreviewView {
    fn new(
        source: PathBuf,
        requested_width: u32,
        requested_height: u32,
        framerate: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let brightness = cx.new(|_| {
            SliderState::new()
                .min(-1.)
                .max(1.)
                .step(0.05)
                .default_value(0.)
        });
        let contrast = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(2.)
                .step(0.05)
                .default_value(1.)
        });
        let subscriptions = vec![
            cx.subscribe_in(&brightness, window, |this, _, _: &SliderEvent, _, cx| {
                this.effects_dirty = true;
                cx.notify();
            }),
            cx.subscribe_in(&contrast, window, |this, _, _: &SliderEvent, _, cx| {
                this.effects_dirty = true;
                cx.notify();
            }),
        ];
        let shared = Arc::new(Mutex::new(SharedPreview {
            latest: None,
            status: format!("Opening {}", source.display()),
            captured_frames: 0,
            stop: false,
        }));
        let capture_shared = Arc::clone(&shared);
        let capture_source = source.clone();
        thread::spawn(move || {
            let result = capture_loop(
                capture_source,
                requested_width,
                requested_height,
                framerate,
                capture_shared.clone(),
            );
            if let Err(error) = result {
                set_status(&capture_shared, format!("Camera stopped: {error}"));
            }
        });

        let poll_shared = Arc::clone(&shared);
        let poll_task = cx.spawn(async move |this, cx| loop {
            let update = {
                let mut shared = poll_shared.lock().expect("preview state poisoned");
                (
                    shared.latest.take(),
                    shared.status.clone(),
                    shared.captured_frames,
                )
            };
            if this
                .update(cx, |view, cx| {
                    view.status = update.1;
                    view.captured_frames = update.2;
                    if let Some(frame) = update.0 {
                        view.pending_frame = Some(frame);
                    }
                    cx.notify();
                })
                .is_err()
            {
                break;
            }

            let delay = cx
                .background_executor()
                .spawn(async { thread::sleep(POLL_INTERVAL) });
            delay.await;
        });

        Self {
            source,
            requested_width,
            requested_height,
            framerate,
            original_image: None,
            processed_image: None,
            last_frame: None,
            pending_frame: None,
            brightness,
            contrast,
            flip_horizontal: false,
            effects_dirty: false,
            _subscriptions: subscriptions,
            status: "等待摄像头帧".to_string(),
            captured_frames: 0,
            shared,
            _poll_task: poll_task,
        }
    }
}

impl Drop for PreviewView {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.stop = true;
        }
    }
}

impl Render for PreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let brightness = self.brightness.read(cx).value().start();
        let contrast = self.contrast.read(cx).value().start();
        if let Some(frame) = self.pending_frame.take() {
            if let Some(previous) = self.original_image.take() {
                let _ = window.drop_image(previous);
            }
            if let Some(previous) = self.processed_image.take() {
                let _ = window.drop_image(previous);
            }
            self.last_frame = Some(frame);
            if let Some(frame) = self.last_frame.as_ref() {
                self.original_image = Some(render_image(frame.clone()));
                self.processed_image = Some(render_image(apply_effects(
                    frame,
                    brightness,
                    contrast,
                    self.flip_horizontal,
                )));
            }
            self.effects_dirty = false;
        } else if self.effects_dirty {
            if let Some(frame) = self.last_frame.as_ref() {
                if let Some(previous) = self.processed_image.take() {
                    let _ = window.drop_image(previous);
                }
                self.processed_image = Some(render_image(apply_effects(
                    frame,
                    brightness,
                    contrast,
                    self.flip_horizontal,
                )));
            }
            self.effects_dirty = false;
        }

        let original = image_element(
            self.original_image.as_ref(),
            "Waiting for camera frame",
            &self.status,
        );
        let processed = image_element(
            self.processed_image.as_ref(),
            "Waiting for processed frame",
            &self.status,
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x11161d))
            .text_color(rgb(0xe2e8f0))
            .child(
                div()
                    .h(px(56.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0x2b3542))
                    .child(
                        div().flex().flex_col().gap_1().child("vcam preview").child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x94a3b8))
                                .child(self.source.display().to_string()),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_sm()
                            .text_color(rgb(0x94a3b8))
                            .child(format!(
                                "{}x{} @ {} FPS",
                                self.requested_width, self.requested_height, self.framerate
                            ))
                            .child(format!("{} frames", self.captured_frames)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .p_3()
                    .child(side_panel())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(preview_pane(
                                format!("Original {}", self.source.display()),
                                original,
                            ))
                            .child(preview_pane("After effects".to_string(), processed))
                            .child(
                                div()
                                    .h(px(30.))
                                    .flex_shrink_0()
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(rgb(0x94a3b8))
                                    .child(self.status.clone())
                                    .child("Live capture · processed preview below"),
                            ),
                    )
                    .child(control_panel(self, cx)),
            )
            .child(
                div()
                    .h(px(34.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(0x2b3542))
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child("AMCap-style camera monitor")
                    .child("Close the window to stop capture"),
            )
    }
}

fn side_panel() -> impl IntoElement {
    div()
        .w(px(220.))
        .flex()
        .flex_col()
        .gap_3()
        .bg(rgb(0x171e27))
        .border_1()
        .border_color(rgb(0x2b3542))
        .p_3()
        .child(section_title("Device"))
        .child(info_row("Input", "V4L2 camera"))
        .child(info_row("Format", "MJPG / YUYV"))
        .child(info_row("Mode", "Live preview"))
        .child(section_title("Capture"))
        .child(info_row("Exposure", "Auto"))
        .child(info_row("Gain", "Auto"))
        .child(info_row("Output", "Not connected"))
}

fn control_panel(view: &PreviewView, cx: &mut Context<PreviewView>) -> impl IntoElement {
    let brightness = view.brightness.read(cx).value().start();
    let contrast = view.contrast.read(cx).value().start();

    div()
        .w(px(240.))
        .flex_shrink_0()
        .min_h_0()
        .flex()
        .flex_col()
        .gap_3()
        .bg(rgb(0x171e27))
        .border_1()
        .border_color(rgb(0x2b3542))
        .p_3()
        .child(section_title("Effects"))
        .child(effect_slider(
            "Brightness",
            format!("{brightness:+.2}"),
            Slider::new(&view.brightness).w(px(190.)),
        ))
        .child(effect_slider(
            "Contrast",
            format!("{contrast:.2}"),
            Slider::new(&view.contrast).w(px(190.)),
        ))
        .child(
            Checkbox::new("flip-horizontal")
                .checked(view.flip_horizontal)
                .label("Horizontal flip")
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.flip_horizontal = *checked;
                    this.effects_dirty = true;
                    cx.notify();
                })),
        )
        .child(section_title("Camera"))
        .child(info_row("Input", "V4L2 camera"))
        .child(info_row("Exposure", "Auto"))
        .child(info_row("Gain", "Auto"))
        .child(
            div()
                .mt_2()
                .text_xs()
                .text_color(rgb(0x64748b))
                .child("Controls affect the lower processed preview."),
        )
}

fn effect_slider<E: IntoElement>(
    label: &'static str,
    value: String,
    slider: E,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .child(label)
                .child(div().text_color(rgb(0x94a3b8)).child(value)),
        )
        .child(slider)
}

fn preview_pane(title: String, image: impl IntoElement) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(rgb(0x05070a))
        .border_1()
        .border_color(rgb(0x334155))
        .overflow_hidden()
        .child(
            div()
                .h(px(28.))
                .flex_shrink_0()
                .px_2()
                .flex()
                .items_center()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child(title),
        )
        .child(image)
}

fn image_element(
    image: Option<&Arc<RenderImage>>,
    waiting_label: &'static str,
    status: &str,
) -> impl IntoElement {
    match image {
        Some(image) => div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .child(
                img(ImageSource::Render(Arc::clone(image)))
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
        None => div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(rgb(0x94a3b8))
            .child(waiting_label)
            .child(status.to_string())
            .into_any_element(),
    }
}

fn section_title(title: &'static str) -> impl IntoElement {
    div().text_sm().text_color(rgb(0x38bdf8)).child(title)
}

fn info_row(label: &'static str, value: &'static str) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .text_sm()
        .child(label)
        .child(div().text_color(rgb(0x94a3b8)).child(value))
}

fn capture_loop(
    source_path: PathBuf,
    width: u32,
    height: u32,
    framerate: u32,
    shared: Arc<Mutex<SharedPreview>>,
) -> io::Result<()> {
    let source = Device::with_path(&source_path)?;
    let source_format = super::configure_source(&source, width, height)?;
    Capture::set_params(
        &source,
        &v4l::video::capture::Parameters::with_fps(framerate),
    )?;
    let mut capture = MmapStream::with_buffers(&source, Type::VideoCapture, 4)?;
    let needed = (source_format.width * source_format.height * 2) as usize;
    let mut frame = vec![0u8; needed];
    let mut bad_jpegs = 0u32;
    set_status(
        &shared,
        format!(
            "Connected: {} {}x{} @ {} FPS",
            source_format.fourcc, source_format.width, source_format.height, framerate
        ),
    );

    loop {
        if shared.lock().expect("preview state poisoned").stop {
            return Ok(());
        }
        let (input, metadata) = CaptureStream::next(&mut capture)?;
        let input = &input[..(metadata.bytesused as usize).min(input.len())];
        match source_format.fourcc.str().unwrap_or("") {
            "MJPG" | "JPEG" => {
                if let Err(error) = super::jpeg_to_yuyv(
                    input,
                    &mut frame,
                    source_format.width,
                    source_format.height,
                ) {
                    if error.kind() != io::ErrorKind::InvalidData {
                        return Err(error);
                    }
                    bad_jpegs += 1;
                    if bad_jpegs >= 30 {
                        return Err(error);
                    }
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

        let bgra = yuyv_to_bgra(&frame, source_format.width, source_format.height);
        let mut shared = shared.lock().expect("preview state poisoned");
        shared.latest = Some(CapturedFrame {
            width: source_format.width,
            height: source_format.height,
            bgra,
        });
        shared.captured_frames += 1;
    }
}

fn set_status(shared: &Arc<Mutex<SharedPreview>>, status: String) {
    shared.lock().expect("preview state poisoned").status = status;
}

fn yuyv_to_bgra(frame: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut output = Vec::with_capacity((width * height * 4) as usize);
    for pair in frame.chunks_exact(4) {
        let u = i32::from(pair[1]) - 128;
        let v = i32::from(pair[3]) - 128;
        output.extend(rgb_pixel(pair[0], u, v));
        output.extend(rgb_pixel(pair[2], u, v));
    }
    output
}

fn rgb_pixel(y: u8, u: i32, v: i32) -> [u8; 4] {
    let c = (i32::from(y) - 16).max(0);
    let r = (298 * c + 409 * v + 128) >> 8;
    let g = (298 * c - 100 * u - 208 * v + 128) >> 8;
    let b = (298 * c + 516 * u + 128) >> 8;
    [
        b.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        r.clamp(0, 255) as u8,
        255,
    ]
}

fn apply_effects(
    frame: &CapturedFrame,
    brightness: f32,
    contrast: f32,
    flip_horizontal: bool,
) -> CapturedFrame {
    let mut bgra = frame.bgra.clone();
    for pixel in bgra.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            let adjusted = (f32::from(*channel) - 128.) * contrast + 128. + brightness * 255.;
            *channel = adjusted.round().clamp(0., 255.) as u8;
        }
    }
    if flip_horizontal {
        let row_bytes = frame.width as usize * 4;
        for row in bgra.chunks_exact_mut(row_bytes) {
            for x in 0..frame.width as usize / 2 {
                let left = x * 4;
                let right = (frame.width as usize - 1 - x) * 4;
                for channel in 0..4 {
                    row.swap(left + channel, right + channel);
                }
            }
        }
    }
    CapturedFrame {
        width: frame.width,
        height: frame.height,
        bgra,
    }
}

fn render_image(frame: CapturedFrame) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(
        RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
            .expect("camera frame dimensions must match RGBA buffer"),
    )]))
}

#[cfg(test)]
mod tests {
    use super::{apply_effects, yuyv_to_bgra, CapturedFrame};

    #[test]
    fn effects_change_only_processed_frame_and_can_flip_it() {
        let frame = CapturedFrame {
            width: 2,
            height: 1,
            bgra: vec![10, 20, 30, 255, 200, 210, 220, 255],
        };

        let processed = apply_effects(&frame, 0.1, 1.0, true);

        assert_eq!(frame.bgra, vec![10, 20, 30, 255, 200, 210, 220, 255]);
        assert_eq!(processed.bgra, vec![226, 236, 246, 255, 36, 46, 56, 255]);
    }

    #[test]
    fn yuyv_preview_conversion_keeps_neutral_black_and_white() {
        let frame = [16, 128, 235, 128];
        assert_eq!(
            yuyv_to_bgra(&frame, 2, 1),
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
    }
}
