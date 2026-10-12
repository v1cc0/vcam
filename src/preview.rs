use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui_kit::base::{Button, Disableable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{
    accordion::Accordion,
    checkbox::Checkbox,
    searchable_list::SearchableVec,
    select::{Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    IndexPath,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, hsla, img, px, rgb, size, Context, Entity, ImageSource, ObjectFit, Render, RenderImage,
    SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use image::RgbaImage;
use v4l::buffer::Type;
use v4l::control::{Control, Flags as ControlFlags, Value as ControlValue};
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::Device;

const POLL_INTERVAL: Duration = Duration::from_millis(33);
type EffectOptions = SearchableVec<&'static str>;
const CAMERA_CONTROL_NAMES: &[&str] = &[
    "brightness",
    "contrast",
    "saturation",
    "hue",
    "white_balance_automatic",
    "gamma",
    "gain",
    "power_line_frequency",
    "white_balance_temperature",
    "sharpness",
    "backlight_compensation",
    "auto_exposure",
    "exposure_time_absolute",
];

#[derive(Clone)]
struct ControlSpec {
    id: u32,
    minimum: i64,
    maximum: i64,
    step: u64,
    value: i64,
    default: i64,
    inactive: bool,
}

struct CameraControls {
    specs: HashMap<String, ControlSpec>,
    error: Option<String>,
}

impl CameraControls {
    fn spec(&self, name: &str) -> Option<&ControlSpec> {
        self.specs.get(name)
    }

    fn value(&self, name: &str, fallback: i64) -> i64 {
        self.spec(name).map_or(fallback, |spec| spec.value)
    }
}

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
    let rotation = config.rotation;
    let title = format!("vcam Preview - {}", source_path.display());

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
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
                cx.new(|cx| {
                    PreviewView::new(source_path, width, height, framerate, rotation, window, cx)
                })
            })
            .expect("failed to open vcam preview window");
        });

    Ok(())
}

