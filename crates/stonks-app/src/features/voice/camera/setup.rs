use super::{Command, Voice};
use crate::runtime::spawn_core;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::sync::Arc;
use stonks_core::voice::{
    camera::{
        devices, resolve_device, run_preview, CameraDevice, CameraError, CameraOptions,
        CameraQuality, CameraState,
    },
    VoicePhase,
};
use stonks_ui::{
    components::{Button, ButtonVariant, UiAccordion, UiCloseButton, UiDialog},
    i18n::t,
    icons::Icon,
    theme::Theme,
};
use tokio::sync::{oneshot, watch};

pub(in crate::features::voice) struct Setup {
    generation: u64,
    devices: Option<Result<Vec<CameraDevice>, CameraError>>,
    options: CameraOptions,
    focus: FocusHandle,
    opener: Option<FocusHandle>,
    _discovery: Task<()>,
    device_open: bool,
    device_trigger: FocusHandle,
    // A setup can outlive several capture attempts; only the newest may update it.
    preview_revision: u64,
    preview: Option<Preview>,
    frame: Option<Arc<RenderImage>>,
    preview_error: Option<CameraError>,
}

struct Preview {
    _stop: oneshot::Sender<()>,
    _task: Task<()>,
}

impl Setup {
    fn selected_device(&self) -> Option<&CameraDevice> {
        resolve_device(self.devices.as_ref()?.as_ref().ok()?, &self.options.device).ok()
    }

    fn capture_options(&self) -> Option<CameraOptions> {
        // Preview and Start both use the concrete device shown in this dialog.
        Some(CameraOptions {
            device: self.selected_device()?.id.clone(),
            quality: self.options.quality,
        })
    }

    pub(in crate::features::voice) fn stop_preview(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        self.preview_revision += 1;
        self.preview.take();
        if let Some(frame) = self.frame.take() {
            cx.drop_image(frame, window);
        }
        self.preview_error = None;
    }
}

