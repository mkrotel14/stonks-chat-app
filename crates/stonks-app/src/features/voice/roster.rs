use std::{cell::Cell, rc::Rc};

mod strip;
pub(super) use strip::Overflow;

use super::Voice;
use crate::shared::images::Images;
use gpui_kit::{
    component::slider::{SliderEvent, SliderState, SliderValue},
    prelude::FluentBuilder,
    *,
};
use stonks_core::connection::{AppSnapshot, ClientChannel};
use stonks_core::voice::{VoicePhase, VoiceSnapshot};
use stonks_ui::{
    components::{avatar, Button, ButtonVariant, UiRange},
    i18n::{t, t_vars},
    icons::Icon,
    theme::feedback_overlay,
    Theme,
};

pub const AVATAR_EDGE: u32 = 64;
const AVATAR_SIZE: f32 = 32.;

pub struct Participant {
    id: String,
    name: String,
    pub avatar_url: Option<String>,
    live: bool,
    is_self: bool,
    camera: bool,
    speaking: bool,
    muted: bool,
    deafened: bool,
}

pub fn participants(
    channel: &ClientChannel,
    snapshot: &AppSnapshot,
    autumn: Option<&str>,
    local: Option<&VoiceSnapshot>,
) -> Vec<Participant> {
    let local = local.filter(|state| {
        state.channel_id == channel.id
            && matches!(
                state.phase,
                VoicePhase::Connected | VoicePhase::Reconnecting
            )
    });
    let entries = snapshot
        .voice_states
        .iter()
        .find(|state| state.id == channel.id)
        .into_iter()
        .flat_map(|state| &state.participants)
        .collect::<Vec<_>>();
    let me = snapshot.current_user.id.as_str();
    let local_only = local.is_some() && !entries.iter().any(|entry| entry.id == me);
    entries
        .into_iter()
        .map(|entry| (entry.id.as_str(), Some(entry)))
        .chain(local_only.then_some((me, None)))
        .map(|(id, entry)| {
            let speaking = local.is_some_and(|state| state.is_speaking(id));
            let local = if id == me { local } else { None };
            let user = if id == me {
                Some(&snapshot.current_user)
            } else {
                snapshot.users.iter().find(|user| user.id == id)
            };
            let member = snapshot.members.iter().find(|member| {
                channel.server.as_ref() == Some(&member.id.server) && member.id.user == id
            });
            let name = member
                .and_then(|member| member.nickname.clone())
                .or_else(|| {
                    user.map(|user| {
                        user.display_name
                            .clone()
                            .unwrap_or_else(|| user.username.clone())
                    })
                })
                .unwrap_or_else(|| id.to_owned());
            let avatar_url = member
                .and_then(|member| member.avatar.as_ref())
                .or_else(|| user.and_then(|user| user.avatar.as_ref()))
                .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id));
            Participant {
                id: id.to_owned(),
                name,
                avatar_url,
                is_self: id == me,
                live: entry.is_some_and(|entry| entry.screensharing),
                camera: entry.is_some_and(|entry| entry.camera),
                speaking,
                muted: local.map_or_else(
                    || entry.is_some_and(|entry| entry.is_publishing == Some(false)),
                    VoiceSnapshot::effectively_muted,
                ),
                // The gateway has no reliable signal for another client's local playback mute.
                deafened: local.is_some_and(|state| state.deafened),
            }
        })
        .collect()
}

fn is_connected_roster(state: Option<&VoiceSnapshot>, channel: &str) -> bool {
    state.is_some_and(|state| state.channel_id == channel && state.phase == VoicePhase::Connected)
}