fn configure_theme(cx: &mut gpui_kit::App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        let black = hsla(0., 0., 0., 1.);
        let white = hsla(0., 0., 1., 1.);

        theme.colors.background = black;
        theme.colors.foreground = white;
        theme.colors.accordion = black;
        theme.colors.border = white;
        theme.colors.input = black;
        theme.colors.muted = black;
        theme.colors.muted_foreground = white;
        theme.colors.popover = black;
        theme.colors.popover_foreground = white;
        theme.colors.list = black;
        theme.colors.list_active = white;
        theme.colors.list_active_border = white;
        theme.colors.list_even = black;
        theme.colors.list_head = black;
        theme.colors.list_hover = black;
        theme.colors.primary = white;
        theme.colors.primary_active = white;
        theme.colors.primary_foreground = black;
        theme.colors.primary_hover = white;
        theme.colors.accent = white;
        theme.colors.accent_foreground = black;
        theme.colors.button = black;
        theme.colors.button_active = white;
        theme.colors.button_foreground = white;
        theme.colors.button_hover = black;
        theme.colors.ring = white;
        theme.colors.scrollbar = black;
        theme.colors.scrollbar_thumb = white;
        theme.colors.scrollbar_thumb_hover = white;
    });
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
    camera_brightness: Entity<SliderState>,
    camera_contrast: Entity<SliderState>,
    saturation: Entity<SliderState>,
    hue: Entity<SliderState>,
    gamma: Entity<SliderState>,
    gain: Entity<SliderState>,
    power_line_frequency: Entity<SliderState>,
    white_balance_temperature: Entity<SliderState>,
    sharpness: Entity<SliderState>,
    auto_exposure: Entity<SliderState>,
    exposure_time_absolute: Entity<SliderState>,
    camera_controls: CameraControls,
    white_balance_automatic: bool,
    backlight_compensation: bool,
    control_message: Option<String>,
    rotation: super::Rotation,
    rotation_select: Entity<SelectState<EffectOptions>>,
    flip_select: Entity<SelectState<EffectOptions>>,
    camera_controls_open: bool,
    software_effects_open: bool,
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
        rotation: super::Rotation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let camera_controls = discover_camera_controls(&source);
        let brightness = software_slider(cx, -1., 1., 0., 0.05);
        let contrast = software_slider(cx, 0., 2., 1., 0.05);
        let camera_brightness = make_camera_slider(cx, &camera_controls, "brightness", 5.);
        let camera_contrast = make_camera_slider(cx, &camera_controls, "contrast", 33.);
        let saturation = make_camera_slider(cx, &camera_controls, "saturation", 48.);
        let hue = make_camera_slider(cx, &camera_controls, "hue", 0.);
        let gamma = make_camera_slider(cx, &camera_controls, "gamma", 105.);
        let gain = make_camera_slider(cx, &camera_controls, "gain", 32.);
        let power_line_frequency =
            make_camera_slider(cx, &camera_controls, "power_line_frequency", 1.);
        let white_balance_temperature =
            make_camera_slider(cx, &camera_controls, "white_balance_temperature", 4000.);
        let sharpness = make_camera_slider(cx, &camera_controls, "sharpness", 4.);
        let auto_exposure = make_camera_slider(cx, &camera_controls, "auto_exposure", 3.);
        let exposure_time_absolute =
            make_camera_slider(cx, &camera_controls, "exposure_time_absolute", 313.);
        let white_balance_automatic = camera_controls.value("white_balance_automatic", 1) != 0;
        let backlight_compensation = camera_controls.value("backlight_compensation", 0) != 0;
        let rotation_select = cx.new(|cx| {
            SelectState::new(
                rotation_options(),
                Some(IndexPath::new(rotation_index(rotation))),
                window,
                cx,
            )
        });
        let flip_select = cx.new(|cx| {
            SelectState::new(
                flip_options(),
                Some(IndexPath::new(flip_index(rotation))),
                window,
                cx,
            )
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
            cx.subscribe_in(
                &camera_brightness,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    let value = this.camera_brightness.read(cx).value().start().round() as i64;
                    this.set_camera_control("brightness", value);
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &camera_contrast,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    let value = this.camera_contrast.read(cx).value().start().round() as i64;
                    this.set_camera_control("contrast", value);
                    cx.notify();
                },
            ),
            cx.subscribe_in(&saturation, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.saturation.read(cx).value().start().round() as i64;
                this.set_camera_control("saturation", value);
                cx.notify();
            }),
            cx.subscribe_in(&hue, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.hue.read(cx).value().start().round() as i64;
                this.set_camera_control("hue", value);
                cx.notify();
            }),
            cx.subscribe_in(&gamma, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.gamma.read(cx).value().start().round() as i64;
                this.set_camera_control("gamma", value);
                cx.notify();
            }),
            cx.subscribe_in(&gain, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.gain.read(cx).value().start().round() as i64;
                this.set_camera_control("gain", value);
                cx.notify();
            }),
            cx.subscribe_in(
                &power_line_frequency,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    let value = this.power_line_frequency.read(cx).value().start().round() as i64;
                    this.set_camera_control("power_line_frequency", value);
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &white_balance_temperature,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    let value = this
                        .white_balance_temperature
                        .read(cx)
                        .value()
                        .start()
                        .round() as i64;
                    this.set_camera_control("white_balance_temperature", value);
                    cx.notify();
                },
            ),
            cx.subscribe_in(&sharpness, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.sharpness.read(cx).value().start().round() as i64;
                this.set_camera_control("sharpness", value);
                cx.notify();
            }),
            cx.subscribe_in(&auto_exposure, window, |this, _, _: &SliderEvent, _, cx| {
                let value = this.auto_exposure.read(cx).value().start().round() as i64;
                this.set_camera_control("auto_exposure", value);
                cx.notify();
            }),
            cx.subscribe_in(
                &exposure_time_absolute,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    let value = this.exposure_time_absolute.read(cx).value().start().round() as i64;
                    this.set_camera_control("exposure_time_absolute", value);
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &rotation_select,
                window,
                |this, _, event: &SelectEvent<EffectOptions>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.rotation = match *value {
                            "Clockwise" => super::Rotation::Clockwise,
                            "Counter-clockwise" => super::Rotation::CounterClockwise,
                            "Half turn" => super::Rotation::HalfTurn,
                            _ => super::Rotation::None,
                        };
                        this.flip_select.update(cx, |state, cx| {
                            state.set_selected_value(&"None", window, cx);
                        });
                        this.flip_horizontal = false;
                        this.effects_dirty = true;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &flip_select,
                window,
                |this, _, event: &SelectEvent<EffectOptions>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.rotation = match *value {
                            "Horizontal" => super::Rotation::HorizontalFlip,
                            "Vertical" => super::Rotation::VerticalFlip,
                            _ => super::Rotation::None,
                        };
                        this.rotation_select.update(cx, |state, cx| {
                            state.set_selected_value(&"None", window, cx);
                        });
                        this.flip_horizontal = false;
                        this.effects_dirty = true;
                        cx.notify();
                    }
                },
            ),
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
            camera_brightness,
            camera_contrast,
            saturation,
            hue,
            gamma,
            gain,
            power_line_frequency,
            white_balance_temperature,
            sharpness,
            auto_exposure,
            exposure_time_absolute,
            camera_controls,
            white_balance_automatic,
            backlight_compensation,
            control_message: None,
            rotation,
            rotation_select,
            flip_select,
            camera_controls_open: true,
            software_effects_open: true,
            flip_horizontal: false,
            effects_dirty: false,
            _subscriptions: subscriptions,
            status: "等待摄像头帧".to_string(),
            captured_frames: 0,
            shared,
            _poll_task: poll_task,
        }
    }

    fn reset_camera_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut failures = 0;
        let mut skipped = 0;
        let defaults: Vec<(String, i64, bool)> = CAMERA_CONTROL_NAMES
            .iter()
            .filter_map(|name| {
                self.camera_controls
                    .spec(name)
                    .map(|spec| ((*name).to_string(), spec.default, spec.inactive))
            })
            .collect();

        for (name, default, inactive) in defaults {
            if inactive {
                skipped += 1;
                continue;
            }
            let updated = if name == "white_balance_automatic" {
                self.set_boolean_control(&name, default != 0)
            } else {
                self.set_camera_control(&name, default)
            };
            if !updated {
                failures += 1;
            }
        }

        let slider_defaults = [
            (&self.camera_brightness, "brightness"),
            (&self.camera_contrast, "contrast"),
            (&self.saturation, "saturation"),
            (&self.hue, "hue"),
            (&self.gamma, "gamma"),
            (&self.gain, "gain"),
            (&self.power_line_frequency, "power_line_frequency"),
            (&self.white_balance_temperature, "white_balance_temperature"),
            (&self.sharpness, "sharpness"),
            (&self.auto_exposure, "auto_exposure"),
            (&self.exposure_time_absolute, "exposure_time_absolute"),
        ];
        for (slider, name) in slider_defaults {
            if let Some(spec) = self.camera_controls.spec(name) {
                slider.update(cx, |state, cx| {
                    state.set_value(spec.default as f32, window, cx)
                });
            }
        }
        self.white_balance_automatic = self
            .camera_controls
            .spec("white_balance_automatic")
            .is_some_and(|spec| spec.default != 0);
        self.backlight_compensation = self
            .camera_controls
            .spec("backlight_compensation")
            .is_some_and(|spec| spec.default != 0);
        self.control_message = Some(if failures == 0 && skipped == 0 {
            "Camera defaults restored".to_string()
        } else {
            format!("Camera defaults restored; {failures} failed, {skipped} inactive")
        });
    }

    fn set_camera_control(&mut self, name: &str, value: i64) -> bool {
        let Some(spec) = self.camera_controls.spec(name) else {
            return false;
        };
        let value = value.clamp(spec.minimum, spec.maximum);
        let result = Device::with_path(&self.source).and_then(|device| {
            device.set_control(Control {
                id: spec.id,
                value: ControlValue::Integer(value),
            })
        });
        match result {
            Ok(()) => {
                if let Some(spec) = self.camera_controls.specs.get_mut(name) {
                    spec.value = value;
                }
                self.control_message = Some(format!("{name} = {value}"));
                true
            }
            Err(error) => {
                self.control_message = Some(format!("{name}: {error}"));
                false
            }
        }
    }

    fn set_boolean_control(&mut self, name: &str, value: bool) -> bool {
        let Some(spec) = self.camera_controls.spec(name) else {
            return false;
        };
        let result = Device::with_path(&self.source).and_then(|device| {
            device.set_control(Control {
                id: spec.id,
                value: ControlValue::Boolean(value),
            })
        });
        match result {
            Ok(()) => {
                if let Some(spec) = self.camera_controls.specs.get_mut(name) {
                    spec.value = i64::from(value);
                }
                self.control_message = Some(format!("{name} = {value}"));
                true
            }
            Err(error) => {
                self.control_message = Some(format!("{name}: {error}"));
                false
            }
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
                    self.rotation,
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
                    self.rotation,
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
            .bg(rgb(0x000000))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .h(px(56.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0xffffff))
                    .child(
                        div().flex().flex_col().gap_1().child("vcam preview").child(
                            div()
                                .text_sm()
                                .text_color(rgb(0xffffff))
                                .child(self.source.display().to_string()),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_sm()
                            .text_color(rgb(0xffffff))
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
                    .child(side_panel(self, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(preview_pane(
                                        format!("Original {}", self.source.display()),
                                        original,
                                    ))
                                    .child(preview_pane("After effects".to_string(), processed)),
                            )
                            .child(
                                div()
                                    .h(px(30.))
                                    .flex_shrink_0()
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(rgb(0xffffff))
                                    .child(self.status.clone())
                                    .child("Live capture · processed preview on the right"),
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
                    .border_color(rgb(0xffffff))
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .child("AMCap-style camera monitor")
                    .child("Close the window to stop capture"),
            )
    }
}

fn side_panel(view: &PreviewView, cx: &mut Context<PreviewView>) -> impl IntoElement {
    div()
        .w(px(320.))
        .flex_shrink_0()
        .min_h_0()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_3()
        .bg(rgb(0x000000))
        .border_1()
        .border_color(rgb(0xffffff))
        .p_3()
        .child(section_title("Device"))
        .child(info_row("Input", "V4L2 camera"))
        .child(info_row("Format", "MJPG / YUYV"))
        .child(info_row("Mode", "Live preview"))
        .child(section_title("Capture"))
        .child(info_row("Exposure", "Auto"))
        .child(info_row("Gain", "Auto"))
        .child(info_row("Output", "Not connected"))
        .child(
            Accordion::new("camera-controls-left")
                .multiple(true)
                .bordered(false)
                .item(|item| {
                    item.title("Camera controls")
                        .open(view.camera_controls_open)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(reset_camera_button(cx))
                                .child(camera_controls(view, cx)),
                        )
                })
                .on_toggle_click(cx.listener(|this, open: &[usize], _, cx| {
                    this.camera_controls_open = open.contains(&0);
                    cx.notify();
                })),
        )
}

fn discover_camera_controls(source: &Path) -> CameraControls {
    let result = (|| -> io::Result<HashMap<String, ControlSpec>> {
        let device = Device::with_path(source)?;
        let descriptions = device.query_controls()?;
        let mut specs = HashMap::new();
        for description in descriptions {
            let name = normalize_control_name(&description.name);
            if !CAMERA_CONTROL_NAMES.contains(&name.as_str()) {
                continue;
            }
            let (minimum, maximum) =
                ordered_control_bounds(description.minimum, description.maximum);
            let value = device
                .control(description.id)
                .ok()
                .and_then(|control| match control.value {
                    ControlValue::Integer(value) => Some(value),
                    ControlValue::Boolean(value) => Some(i64::from(value)),
                    _ => None,
                })
                .unwrap_or(description.default)
                .clamp(minimum, maximum);
            let default = description.default.clamp(minimum, maximum);
            specs.insert(
                name,
                ControlSpec {
                    id: description.id,
                    minimum,
                    maximum,
                    step: description.step.max(1),
                    value,
                    default,
                    inactive: description.flags.contains(ControlFlags::INACTIVE),
                },
            );
        }
        Ok(specs)
    })();

    match result {
        Ok(specs) => CameraControls { specs, error: None },
        Err(error) => CameraControls {
            specs: HashMap::new(),
            error: Some(error.to_string()),
        },
    }
}

fn ordered_control_bounds(minimum: i64, maximum: i64) -> (i64, i64) {
    if minimum <= maximum {
        (minimum, maximum)
    } else {
        (maximum, minimum)
    }
}

fn normalize_control_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn slider_state(minimum: f32, maximum: f32, step: f32, value: f32) -> SliderState {
    let mut state = SliderState::new();
    if minimum > state.max_value() {
        state = state.max(maximum).min(minimum);
    } else if maximum < state.min_value() {
        state = state.min(minimum).max(maximum);
    } else {
        state = state.min(minimum).max(maximum);
    }
    state.step(step).default_value(value)
}

fn software_slider(
    cx: &mut Context<PreviewView>,
    minimum: f32,
    maximum: f32,
    value: f32,
    step: f32,
) -> Entity<SliderState> {
    cx.new(|_| slider_state(minimum, maximum, step, value))
}

fn make_camera_slider(
    cx: &mut Context<PreviewView>,
    controls: &CameraControls,
    name: &str,
    fallback: f32,
) -> Entity<SliderState> {
    let (minimum, maximum, step, value) =
        controls
            .spec(name)
            .map_or((fallback - 1., fallback + 1., 1., fallback), |spec| {
                let (minimum, maximum) = ordered_control_bounds(spec.minimum, spec.maximum);
                (
                    minimum as f32,
                    maximum as f32,
                    spec.step as f32,
                    (spec.value as f32).clamp(minimum as f32, maximum as f32),
                )
            });
    cx.new(|_| slider_state(minimum, maximum, step, value))
}

fn reset_camera_button(cx: &mut Context<PreviewView>) -> impl IntoElement {
    Button::new("reset-camera-defaults")
        .accessibility_label("Reset camera defaults")
        .px_2()
        .py_1()
        .border_1()
        .border_color(rgb(0xffffff))
        .text_color(rgb(0xffffff))
        .on_click(cx.listener(|this, _, window, cx| {
            this.reset_camera_defaults(window, cx);
            cx.notify();
        }))
        .child("Reset camera defaults")
}

fn control_panel(view: &PreviewView, cx: &mut Context<PreviewView>) -> impl IntoElement {
    div()
        .w(px(320.))
        .flex_shrink_0()
        .min_h_0()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .bg(rgb(0x000000))
        .border_1()
        .border_color(rgb(0xffffff))
        .p_2()
        .child(
            Accordion::new("software-effects-panel")
                .multiple(true)
                .bordered(false)
                .item(|item| {
                    item.title("Software effects")
                        .open(view.software_effects_open)
                        .child(software_effects(view, cx))
                })
                .on_toggle_click(cx.listener(|this, open: &[usize], _, cx| {
                    this.software_effects_open = open.contains(&0);
                    cx.notify();
                })),
        )
        .when_some(view.control_message.clone(), |element, message| {
            element.child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .child(message),
            )
        })
}

fn camera_controls(view: &PreviewView, cx: &mut Context<PreviewView>) -> impl IntoElement {
    let temp_disabled = view
        .camera_controls
        .spec("white_balance_temperature")
        .map_or(true, |spec| spec.inactive && view.white_balance_automatic);
    let exposure_disabled = view
        .camera_controls
        .spec("exposure_time_absolute")
        .map_or(true, |spec| {
            spec.inactive && view.auto_exposure.read(cx).value().start().round() == 3.
        });
    let controls_available = view.camera_controls.error.is_none();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(camera_slider(
            "Brightness",
            view.camera_brightness.read(cx).value().start().round() as i64,
            Slider::new(&view.camera_brightness)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Contrast",
            view.camera_contrast.read(cx).value().start().round() as i64,
            Slider::new(&view.camera_contrast)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Saturation",
            view.saturation.read(cx).value().start().round() as i64,
            Slider::new(&view.saturation)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Hue",
            view.hue.read(cx).value().start().round() as i64,
            Slider::new(&view.hue)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(
            Checkbox::new("white-balance-automatic")
                .checked(view.white_balance_automatic)
                .disabled(!controls_available)
                .label("Auto white balance")
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.white_balance_automatic = *checked;
                    this.set_boolean_control("white_balance_automatic", *checked);
                    cx.notify();
                })),
        )
        .child(camera_slider(
            "White balance temp",
            view.white_balance_temperature
                .read(cx)
                .value()
                .start()
                .round() as i64,
            Slider::new(&view.white_balance_temperature)
                .min_w_0()
                .disabled(temp_disabled),
            temp_disabled,
        ))
        .child(camera_slider(
            "Gamma",
            view.gamma.read(cx).value().start().round() as i64,
            Slider::new(&view.gamma)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Gain",
            view.gain.read(cx).value().start().round() as i64,
            Slider::new(&view.gain)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Power frequency",
            view.power_line_frequency.read(cx).value().start().round() as i64,
            Slider::new(&view.power_line_frequency)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Sharpness",
            view.sharpness.read(cx).value().start().round() as i64,
            Slider::new(&view.sharpness)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(
            Checkbox::new("backlight-compensation")
                .checked(view.backlight_compensation)
                .disabled(!controls_available)
                .label("Backlight compensation")
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.backlight_compensation = *checked;
                    this.set_camera_control("backlight_compensation", i64::from(*checked));
                    cx.notify();
                })),
        )
        .child(camera_slider(
            "Auto exposure mode",
            view.auto_exposure.read(cx).value().start().round() as i64,
            Slider::new(&view.auto_exposure)
                .min_w_0()
                .disabled(!controls_available),
            !controls_available,
        ))
        .child(camera_slider(
            "Exposure time",
            view.exposure_time_absolute.read(cx).value().start().round() as i64,
            Slider::new(&view.exposure_time_absolute)
                .min_w_0()
                .disabled(exposure_disabled),
            exposure_disabled,
        ))
        .when_some(view.camera_controls.error.clone(), |element, error| {
            element.child(
                div()
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .child(format!("V4L2 controls unavailable: {error}")),
            )
        })
}

