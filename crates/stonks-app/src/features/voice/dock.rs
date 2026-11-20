use super::*;
use crate::shared::images::Images;
use gpui_kit::prelude::FluentBuilder;
use stonks_core::connection::AppSnapshot;
use stonks_core::voice::camera::CameraState;
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    i18n::t,
    icons::Icon,
    Theme,
};

struct DockDrag;
struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaLayout {
    IconAndLabel,
    LabelOnly,
    IconOnly,
}

impl MediaLayout {
    fn for_slot(
        label_space: Pixels,
        icon_space: Pixels,
        mut measure: impl FnMut(&str) -> Pixels,
    ) -> Self {
        // Tauri keeps the row mode independent of action state. Fit GPUI's
        // steady labels to its button padding; pending copy can ellipsize in
        // that same slot and remains available in the tooltip and a11y label.
        let widest_label = ["voice.video", "voice.screen", "voice.sounds", "voice.stop"]
            .into_iter()
            .map(&mut measure)
            .fold(px(0.), Pixels::max);
        if label_space >= widest_label + icon_space {
            Self::IconAndLabel
        } else if label_space >= widest_label {
            Self::LabelOnly
        } else {
            Self::IconOnly
        }
    }
}

impl Voice {
    pub fn dock(
        &mut self,
        sidebar_width: Option<f32>,
        focused: Option<(Option<&AppSnapshot>, Option<&str>, &Entity<Images>)>,
        snapshot: Option<&AppSnapshot>,
        images: &Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let music = self.music_card(snapshot, images, window, cx);
        if self.quick_sounds_open()
            && !self
                .roster_state()
                .is_some_and(|state| state.phase == VoicePhase::Connected)
        {
            self.dismiss_soundboard(window, cx);
        }
        if focused.is_none()
            || !self
                .roster_state()
                .is_some_and(|state| state.phase == VoicePhase::Connected)
        {
            self.clear_dock_roster();
        }
        let Some(call) = self
            .call
            .as_ref()
            .filter(|call| !call.hidden && call.stop.is_some())
        else {
            if let Some((_, _, images)) = focused {
                images.update(cx, |images, _| images.set_sources("voice-dock", []));
            }
            return None;
        };
        let state = call.snapshot.as_ref();
        let phase = state.map_or(VoicePhase::Connecting, |state| state.phase);
        let theme = Theme::current(cx);
        let (status_key, status_color) = match phase {
            VoicePhase::Connecting => ("voice.connecting", theme.text_muted),
            VoicePhase::Reconnecting => ("voice.reconnecting", theme.text_muted),
            VoicePhase::Connected => ("voice.connected", theme.success_text),
            VoicePhase::Disconnected => return None,
        };
        let focused_card = focused.is_some();
        let roster = focused.and_then(|(snapshot, autumn, images)| {
            let snapshot = snapshot?;
            let channel = snapshot
                .channels
                .iter()
                .find(|channel| channel.id == call.id)?;
            (phase == VoicePhase::Connected).then(|| {
                (
                    call.id.clone(),
                    roster::participants(channel, snapshot, autumn, state),
                    images,
                )
            })
        });
        if let Some((_, _, images)) = focused {
            images.update(cx, |images, _| {
                images.set_sources(
                    "voice-dock",
                    roster
                        .iter()
                        .flat_map(|(_, entries, _)| entries)
                        .filter_map(|entry| entry.avatar_url.clone())
                        .map(|url| (url, roster::AVATAR_EDGE)),
                )
            });
        }
        let floating = sidebar_width.is_none();
        let viewport = window.viewport_size();
        let width =
            sidebar_width.map_or_else(|| px(276.).min((viewport.width - px(24.)).max(px(1.))), px);
        let margins = if floating { 0. } else { 2. * theme.space2 };
        let content_width = width - px(margins + 2. * theme.space2 + 2.);
        let focused_width = width - px(margins + 16.);
        let stacked = focused_card && focused_width < px(328.);
        let split_controls = focused_card && focused_width < px(233.);
        let compact = content_width <= px(170.);
        let dock = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(theme.space2))
            .p(px(theme.space2))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_md))
            .bg(theme.surface_raised)
            .when(focused_card, |dock| {
                dock.p(px(7.))
                    .gap(px(3.))
                    .min_h(px(72.))
                    .rounded(px(theme.radius_lg))
            });
        let saved = self.dock_position;
        let position = |height| dock_position(saved, viewport, size(width, px(height)));
        let status = t(status_key);
        let muted = state.is_some_and(VoiceSnapshot::effectively_muted);
        let deafened = state.is_some_and(|state| state.deafened);
        let next_mute = !state.is_some_and(|state| state.muted);
        let disabled = phase != VoicePhase::Connected;
        let control = |id, icon: Icon, label: String, danger| {
            Button::new(id, theme)
                .variant(if danger {
                    ButtonVariant::DangerSubtle
                } else {
                    ButtonVariant::Secondary
                })
                .icon_only(true)
                .leading(icon.render(
                    if danger { theme.danger } else { theme.text },
                    if focused_card { 20. } else { 17. },
                ))
                .accessibility_label(label.clone())
                .tooltip(label)
        };
        let mute = control(
            "voice-mute",
            if muted { Icon::MicOff } else { Icon::Mic },
            t(if next_mute {
                "settings.voiceSounds.mute"
            } else {
                "settings.voiceSounds.unmute"
            }),
            muted,
        )
        .disabled(disabled || deafened || !state.is_some_and(|state| state.can_speak))
        .on_click(cx.listener(move |this, _, _, _| this.control(Control::Mute(next_mute))));
        let deafen = control(
            "voice-deafen",
            if deafened {
                Icon::HeadphonesOff
            } else {
                Icon::Headphones
            },
            t(if deafened {
                "settings.voiceSounds.undeafen"
            } else {
                "settings.voiceSounds.deafen"
            }),
            deafened,
        )
        .disabled(disabled)
        .on_click(cx.listener(move |this, _, _, _| this.control(Control::Deafen(!deafened))));
        let leave = control(
            "voice-leave",
            Icon::PhoneOff,
            t(if phase == VoicePhase::Connecting {
                "common.cancel"
            } else {
                "voice.leave"
            }),
            true,
        )
        .on_click(cx.listener(|this, _, _, cx| this.leave(cx)));
        let sharing = call.screen.state;
        let screen_active = matches!(
            sharing,
            ScreenShareState::Starting | ScreenShareState::Sharing | ScreenShareState::Stopping
        );
        let screen_enabled = match sharing {
            ScreenShareState::Starting | ScreenShareState::Sharing => true,
            ScreenShareState::Stopping => false,
            _ => !disabled,
        };
        let camera_state = call.camera.state;
        let camera_active = matches!(
            camera_state,
            CameraState::Starting | CameraState::Publishing | CameraState::Stopping
        );
        let camera_enabled = match camera_state {
            CameraState::Starting | CameraState::Publishing => true,
            CameraState::Stopping => false,
            _ => !disabled,
        };
        let media_actions = [
            (
                "voice-camera",
                Icon::CameraOutline,
                t(match camera_state {
                    CameraState::Starting => "common.cancel",
                    CameraState::Publishing => "voice.stop",
                    CameraState::Stopping => "voice.cameraStopping",
                    _ => "voice.video",
                }),
            ),
            (
                "voice-screen",
                Icon::MonitorUp,
                t(match sharing {
                    ScreenShareState::Starting => "common.cancel",
                    ScreenShareState::Sharing => "voice.stop",
                    ScreenShareState::Stopping => "voice.screenStopping",
                    _ => "voice.screen",
                }),
            ),
            ("voice-sounds", Icon::Music, t("voice.sounds")),
        ];
        let media_gap = px(theme.space1);
        let label_space = (content_width - media_gap * 2.) / 3. - px(2. * theme.space3);
        let icon_size = 16.;
        // Measure in the same bold Inter style the shared Button renders.
        let mut text_style = window.text_style();
        text_style.font_family = fonts::FAMILY.into();
        text_style.font_weight = FontWeight::BOLD;
        let media_layout = if focused_card {
            MediaLayout::IconOnly
        } else {
            MediaLayout::for_slot(label_space, px(20. + theme.space2), |key| {
                let label = t(key);
                let run = text_style.to_run(label.len());
                let line = window.text_system().shape_line(
                    label.into(),
                    px(theme.font_size_sm),
                    &[run],
                    None,
                );
                px(f32::from(line.width).ceil())
            })
        };
        let sounds_open = self.quick_sounds_open();
        let [camera, screen, sounds] = media_actions.map(|(id, icon, label)| {
            let sounds = id == "voice-sounds";
            let screen = id == "voice-screen";
            let camera = id == "voice-camera";
            let enabled =
                (sounds && !disabled) || (screen && screen_enabled) || (camera && camera_enabled);
            let cue = if sounds {
                t(if sounds_open {
                    "voice.closeSoundboard"
                } else {
                    "voice.openSoundboard"
                })
            } else if screen {
                t(match sharing {
                    ScreenShareState::Starting => "voice.cancelScreenSelection",
                    ScreenShareState::Sharing => "voice.stopSharing",
                    ScreenShareState::Stopping => "voice.screenStopping",
                    _ => "voice.shareScreen",
                })
            } else {
                t(match camera_state {
                    CameraState::Starting => "common.cancel",
                    CameraState::Publishing => "voice.cameraOff",
                    CameraState::Stopping => "voice.cameraStopping",
                    _ => "voice.cameraOn",
                })
            };
            let button = Button::new(id, theme)
                .variant(ButtonVariant::Secondary)
                .tooltip(cue.clone())
                .accessibility_label(cue)
                .disabled(!enabled);
            let button = if screen {
                button
                    .selected(screen_active)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_screen(window, cx)))
            } else if camera {
                button
                    .variant(if camera_active {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .selected(camera_active)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_camera(window, cx)))
            } else if enabled {
                button.expanded(sounds_open).selected(sounds_open).on_click(
                    cx.listener(|this, _, window, cx| this.toggle_quick_sounds(window, cx)),
                )
            } else {
                button
            };
            let button = if focused_card && (screen && screen_active || sounds && sounds_open) {
                button.variant(ButtonVariant::Primary)
            } else {
                button
            };
            let icon = || {
                icon.render_stroke(
                    if !enabled {
                        theme.text_muted
                    } else if camera && camera_active
                        || focused_card && (screen && screen_active || sounds && sounds_open)
                    {
                        theme.accent_text
                    } else {
                        theme.text
                    },
                    if focused_card || camera {
                        20.
                    } else {
                        icon_size
                    },
                    if focused_card || camera { 1.8 } else { 2. },
                )
            };
            match media_layout {
                MediaLayout::IconOnly => button.icon_only(true).leading(icon()),
                _ => button.leading(
                    div()
                        .w(label_space)
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(theme.space2))
                        .when(media_layout == MediaLayout::IconAndLabel, |content| {
                            content.child(div().flex_shrink_0().child(icon()))
                        })
                        .child(div().min_w_0().truncate().child(label)),
                ),
            }
        });
        let row = || div().flex().items_center().gap(px(theme.space1));
        let name = call.name.replace(['\r', '\n'], " ");
        let copy = || {
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(theme.font_size_xs))
                .child(
                    div()
                        .text_color(status_color)
                        .truncate()
                        .child(status.clone()),
                )
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(name.clone()),
                )
        };
        let music = music.and_then(|card| {
            let view = card.read(cx);
            view.visible().then(|| {
                use super::music::MusicLayout;
                let layout = if focused_card {
                    MusicLayout::Horizontal
                } else if compact {
                    MusicLayout::Compact
                } else {
                    MusicLayout::Standard
                };
                let music_width = if floating {
                    content_width
                } else {
                    width - px(margins)
                };
                view.render_card(&card, layout, music_width, theme, cx)
            })
        });
        let height = if compact { 164. } else { 124. } + if music.is_some() { 120. } else { 0. };
        let dock = if floating {
            let origin = position(height);
            dock.absolute()
                .left(origin.x)
                .top(origin.y)
                .w(width)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_md.color.into(),
                    offset: point(px(0.), px(theme.shadow_md.dy)),
                    blur_radius: px(theme.shadow_md.blur),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .child(
                    div()
                        .id("voice-dock-drag")
                        .self_center()
                        .w(px(44.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_move()
                        .child(Icon::GripHorizontal.render(theme.text_muted, 20.))
                        .on_drag(DockDrag, {
                            let voice = cx.entity();
                            move |_, _, window, cx| {
                                voice.update(cx, |voice, _| {
                                    voice.drag_offset = window.mouse_position() - origin
                                });
                                cx.new(|_| DragPreview)
                            }
                        })
                        .on_drag_move(cx.listener(
                            move |this, event: &DragMoveEvent<DockDrag>, window, cx| {
                                this.dock_position = Some(dock_position(
                                    Some(event.event.position - this.drag_offset),
                                    window.viewport_size(),
                                    size(width, px(height)),
                                ));
                                cx.notify();
                            },
                        )),
                )
        } else {
            dock
        };
        let (mut dock, music) = if floating {
            (dock.children(music), None)
        } else {
            (dock, music)
        };
        let voice = cx.weak_entity();
        let sounds = div().relative().child(sounds).when(sounds_open, |trigger| {
            trigger.child(
                canvas(
                    move |bounds, _, cx| {
                        let voice = voice.clone();
                        cx.defer(move |cx| {
                            let _ =
                                voice.update(cx, |this, cx| this.measure_sounds_anchor(bounds, cx));
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        });
        let media_errors = |dock: Div| {
            dock.when_some(
                match sharing {
                    ScreenShareState::Failed(error) => Some(error),
                    _ => None,
                },
                |dock, error| {
                    dock.child(
                        div()
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.danger)
                            .child(error.to_string()),
                    )
                },
            )
            .when_some(
                match camera_state {
                    CameraState::Failed(error) => Some(error),
                    _ => None,
                },
                |dock, error| {
                    dock.child(
                        div()
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.danger)
                            .child(error.to_string()),
                    )
                },
            )
        };
        if focused_card {
            let controls = || div().flex().items_center().gap(px(4.)).flex_shrink_0();
            let media = controls().child(camera).child(screen).child(sounds);
            let call = controls().child(mute).child(deafen).child(leave);
            let copy_width = focused_width - px(if stacked { 8. } else { 247. });
            let copy = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .pl(px(8.))
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(theme.font_size_md))
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(name),
                )
                .when_some(roster, |copy, (channel, entries, images)| {
                    copy.child(self.dock_participants(
                        &channel, &entries, copy_width, images, theme, viewport, cx,
                    ))
                });
            let status_color = if disabled {
                theme.warning
            } else {
                theme.success
            };
            let status = row()
                .justify_end()
                .gap(px(7.))
                .text_size(px(theme.font_size_xs))
                .font_weight(FontWeight::BOLD)
                .text_color(status_color)
                .child(
                    div()
                        .size(px(4.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(status_color),
                )
                .child(div().truncate().child(status));
            let actions = div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(3.))
                .min_w_0()
                .child(status)
                .child(
                    row()
                        .justify_end()
                        .gap(px(6.))
                        .when(split_controls, |row| row.flex_col().items_end())
                        .child(media)
                        .when(!split_controls, |row| {
                            row.child(div().w(px(1.)).h(px(28.)).bg(theme.outline))
                        })
                        .child(call),
                );
            dock = dock.child(
                row()
                    .gap(px(6.))
                    .when(stacked, |row| row.flex_col().items_stretch())
                    .child(copy)
                    .child(actions),
            );
            dock = media_errors(dock);
        } else {
            dock = if compact {
                dock.child(copy()).child(
                    row()
                        .justify_center()
                        .child(mute)
                        .child(deafen)
                        .child(leave),
                )
            } else {
                dock.child(row().child(copy()).child(mute).child(deafen).child(leave))
            };
            dock = media_errors(dock).child(
                row()
                    .justify_center()
                    .child(camera)
                    .child(screen)
                    .child(sounds),
            );
        }
        Some(if floating {
            dock
        } else {
            // Measure the whole stack in the existing focused overlay, not just the call card.
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .min_w_0()
                .m(px(theme.space2))
                .gap(px(theme.space2))
                .children(music)
                .child(dock)
        })
    }

    pub fn error_notice(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        // A disconnected or failed-to-close room is not a failed join.
        let error = self
            .error
            .as_ref()
            .filter(|error| !matches!(error, VoiceError::Disconnected | VoiceError::Close))?;
        let theme = Theme::current(cx);
        Some(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(theme.space2))
                .p(px(theme.space2))
                .border_1()
                .border_color(theme.outline)
                .rounded(px(theme.radius_md))
                .bg(theme.surface_raised)
                .text_size(px(theme.font_size_xs))
                .text_color(theme.danger)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(error_label(error, is_pt_br(cx))),
                )
                .child(
                    Button::new("voice-error-close", theme)
                        .variant(ButtonVariant::Close)
                        .tooltip(t("common.close"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{px, MediaLayout};

    #[test]
    fn media_layout_fits_steady_labels_without_budgeting_for_pending_copy() {
        use MediaLayout::*;
        // Representative shaped widths, not font metrics. Pending labels
        // wider than the slot must not affect any of these mode boundaries.
        let measure = |key: &str| {
            px(match key {
                "voice.video" => 30.,
                "voice.screen" => 39.,
                "voice.sounds" => 43.,
                "voice.stop" => 27.,
                "common.cancel" => 48.,
                "voice.cameraStopping" | "voice.screenStopping" => 110.,
                _ => panic!("unexpected media label: {key}"),
            })
        };
        for (slot, expected) in [
            (71., IconAndLabel),
            (70., LabelOnly),
            (59., LabelOnly), // Approximate default docked/floating label space.
            (43., LabelOnly),
            (42., IconOnly),
        ] {
            assert_eq!(MediaLayout::for_slot(px(slot), px(28.), measure), expected);
        }
    }
}

fn dock_position(
    saved: Option<Point<Pixels>>,
    viewport: Size<Pixels>,
    card: Size<Pixels>,
) -> Point<Pixels> {
    let limit = point(
        (viewport.width - card.width).max(px(0.)),
        (viewport.height - card.height).max(px(0.)),
    );
    let desired = saved.unwrap_or_else(|| point(limit.x - px(12.), limit.y - px(64.)));
    point(
        desired.x.clamp(px(0.), limit.x),
        desired.y.clamp(px(0.), limit.y),
    )
}

impl Voice {
    pub(crate) fn shortcut_toggle_audio(&self, deafen: bool) -> bool {
        let Some(state) = self
            .roster_state()
            .filter(|state| state.phase == VoicePhase::Connected)
        else {
            return false;
        };
        let command = if deafen {
            Control::Deafen(!state.deafened)
        } else if state.can_speak && !state.deafened {
            Control::Mute(!state.muted)
        } else {
            return false;
        };
        self.control(command);
        true
    }

    fn control(&self, command: Control) {
        if let Some(call) = self.call.as_ref().filter(|call| call.stop.is_some()) {
            let _ = call.controls.try_send(command);
        }
    }
}

fn is_pt_br(cx: &App) -> bool {
    cx.global::<crate::preferences::NativePreferences>()
        .saved
        .effective_language()
        == "pt-BR"
}

fn localized(is_pt_br: bool, en: &str, pt_br: &str) -> String {
    if is_pt_br {
        pt_br.into()
    } else {
        en.into()
    }
}

fn error_label(error: &VoiceError, pt_br: bool) -> String {
    match error {
        VoiceError::ConnectDenied => localized(
            pt_br,
            "Voice access is unavailable for this channel.",
            "O acesso por voz não está disponível neste canal.",
        ),
        VoiceError::SpeakDenied => localized(
            pt_br,
            "Speaking permission changed. Rejoin to listen.",
            "A permissão para falar mudou. Entre novamente para ouvir.",
        ),
        VoiceError::Capture | VoiceError::Playback => localized(
            pt_br,
            "Voice audio failed. Check PipeWire and your audio devices.",
            "O áudio da chamada falhou. Verifique o PipeWire e os dispositivos de áudio.",
        ),

        _ => t("voice.connectionFailed"),
    }
}