pub fn render(
    channel_id: &str,
    participants: &[Participant],
    images: &Entity<Images>,
    voice: &Entity<Voice>,
    theme: Theme,
    cx: &mut App,
) -> Stateful<Div> {
    let can_view = voice.read(cx).can_view_video(channel_id);
    let connected = voice.read(cx).roster_state();
    let adjustable = is_connected_roster(connected, channel_id);
    div()
        .id(ElementId::Name(format!("voice-roster-{channel_id}").into()))
        .flex()
        .flex_col()
        .gap(px(3.))
        .mt(px(1.))
        .mr(px(theme.space1))
        .mb(px(theme.space2))
        .ml(px(theme.space5))
        .children(participants.iter().map(|entry| {
            let live = if entry.is_self && voice.read(cx).active(channel_id) {
                voice.read(cx).is_sharing_screen()
            } else {
                entry.live
            };
            let camera = if entry.is_self && voice.read(cx).active(channel_id) {
                voice.read(cx).is_publishing_camera()
            } else {
                entry.camera
            };
            let image = images.update(cx, |images, cx| {
                images.get(entry.avatar_url.as_deref(), AVATAR_EDGE, cx)
            });
            // The shared avatar's image ID is local to this participant row,
            // so GPUI keeps each avatar's playback state separate.
            let open = adjustable
                && !entry.is_self
                && voice
                    .read(cx)
                    .participant_volume
                    .as_ref()
                    .is_some_and(|panel| {
                        !panel.in_dock && panel.channel == channel_id && panel.id == entry.id
                    });
            let focus = voice
                .read(cx)
                .participant_volume
                .as_ref()
                .filter(|_| open)
                .map(|p| p.opener.clone())
                .unwrap_or_else(|| cx.focus_handle());
            div()
                .id(SharedString::from(entry.id.clone()))
                .relative()
                .when(adjustable && !entry.is_self, |row| {
                    let voice = voice.clone();
                    let channel = channel_id.to_owned();
                    let id = entry.id.clone();
                    row.aria_expanded(open)
                        .tab_index(0)
                        .track_focus(&focus)
                        .hover(|row| row.bg(theme.message_hover))
                        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                            voice.update(cx, |voice, cx| {
                                voice.open_participant_menu(
                                    &channel,
                                    &id,
                                    event.position,
                                    &focus,
                                    false,
                                    window,
                                    cx,
                                );
                            });
                            cx.stop_propagation();
                        })
                })
                .when(open, |row| {
                    row.child(ParticipantMenu {
                        voice: voice.clone(),
                        name: entry.name.clone(),
                        theme,
                        opener_bounds: None,
                    })
                })
                .flex()
                .items_center()
                .gap(px(10.))
                .min_w_0()
                .min_h(px(42.))
                .px(px(theme.space2))
                .py(px(5.))
                .text_size(px(theme.font_size_sm))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(
                    div()
                        .relative()
                        .flex_shrink_0()
                        .child(avatar(&theme, &entry.name, AVATAR_SIZE, image))
                        .when(entry.speaking, |wrapper| {
                            wrapper.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .left(px(-2.))
                                    .size(px(AVATAR_SIZE + 4.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(theme.success),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .child(entry.name.clone()),
                )
                .when(
                    live || entry.camera || entry.muted || entry.deafened,
                    |row| {
                        row.child(
                            div()
                                .id(ElementId::Name(format!("voice-badges-{}", entry.id).into()))
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap(px(6.))
                                .when(camera, |badges| {
                                    if !can_view || entry.is_self {
                                        return badges.child(badge(
                                            "camera",
                                            "voice.video",
                                            Icon::Video,
                                            theme.text_muted,
                                        ));
                                    }
                                    let (voice, channel, id, name) = (
                                        voice.clone(),
                                        channel_id.to_owned(),
                                        entry.id.clone(),
                                        entry.name.clone(),
                                    );
                                    let label = t("voice.viewCamera");
                                    badges.child(
                                        Button::new("camera", theme)
                                            .variant(ButtonVariant::Secondary)
                                            .tooltip(label.clone())
                                            .accessibility_label(label)
                                            .icon_only(true)
                                            .leading(Icon::Video.render(theme.text, 14.))
                                            .on_click(move |_, window, cx| {
                                                voice.update(cx, |voice, cx| {
                                                    voice.view_video(
                                                        super::video::Kind::Camera,
                                                        &channel,
                                                        &id,
                                                        &name,
                                                        window,
                                                        cx,
                                                    );
                                                })
                                            }),
                                    )
                                })
                                .when(live, |badges| {
                                    let screen = live_badge(entry, can_view, theme);
                                    if entry.is_self || !can_view {
                                        return badges.child(screen);
                                    }
                                    let (voice, channel, id, name) = (
                                        voice.clone(),
                                        channel_id.to_owned(),
                                        entry.id.clone(),
                                        entry.name.clone(),
                                    );
                                    badges.child(screen.on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        voice.update(cx, |voice, cx| {
                                            voice.view_video(
                                                super::video::Kind::Screen,
                                                &channel,
                                                &id,
                                                &name,
                                                window,
                                                cx,
                                            )
                                        });
                                    }))
                                })
                                .when(entry.muted, |badges| {
                                    badges.child(badge(
                                        "mute",
                                        "voice.muted",
                                        Icon::MicOff,
                                        theme.text_muted,
                                    ))
                                })
                                .when(entry.deafened, |badges| {
                                    badges.child(badge(
                                        "deafen",
                                        "voice.deafened",
                                        Icon::HeadphonesOff,
                                        theme.text_muted,
                                    ))
                                }),
                        )
                    },
                )
        }))
}