fn rotation_options() -> EffectOptions {
    SearchableVec::new(["None", "Clockwise", "Counter-clockwise", "Half turn"])
}

fn flip_options() -> EffectOptions {
    SearchableVec::new(["None", "Horizontal", "Vertical"])
}

fn rotation_index(rotation: super::Rotation) -> usize {
    match rotation {
        super::Rotation::None | super::Rotation::HorizontalFlip | super::Rotation::VerticalFlip => {
            0
        }
        super::Rotation::Clockwise => 1,
        super::Rotation::CounterClockwise => 2,
        super::Rotation::HalfTurn => 3,
    }
}

fn flip_index(rotation: super::Rotation) -> usize {
    match rotation {
        super::Rotation::HorizontalFlip => 1,
        super::Rotation::VerticalFlip => 2,
        _ => 0,
    }
}

fn software_effects(view: &PreviewView, cx: &mut Context<PreviewView>) -> impl IntoElement {
    let brightness = view.brightness.read(cx).value().start();
    let contrast = view.contrast.read(cx).value().start();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(effect_select("Video rotation", &view.rotation_select))
        .child(effect_select("Flip", &view.flip_select))
        .child(effect_slider(
            "Brightness",
            format!("{brightness:+.2}"),
            Slider::new(&view.brightness).w(px(220.)),
        ))
        .child(effect_slider(
            "Contrast",
            format!("{contrast:.2}"),
            Slider::new(&view.contrast).w(px(220.)),
        ))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xffffff))
                .child("Only the processed preview uses software effects."),
        )
}