impl Voice {
    pub(in crate::features::voice) fn toggle_camera(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.screen_setup_open() {
            return;
        }
        let Some(call) = self
            .call
            .as_mut()
            .filter(|call| !call.hidden && call.stop.is_some())
        else {
            return;
        };
        match call.camera.state {
            CameraState::Starting | CameraState::Publishing => {
                if call.camera.commands.try_send(Command::Stop).is_err() {
                    return;
                }
                call.camera.state = CameraState::Stopping;
            }
            CameraState::Idle | CameraState::Failed(_)
                if call.camera.setup.is_none()
                    && call
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.phase == VoicePhase::Connected) =>
            {
                self.camera_setup_generation += 1;
                let generation = self.camera_setup_generation;
                let focus = cx.focus_handle();
                let opener = window.focused(cx);
                focus.focus(window, cx);
                let discovery = cx.spawn_in(window, async move |this, cx| {
                    let result = spawn_core(devices()).await;
                    let _ = cx.update(|window, cx| {
                        this.update(cx, |voice, cx| {
                            if let Some(setup) = voice
                                .camera_setup_mut()
                                .filter(|setup| setup.generation == generation)
                            {
                                setup.devices = Some(result);
                                voice.restart_local_preview(window, cx);
                            }
                        })
                    });
                });
                call.camera.setup = Some(Setup {
                    generation,
                    devices: None,
                    focus,
                    opener,
                    _discovery: discovery,
                    device_open: false,
                    device_trigger: cx.focus_handle(),
                    preview_revision: 0,
                    preview: None,
                    frame: None,
                    preview_error: None,
                    options: cx
                        .global::<crate::preferences::NativePreferences>()
                        .saved
                        .camera_options(),
                });
            }
            _ => return,
        }
        cx.notify();
    }

    pub(crate) fn camera_setup_open(&self) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| !call.hidden && call.stop.is_some() && call.camera.setup.is_some())
    }

    pub(crate) fn dismiss_camera_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut setup) = self.call.as_mut().and_then(|call| call.camera.setup.take()) {
            setup.stop_preview(Some(window), cx);
            if let Some(opener) = setup.opener {
                opener.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn confirm_camera_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(call) = self.call.as_mut().filter(|call| {
            !call.hidden
                && call.stop.is_some()
                && call
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.phase == VoicePhase::Connected)
                && matches!(
                    call.camera.state,
                    CameraState::Idle | CameraState::Failed(_)
                )
        }) else {
            return;
        };
        let Some(setup) = call.camera.setup.as_mut() else {
            return;
        };
        let Some(options) = setup.capture_options() else {
            return;
        };
        if call
            .camera
            .commands
            .try_send(Command::Start(options))
            .is_ok()
        {
            call.camera.state = CameraState::Starting;
            self.dismiss_camera_setup(window, cx);
        }
    }

    fn restart_local_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(setup) = self.camera_setup_mut() else {
            return;
        };
        setup.stop_preview(Some(window), cx);
        let Some(options) = setup.capture_options() else {
            cx.notify();
            return;
        };
        let (stop, cancelled) = oneshot::channel();
        let (frames, mut latest) = watch::channel(None);
        let finished = spawn_core(run_preview(options, cancelled, frames));
        let generation = setup.generation;
        let revision = setup.preview_revision;
        let task = cx.spawn_in(window, async move |this, cx| {
            while latest.changed().await.is_ok() {
                let frame = {
                    let latest = latest.borrow_and_update();
                    latest.as_ref().and_then(|frame| {
                        image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())
                    })
                };
                let Some(frame) = frame else {
                    continue;
                };
                let frame = Arc::new(RenderImage::new(vec![image::Frame::new(frame)]));
                if cx
                    .update(|window, cx| {
                        this.update(cx, |voice, cx| {
                            let Some(setup) = voice.preview_setup(generation, revision) else {
                                return false;
                            };
                            if let Some(old) = setup.frame.replace(frame) {
                                let _ = window.drop_image(old);
                            }
                            cx.notify();
                            true
                        })
                    })
                    .ok()
                    .and_then(Result::ok)
                    != Some(true)
                {
                    return;
                }
            }
            let error = finished.await.err();
            let _ = cx.update(|window, cx| {
                this.update(cx, |voice, cx| {
                    if let Some(setup) = voice.preview_setup(generation, revision) {
                        setup.stop_preview(Some(window), cx);
                        setup.preview_error = error;
                        cx.notify();
                    }
                })
            });
        });
        setup.preview = Some(Preview {
            _stop: stop,
            _task: task,
        });
        cx.notify();
    }

    fn camera_setup_mut(&mut self) -> Option<&mut Setup> {
        self.call
            .as_mut()
            .filter(|call| !call.hidden && call.stop.is_some())?
            .camera
            .setup
            .as_mut()
    }

    fn preview_setup(&mut self, generation: u64, revision: u64) -> Option<&mut Setup> {
        self.camera_setup_mut()
            .filter(|setup| setup.generation == generation && setup.preview_revision == revision)
    }

    pub(crate) fn camera_setup_surface(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.camera_setup_open() {
            return None;
        }
        let setup = self.call.as_ref()?.camera.setup.as_ref()?;
        let theme = Theme::current(cx);
        let system_default = t("settings.voice.systemDefault");
        let selected_label = if setup.options.device == "default" {
            setup.selected_device().map_or_else(
                || system_default.clone(),
                |device| format!("{system_default} · {}", device.label),
            )
        } else if let Some(device) = setup.selected_device() {
            device.label.clone()
        } else if setup.devices.is_none() {
            t("voice.cameraDiscovering")
        } else {
            t("settings.voice.unavailable")
        };
        let mut device_picker = UiAccordion::new("camera-devices", theme, t("voice.cameraDevice"))
            .subtitle(selected_label)
            .leading(camera_icon(theme))
            .track_focus(&setup.device_trigger)
            .open(setup.device_open)
            .on_toggle(cx.listener(|this, _, _, cx| {
                if let Some(setup) = this.camera_setup_mut() {
                    setup.device_open = !setup.device_open;
                    cx.notify();
                }
            }));
        if let Some(Ok(devices)) = &setup.devices {
            device_picker = device_picker.child(
                div()
                    .id("camera-device-options")
                    .role(Role::RadioGroup)
                    .aria_label(t("voice.cameraDevice"))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .max_h(px(176.))
                    .overflow_y_scroll()
                    .children(
                        std::iter::once(("default".to_owned(), system_default, None))
                            .chain(devices.iter().map(|device| {
                                (
                                    device.id.clone(),
                                    device.label.clone(),
                                    device
                                        .is_default
                                        .then(|| t("settings.voice.currentDefault")),
                                )
                            }))
                            .map(|(id, label, description)| {
                                disclosure_choice(
                                    ElementId::Name(id.clone().into()),
                                    theme,
                                    label,
                                    description,
                                    setup.options.device == id,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        let Some(setup) = this.camera_setup_mut() else {
                                            return;
                                        };
                                        setup.device_open = false;
                                        setup.device_trigger.focus(window, cx);
                                        if setup.options.device != id {
                                            setup.options.device.clone_from(&id);
                                            this.restart_local_preview(window, cx);
                                        } else {
                                            cx.notify();
                                        }
                                    },
                                ))
                            }),
                    ),
            );
        }
        let compact = window.viewport_size().width <= px(400.);
        let quality_picker = div()
            .id("camera-quality-options")
            .role(Role::Group)
            .aria_label(t("voice.chooseCameraQuality"))
            .flex()
            .when(compact, |row| row.flex_col())
            .gap(px(theme.space2))
            .children(CameraQuality::ALL.map(|quality| {
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .when(!compact, |cell| cell.flex_1())
                    .child(
                        Button::new(("camera-quality-option", quality as usize), theme)
                            .variant(if setup.options.quality == quality {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .label(quality.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(setup) = this.camera_setup_mut() {
                                    if setup.options.quality != quality {
                                        setup.options.quality = quality;
                                        this.restart_local_preview(window, cx);
                                    }
                                }
                            })),
                    )
            }));
        let error = match &setup.devices {
            Some(Err(error)) => Some(error.to_string()),
            Some(Ok(devices)) if devices.is_empty() => Some(t("voice.cameraNone")),
            Some(Ok(_)) if setup.selected_device().is_none() => {
                Some(t("voice.cameraSelectionUnavailable"))
            }
            _ => setup.preview_error.map(|error| error.to_string()),
        };
        let preview = div()
            .flex()
            .items_center()
            .justify_center()
            .min_h(px(250.))
            .flex_shrink_0()
            .overflow_hidden()
            .rounded(px(theme.radius_lg))
            .bg(theme.canvas)
            .text_center()
            .text_size(px(theme.font_size_md))
            .when_some(setup.frame.clone(), |panel, frame| {
                panel.child(
                    img(frame)
                        .id("camera-preview-frame")
                        .w_full()
                        .h(px(250.))
                        .object_fit(ObjectFit::Contain),
                )
            })
            .when_some(error, |panel, error| {
                panel.child(
                    div()
                        .p(px(theme.space3))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .when(setup.frame.is_none() && setup.preview.is_some(), |panel| {
                panel.child(
                    div()
                        .text_color(theme.text_muted)
                        .child(t("voice.cameraPreviewOpening")),
                )
            })
            .when(setup.devices.is_none(), |panel| {
                panel.child(
                    div()
                        .text_color(theme.text_muted)
                        .child(t("voice.cameraDiscovering")),
                )
            });
        let title = t("voice.cameraSettings");
        Some(
            UiDialog::new("camera-setup", theme, title.clone())
                .width(760.)
                .viewport_inset(10.)
                .padding(theme.space5)
                .focus(&setup.focus)
                .show_close(false)
                .on_close(cx.listener(|this, _, window, cx| this.dismiss_camera_setup(window, cx)))
                .title_content(
                    div()
                        .relative()
                        .pr(px(44.))
                        .flex()
                        .flex_col()
                        .gap(px(theme.space2))
                        .font_weight(FontWeight::BOLD)
                        .line_height(relative(theme.line_height_compact))
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .font_weight(FontWeight(800.))
                                .child(t("voice.camera")),
                        )
                        .child(div().text_size(px(theme.font_size_xl)).child(title))
                        .child(div().absolute().top_0().right_0().child(
                            UiCloseButton::new("camera-setup-close", theme).on_click(cx.listener(
                                |this, _, window, cx| this.dismiss_camera_setup(window, cx),
                            )),
                        )),
                )
                .child(preview)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t("voice.chooseCameraQuality")),
                )
                .child(quality_picker)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t("voice.chooseCameraDevice")),
                )
                .child(device_picker)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .flex_wrap()
                        .gap(px(theme.space2))
                        .child(
                            Button::new("camera-cancel", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.cancel"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dismiss_camera_setup(window, cx)
                                })),
                        )
                        .child(
                            Button::new("camera-start", theme)
                                .variant(ButtonVariant::Primary)
                                .disabled(setup.selected_device().is_none())
                                .label(t("voice.startCamera"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_camera_setup(window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn disclosure_choice(
    id: ElementId,
    theme: Theme,
    label: String,
    description: Option<String>,
    selected: bool,
) -> Button {
    Button::new(id, theme)
        .variant(ButtonVariant::DisclosureChoice)
        .selected(selected)
        .accessibility_label(label.clone())
        .leading(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(theme.space1))
                .child(div().truncate().child(label))
                .when_some(description, |copy, description| {
                    copy.child(
                        div()
                            .text_size(px(theme.font_size_xs))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme.text_muted)
                            .child(description),
                    )
                }),
        )
}

fn camera_icon(theme: Theme) -> impl IntoElement {
    div()
        .size(px(34.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius_pill))
        .bg(theme.surface_interactive)
        .child(Icon::CameraOutline.render_stroke(theme.accent, 18., 1.6))
}