pub(super) struct VolumePopover {
    channel: String,
    id: String,
    position: Point<Pixels>,
    in_dock: bool,
    slider: Entity<SliderState>,
    focus: FocusHandle,
    opener: FocusHandle,
    _subscription: Subscription,
}

impl Voice {
    fn open_participant_menu(
        &mut self,
        channel: &str,
        id: &str,
        position: Point<Pixels>,
        opener: &FocusHandle,
        in_dock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !is_connected_roster(self.roster_state(), channel) {
            return;
        }
        let gain = self.listening.gains.participant_microphone_gain(id);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(gain))
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let participant = id.to_owned();
        let subscription = cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                this.set_microphone_gain(&participant, value.round() as u8, cx);
            }
        });
        self.participant_volume = Some(VolumePopover {
            channel: channel.to_owned(),
            id: id.to_owned(),
            position,
            in_dock,
            slider,
            focus,
            opener: opener.clone(),
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_participant_volume(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.participant_volume.take() {
            if restore {
                panel.opener.focus(window, cx);
            }
            cx.notify();
        }
    }
}

#[derive(IntoElement)]
struct ParticipantMenu {
    voice: Entity<Voice>,
    name: String,
    theme: Theme,
    opener_bounds: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
}

impl RenderOnce for ParticipantMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let voice = self.voice;
        let Some(panel) = voice.read(cx).participant_volume.as_ref() else {
            return div();
        };
        let slider = panel.slider.clone();
        let position = panel.position;
        let anchor = if panel.in_dock {
            Anchor::BottomLeft
        } else {
            Anchor::TopLeft
        };
        let focus = panel.focus.clone();
        let error = voice.read(cx).listening.storage_error;
        let gain = voice
            .read(cx)
            .listening
            .gains
            .participant_microphone_gain(&panel.id);
        let theme = self.theme;
        let viewport = window.viewport_size();
        let margin = px(12.).min(viewport.width / 4.).min(viewport.height / 4.);
        let width = px(260.).min(viewport.width - margin * 2.);
        let outside = voice.clone();
        let opener_bounds = self.opener_bounds;
        let surface = div()
            .id("participant-volume")
            .role(Role::Dialog)
            .aria_label(t_vars("voice.volumeFor", &[("name", &self.name)]))
            .occlude()
            .w(width)
            .max_h(viewport.height - margin * 2.)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(theme.space3))
            .rounded(px(theme.radius_md))
            .border_1()
            .border_color(theme.outline)
            .bg(theme.surface_raised)
            .text_color(theme.text)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(move |event, window, cx| {
                if opener_bounds
                    .as_ref()
                    .and_then(|bounds| bounds.get())
                    .is_some_and(|bounds| bounds.contains(&event.position))
                {
                    return;
                }
                outside.update(cx, |this, cx| {
                    this.close_participant_volume(false, window, cx)
                });
            })
            .capture_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" {
                    voice.update(cx, |this, cx| {
                        this.close_participant_volume(true, window, cx)
                    });
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .truncate()
                    .text_size(px(theme.font_size_sm))
                    .font_weight(FontWeight::BOLD)
                    .child(self.name.clone()),
            )
            .child(microphone_volume(&slider, &focus, &self.name, gain, theme))
            .when(error, |surface| {
                surface.child(
                    div()
                        .text_size(px(theme.font_size_xs))
                        .text_color(theme.danger)
                        .child(t("voice.listeningSaveError")),
                )
            });
        div().absolute().inset_0().child(
            deferred(
                anchored()
                    .anchor(anchor)
                    .position_mode(AnchoredPositionMode::Window)
                    .position(position)
                    .snap_to_window_with_margin(margin)
                    .child(surface),
            )
            .with_priority(1),
        )
    }
}

fn microphone_volume(
    slider: &Entity<SliderState>,
    focus: &FocusHandle,
    name: &str,
    gain: u8,
    theme: Theme,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme.space2))
        .text_size(px(theme.font_size_xs))
        .text_color(theme.text_muted)
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(Icon::Volume2.render(theme.text_muted, 17.))
                .child(t("voice.participantVoice")),
        )
        .child(
            div().flex_1().min_w_0().child(
                UiRange::new(slider, theme)
                    .track_focus(focus)
                    .label(t_vars("voice.voiceVolumeFor", &[("name", name)])),
            ),
        )
        .child(
            div()
                .w(px(42.))
                .flex_shrink_0()
                .text_right()
                .child(format!("{gain}%")),
        )
}