fn effect_select(
    label: &'static str,
    state: &Entity<SelectState<EffectOptions>>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(
            Select::new(state)
                .id(format!("effect-select-{label}"))
                .w_full()
                .h(px(32.))
                .accessibility_label(label),
        )
}

fn camera_slider<E: IntoElement>(
    label: &'static str,
    value: i64,
    slider: E,
    _disabled: bool,
) -> impl IntoElement {
    effect_slider(label, control_display(label, value), slider)
}

fn control_display(label: &str, value: i64) -> String {
    match label {
        "Power frequency" => match value {
            0 => "Disabled".to_string(),
            1 => "50 Hz".to_string(),
            2 => "60 Hz".to_string(),
            _ => value.to_string(),
        },
        "Auto exposure mode" => match value {
            0 => "Manual".to_string(),
            1 => "Auto".to_string(),
            2 => "Shutter priority".to_string(),
            3 => "Aperture priority".to_string(),
            _ => value.to_string(),
        },
        _ => value.to_string(),
    }
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
                .child(div().text_color(rgb(0xffffff)).child(value)),
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
        .bg(rgb(0x000000))
        .border_1()
        .border_color(rgb(0xffffff))
        .overflow_hidden()
        .child(
            div()
                .h(px(28.))
                .flex_shrink_0()
                .px_2()
                .flex()
                .items_center()
                .text_xs()
                .text_color(rgb(0xffffff))
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
            .text_color(rgb(0xffffff))
            .child(waiting_label)
            .child(status.to_string())
            .into_any_element(),
    }
}