// Matches the sidebar's UiMediaBadge, not the generic Button geometry.
fn live_badge(entry: &Participant, can_view: bool, theme: Theme) -> Stateful<Div> {
    let title: SharedString = if entry.is_self {
        "You are sharing your screen".into()
    } else {
        format!("View screen shared by {}", entry.name).into()
    };
    let badge = div()
        .id("screen")
        .flex_shrink_0()
        .px(px(6.))
        .py(px(3.))
        .rounded(px(theme.radius_sm))
        .bg(theme.surface_raised)
        .text_color(theme.success_text)
        .text_size(px(theme.font_size_xs))
        .line_height(relative(1.5))
        .font_weight(FontWeight::BLACK)
        .whitespace_nowrap()
        .aria_label(title.clone())
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(title.clone()).build(window, cx)
        })
        .child("LIVE");
    if entry.is_self {
        return badge;
    }
    let button = badge.role(Role::Button);
    if !can_view {
        return button.cursor_not_allowed().opacity(0.5);
    }
    let hover_bg = feedback_overlay(theme.surface_raised, theme.text, 0.10);
    let active_bg = feedback_overlay(theme.surface_raised, theme.text, 0.18);
    button
        .tab_index(0)
        .cursor_pointer()
        .hover(move |style| {
            style.bg(hover_bg).shadow(vec![BoxShadow {
                color: Rgba {
                    a: 0.24,
                    ..theme.text
                }
                .into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(1.),
                inset: true,
            }])
        })
        .active(move |style| style.bg(active_bg))
        .focus_visible(move |style| {
            style.shadow(vec![BoxShadow {
                color: theme.focus_ring.into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
        })
}

fn badge(id: &'static str, label: &str, icon: Icon, color: Rgba) -> Stateful<Div> {
    let label = t(label);
    div()
        .id(id)
        .aria_label(label.clone())
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
        })
        .child(icon.render(color, 18.))
}

#[cfg(test)]
mod tests {
    use super::{is_connected_roster, participants};
    use serde_json::json;
    use stonks_core::{
        connection::AppSnapshot,
        voice::{VoicePhase, VoiceSnapshot},
    };

    #[test]
    fn distinguishes_gateway_badges_from_local_controls() {
        let channel =
            serde_json::from_value(json!({"_id":"room","channel_type":"VoiceChannel"})).unwrap();
        let snapshot = AppSnapshot {
            current_user: serde_json::from_value(json!({"_id":"me","username":"Me"})).unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: serde_json::from_value(json!([{"id":"room","participants":[
                {"id":"unknown"},
                {"id":"remote","is_publishing":false,"is_receiving":false,"camera":true,"screensharing":true},
                {"id":"me","isPublishing":true}
            ]}])).unwrap(),
        };
        let pre_join = participants(&channel, &snapshot, None, None);
        assert!(!pre_join[0].muted);
        assert!(pre_join[1].muted && pre_join[1].camera && pre_join[1].live);
        assert!(!pre_join[1].deafened);
        assert!(!pre_join[2].muted && !pre_join[2].deafened);

        let state = VoiceSnapshot {
            channel_id: "room".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: false,
            deafened: true,
            error: None,
            active_speakers: Default::default(),
        };
        assert!(is_connected_roster(Some(&state), "room"));
        assert!(!is_connected_roster(Some(&state), "another-room"));
        assert!(!is_connected_roster(None, "room"));
        let joined = participants(&channel, &snapshot, None, Some(&state));
        assert_eq!(joined.len(), 3);
        assert!(joined[2].muted && joined[2].deafened);
        assert!(!joined[1].deafened);
        assert!(joined.iter().all(|entry| !entry.speaking));

        let mut state = VoiceSnapshot {
            deafened: false,
            active_speakers: ["me".into(), "remote".into()].into(),
            ..state
        };
        let joined = participants(&channel, &snapshot, None, Some(&state));
        assert!(!joined[0].speaking);
        assert!(joined[1].speaking && joined[2].speaking);

        state.phase = VoicePhase::Reconnecting;
        assert!(!is_connected_roster(Some(&state), "room"));
        assert!(participants(&channel, &snapshot, None, Some(&state))
            .iter()
            .all(|entry| !entry.speaking));
        state.phase = VoicePhase::Connected;
        state.channel_id = "another-room".into();
        assert!(participants(&channel, &snapshot, None, Some(&state))
            .iter()
            .all(|entry| !entry.speaking));
    }
}