fn section_title(title: &'static str) -> impl IntoElement {
    div().text_sm().text_color(rgb(0xffffff)).child(title)
}

fn info_row(label: &'static str, value: &'static str) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .text_sm()
        .child(label)
        .child(div().text_color(rgb(0xffffff)).child(value))
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
    rotation: super::Rotation,
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
    let processed = CapturedFrame {
        width: frame.width,
        height: frame.height,
        bgra,
    };
    apply_video_filter(&processed, rotation)
}

fn apply_video_filter(frame: &CapturedFrame, rotation: super::Rotation) -> CapturedFrame {
    if rotation == super::Rotation::None {
        return frame.clone();
    }
    let (width, height) = match rotation {
        super::Rotation::Clockwise | super::Rotation::CounterClockwise => {
            (frame.height, frame.width)
        }
        _ => (frame.width, frame.height),
    };
    let mut bgra = vec![0; (width * height * 4) as usize];
    for y in 0..frame.height as usize {
        for x in 0..frame.width as usize {
            let (dx, dy) = match rotation {
                super::Rotation::Clockwise => (frame.height as usize - 1 - y, x),
                super::Rotation::CounterClockwise => (y, frame.width as usize - 1 - x),
                super::Rotation::HalfTurn => {
                    (frame.width as usize - 1 - x, frame.height as usize - 1 - y)
                }
                super::Rotation::HorizontalFlip => (frame.width as usize - 1 - x, y),
                super::Rotation::VerticalFlip => (x, frame.height as usize - 1 - y),
                super::Rotation::None => (x, y),
            };
            let source = (y * frame.width as usize + x) * 4;
            let target = (dy * width as usize + dx) * 4;
            bgra[target..target + 4].copy_from_slice(&frame.bgra[source..source + 4]);
        }
    }
    CapturedFrame {
        width,
        height,
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
    use super::{apply_effects, slider_state, yuyv_to_bgra, CapturedFrame};
    use crate::Rotation;

    #[test]
    fn control_bounds_are_ordered() {
        assert_eq!(super::ordered_control_bounds(2800, 100), (100, 2800));
        assert_eq!(super::ordered_control_bounds(100, 2800), (100, 2800));
    }

    #[test]
    fn slider_state_accepts_ranges_outside_gpui_defaults() {
        let high = slider_state(2800., 6500., 1., 4000.);
        assert_eq!(high.min_value(), 2800.);
        assert_eq!(high.max_value(), 6500.);

        let low = slider_state(-6500., -2800., 1., -4000.);
        assert_eq!(low.min_value(), -6500.);
        assert_eq!(low.max_value(), -2800.);
    }

    #[test]
    fn effect_dropdown_indices_match_rotation_modes() {
        assert_eq!(super::rotation_index(Rotation::None), 0);
        assert_eq!(super::rotation_index(Rotation::Clockwise), 1);
        assert_eq!(super::rotation_index(Rotation::CounterClockwise), 2);
        assert_eq!(super::rotation_index(Rotation::HalfTurn), 3);
        assert_eq!(super::flip_index(Rotation::None), 0);
        assert_eq!(super::flip_index(Rotation::HorizontalFlip), 1);
        assert_eq!(super::flip_index(Rotation::VerticalFlip), 2);
    }

    #[test]
    fn effects_change_only_processed_frame_and_can_flip_it() {
        let frame = CapturedFrame {
            width: 2,
            height: 1,
            bgra: vec![10, 20, 30, 255, 200, 210, 220, 255],
        };

        let processed = apply_effects(&frame, 0.1, 1.0, true, Rotation::None);

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
