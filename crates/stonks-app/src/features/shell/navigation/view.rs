//! Server rail and channel sidebar.

use super::super::*;
use super::notifications;
use crate::features::home::messages as home_messages;
use crate::features::voice::roster;
use std::collections::HashMap;
use stonks_core::channel_preferences::ChannelPreferences;
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientServer};
use stonks_core::messages::read;
use stonks_core::server::ServerSelection;
use stonks_core::voice::VoicePhase;

fn server_unread_count(
    snapshot: &AppSnapshot,
    server_id: &str,
    preferences: &ChannelPreferences,
    now: i64,
    can_view: impl Fn(&str) -> bool,
) -> usize {
    snapshot
        .channels
        .iter()
        .filter(|channel| {
            channel.server.as_deref() == Some(server_id)
                && channel.last_message_id.as_deref().is_some_and(|last| {
                    read::cursor(snapshot, &channel.id).is_none_or(|seen| last > seen)
                })
                && !preferences.is_muted(&channel.id, now)
                && can_view(&channel.id)
        })
        .count()
}

fn server_has_voice_activity(
    snapshot: &AppSnapshot,
    server: &ClientServer,
    locally_active: impl Fn(&str) -> bool,
) -> bool {
    // A gateway participant matching this account may be connected from another client.
    server.channels.iter().any(|id| locally_active(id))
        || snapshot
            .voice_states
            .iter()
            .any(|state| server.channels.contains(&state.id) && !state.participants.is_empty())
}

fn unread_label(count: usize) -> Option<String> {
    (count > 0).then(|| {
        if count > 9 {
            "9+".into()
        } else {
            count.to_string()
        }
    })
}

fn rail_badge(theme: Theme) -> Div {
    div()
        .absolute()
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .border_3()
        .border_color(theme.surface_raised)
        .rounded(px(theme.radius_pill))
}

fn rail_unread(icon: Div, unread: Option<usize>, theme: Theme) -> Div {
    match unread {
        Some(0) => icon.child(
            div()
                .absolute()
                .right(px(-1.))
                .bottom(px(2.))
                .size(px(10.))
                .border_2()
                .border_color(theme.surface_raised)
                .rounded_full()
                .bg(theme.text),
        ),
        Some(count) => icon.child(
            rail_badge(theme)
                .top(px(-2.))
                .right(px(-1.))
                .min_w(px(18.))
                .px(px(4.))
                .font_weight(FontWeight(850.))
                .bg(theme.danger)
                .text_color(theme.danger_text)
                .text_size(px(9.))
                .line_height(relative(1.))
                .child(unread_label(count).unwrap_or_default()),
        ),
        None => icon,
    }
}

pub(in crate::features::shell) fn channel_icon(channel: &ClientChannel) -> Icon {
    if channel.is_voice() {
        Icon::Headphones
    } else {
        Icon::Hash
    }
}

pub(in crate::features::shell) fn channel_label(
    channel: &ClientChannel,
    snapshot: Option<&stonks_core::connection::AppSnapshot>,
) -> String {
    if let Some(snapshot) = snapshot.filter(|_| channel.server.is_none()) {
        return home_messages::label(channel, snapshot);
    }
    channel
        .name
        .clone()
        .or_else(|| {
            snapshot.and_then(|snapshot| {
                channel
                    .recipients
                    .iter()
                    .filter(|id| *id != &snapshot.current_user.id)
                    .find_map(|id| snapshot.users.iter().find(|user| &user.id == id))
                    .map(|user| {
                        user.display_name
                            .clone()
                            .unwrap_or_else(|| user.username.clone())
                    })
            })
        })
        .unwrap_or_else(|| t("shell.messages"))
}

impl ShellView {
    pub(in crate::features::shell) fn render_server_rail(
        &self,
        servers: &[ClientServer],
        active_server: Option<&ClientServer>,
        snapshot: Option<&AppSnapshot>,
        deployment: Option<&ServerSelection>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let autumn = deployment.and_then(|server| server.endpoints.autumn.as_deref());
        let user = snapshot.map(|snapshot| &snapshot.current_user);
        let own_url = user
            .and_then(|user| user.avatar.as_ref())
            .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id));
        let now = notifications::now();
        let (direct_unread, pins) = snapshot.map_or((0, Vec::new()), |snapshot| {
            let mut unread_count = 0;
            let mut pins = Vec::new();
            for channel in home_messages::direct_channels(snapshot) {
                if !self.session.read(cx).can_view_channel(&channel.id) {
                    continue;
                }
                let unread = (!self.notifications.read(cx).saved.is_muted(&channel.id, now))
                    .then(|| read::unread_badge(snapshot, channel))
                    .flatten();
                unread_count += usize::from(unread.is_some());
                if self.home_messages.pinned(&channel.id) {
                    pins.push((
                        channel,
                        home_messages::peer(channel, snapshot),
                        unread,
                        snapshot,
                    ));
                }
            }
            (unread_count, pins)
        });
        let rail_sources: Vec<_> = servers
            .iter()
            .filter_map(|server| server.icon.as_ref())
            .filter_map(|file| stonks_core::media::file_url(autumn, &file.tag, &file.id))
            .chain(own_url.iter().cloned())
            .chain(pins.iter().filter_map(|(_, peer, _, _)| {
                peer.and_then(|user| home_messages::avatar_url(user, autumn))
            }))
            .map(|url| (url, 80))
            .collect();
        self.images
            .update(cx, |images, _| images.set_sources("rail", rail_sources));
        let own_image = self
            .images
            .update(cx, |images, cx| images.get(own_url.as_deref(), 80, cx));
        let mut rail = div()
            .id("server-rail")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(9.))
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .py(px(10.))
            .px(px(7.))
            .bg(theme.surface_raised)
            .child(
                Button::new("rail-home", theme)
                    .variant(ButtonVariant::Rail)
                    .selected(self.home)
                    .tooltip(t("navigation.home"))
                    .leading(rail_unread(
                        div()
                            .relative()
                            .size(px(42.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::MessageCircle.render(theme.text, 24.)),
                        Some(direct_unread).filter(|n| *n > 0),
                        theme,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_home(false, cx);
                    })),
            );
        for (channel, peer, unread, snapshot) in pins {
            let id = channel.id.clone();
            let name = home_messages::label(channel, snapshot);
            let url = peer.and_then(|user| home_messages::avatar_url(user, autumn));
            let image = self
                .images
                .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
            let avatar = stonks_ui::components::presence_avatar(
                &theme,
                peer.map_or(name.as_str(), home_messages::user_name),
                peer.is_some_and(|user| user.online),
                peer.and_then(|user| user.status.as_ref())
                    .and_then(|status| status.presence.as_deref()),
                image,
            );
            rail = rail.child(
                Button::new(
                    ElementId::Name(format!("rail-conversation-{id}").into()),
                    theme,
                )
                .variant(ButtonVariant::Rail)
                .tooltip(name.clone())
                .accessibility_label(name)
                .selected(self.home && self.selected_channel.as_ref() == Some(&id))
                .leading(rail_unread(div().relative().child(avatar), unread, theme))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_home_conversation(id.clone(), cx)),
                ),
            );
        }
        let rail = rail.child(div().w(px(30.)).h(px(1.)).bg(theme.outline));
        let mut server_buttons = div()
            .id("rail-servers")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(10.))
            .overflow_y_scroll();
        for server in servers {
            let (unread, voice_activity) = snapshot.map_or((0, false), |snapshot| {
                let preferences = &self.notifications.read(cx).saved;
                let session = self.session.read(cx);
                let voice = self.voice.read(cx);
                (
                    server_unread_count(snapshot, &server.id, preferences, now, |id| {
                        session.can_view_channel(id)
                    }),
                    server_has_voice_activity(snapshot, server, |id| {
                        voice.active(id)
                            && voice.roster_state().is_none_or(|state| {
                                state.phase != VoicePhase::Disconnected && state.error.is_none()
                            })
                    }),
                )
            });
            let id = server.id.clone();
            let url = server
                .icon
                .as_ref()
                .and_then(|file| stonks_core::media::file_url(autumn, &file.tag, &file.id));
            let image = self
                .images
                .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
            let icon = div()
                .relative()
                .child(stonks_ui::components::avatar(
                    &theme,
                    &server.name,
                    42.,
                    image,
                ))
                .when_some(unread_label(unread), |icon, label| {
                    icon.child(
                        rail_badge(theme)
                            .top(px(-2.))
                            .right(px(-1.))
                            .min_w(px(18.))
                            .px(px(4.))
                            .font_weight(FontWeight(850.))
                            .bg(theme.danger)
                            .text_color(theme.danger_text)
                            .text_size(px(9.))
                            .line_height(relative(1.))
                            .child(label),
                    )
                })
                .when(voice_activity, |icon| {
                    icon.child(
                        rail_badge(theme)
                            .right(px(-3.))
                            .bottom(px(-3.))
                            .w(px(18.))
                            .bg(theme.surface_selected)
                            .child(Icon::RailVoice.render_stroke(theme.text, 11., 1.8)),
                    )
                });
            server_buttons = server_buttons.child(
                Button::new(ElementId::Name(format!("server-{id}").into()), theme)
                    .variant(ButtonVariant::Rail)
                    .selected(active_server.is_some_and(|active| active.id == id))
                    .leading(icon)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_server(id.clone(), cx)),
                    ),
            );
        }
        rail.child(server_buttons).child(
            Button::new("rail-account", theme)
                .variant(ButtonVariant::Rail)
                .leading(stonks_ui::components::presence_avatar(
                    &theme,
                    user.map(|u| u.username.as_str()).unwrap_or("?"),
                    user.is_some(),
                    user.and_then(|u| u.status.as_ref())
                        .and_then(|s| s.presence.as_deref()),
                    own_image,
                ))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_account_settings(window, cx);
                })),
        )
    }

    pub(in crate::features::shell) fn render_channel_sidebar(
        &self,
        channels: &[&ClientChannel],
        active_server: Option<&ClientServer>,
        snapshot: Option<&AppSnapshot>,
        width: f32,
        dock: Option<Div>,
        bottom_inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        // Channel grouping: uncategorized first (server order), then each
        // category with its uppercase title.
        let categorized: Vec<&str> = active_server
            .map(|server| {
                server
                    .categories
                    .iter()
                    .flat_map(|category| category.channels.iter().map(String::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let uncategorized: Vec<&&ClientChannel> = channels
            .iter()
            .filter(|channel| !categorized.contains(&channel.id.as_str()))
            .collect();

        let autumn = self
            .session
            .read(cx)
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let local_voice = self.voice.read(cx).roster_state();
        let rosters: HashMap<_, _> = snapshot
            .into_iter()
            .flat_map(|snapshot| {
                channels
                    .iter()
                    .copied()
                    .filter(|channel| channel.is_voice())
                    .map(move |channel| {
                        (
                            channel.id.as_str(),
                            roster::participants(channel, snapshot, autumn, local_voice),
                        )
                    })
            })
            .collect();
        self.images.update(cx, |images, _| {
            images.set_sources(
                "voice-roster",
                rosters
                    .values()
                    .flatten()
                    .filter_map(|entry| entry.avatar_url.clone())
                    .map(|url| (url, roster::AVATAR_EDGE)),
            )
        });

        let channel_row = |channel: &&ClientChannel, cx: &mut Context<Self>| {
            let is_voice = channel.is_voice();
            let voice = self.voice.read(cx);
            let selected = if is_voice {
                voice.active(&channel.id)
            } else {
                self.selected_channel.as_ref() == Some(&channel.id)
            };
            let joinable = is_voice && voice.can_join(self.session.read(cx), &channel.id);
            let muted = self
                .notifications
                .read(cx)
                .saved
                .is_muted(&channel.id, notifications::now());
            let unread = snapshot.and_then(|snapshot| read::unread_badge(snapshot, channel));
            let context_channel = channel.id.clone();
            let channel_id = channel.id.clone();
            let name = channel_label(channel, snapshot);
            let icon = channel_icon(channel);
            let mut label = div()
                .flex()
                .items_center()
                .gap(px(theme.space2))
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(icon.render(theme.text_muted, 18.)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(name.replace(['\r', '\n'], " ")),
                );
            if muted {
                label = label.child(
                    div()
                        .flex_shrink_0()
                        .child(Icon::BellOff.render(theme.text_muted, 18.)),
                );
            }
            if unread.is_some() {
                label = label.font_weight(FontWeight::BOLD);
            }
            if let Some(mentions) = unread {
                label = if muted || mentions == 0 {
                    label.child(
                        div()
                            .size(px(7.))
                            .flex_shrink_0()
                            .rounded(px(theme.radius_pill))
                            .bg(theme.text),
                    )
                } else {
                    label.child(
                        div()
                            .min_w(px(18.))
                            .px(px(5.))
                            .py(px(2.))
                            .flex_shrink_0()
                            .rounded(px(theme.radius_sm))
                            .text_size(px(theme.font_size_xs))
                            .bg(theme.danger)
                            .text_color(theme.danger_text)
                            .child(mentions.to_string()),
                    )
                };
            }
            let row = Button::new(
                ElementId::Name(format!("channel-{channel_id}").into()),
                theme,
            )
            .variant(ButtonVariant::Channel)
            .muted(muted)
            .on_right_click(
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_channel_menu(context_channel.clone(), event.position, window, cx)
                }),
            )
            .selected(selected)
            .leading(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if joinable {
                    this.drawer = None;
                    this.join_voice(&channel_id, cx);
                } else if !is_voice {
                    this.select_text_view(channel_id.clone(), cx);
                }
            }));
            div().child(row).when_some(
                rosters
                    .get(channel.id.as_str())
                    .filter(|entries| !entries.is_empty()),
                |row, participants| {
                    row.child(roster::render(
                        &channel.id,
                        participants,
                        &self.images,
                        &self.voice,
                        theme,
                        cx,
                    ))
                },
            )
        };

        let mut channel_list = div()
            .id("channel-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap(px(3.))
            .py(px(theme.space3))
            .pb(px(theme.space3) + bottom_inset)
            .px(px(theme.space2))
            .overflow_y_scroll();
        for channel in &uncategorized {
            channel_list = channel_list.child(channel_row(channel, cx));
        }
        if let Some(server) = active_server {
            for category in &server.categories {
                let category_channels: Vec<&&ClientChannel> = category
                    .channels
                    .iter()
                    .filter_map(|id| channels.iter().find(|channel| &channel.id == id))
                    .collect();
                if category_channels.is_empty() {
                    continue;
                }
                let group_id = category.id.clone();
                let collapsed = self.collapsed_channels.contains(&group_id);
                channel_list = channel_list.child(
                    Button::new(
                        ElementId::Name(format!("channel-group-{group_id}").into()),
                        theme,
                    )
                    .variant(ButtonVariant::Heading)
                    .accessibility_label(category.title.clone())
                    .leading(motion::disclosure(
                        format!("channel-disclosure-{group_id}"),
                        collapsed,
                        theme.text_muted,
                        12.,
                        category_channels.iter().any(|channel| channel.is_voice()),
                    ))
                    .trailing(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(category.title.replace(['\r', '\n'], " ")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed_channels.remove(&group_id) {
                            this.collapsed_channels.insert(group_id.clone());
                        }
                        cx.notify();
                    })),
                );
                if collapsed {
                    continue;
                }
                for channel in category_channels {
                    channel_list = channel_list.child(channel_row(channel, cx));
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .flex_shrink_0()
            .h_full()
            .bg(theme.sidebar)
            .when(!self.focused, |sidebar| {
                sidebar.child(
                    div()
                        .h(px(HEADER_HEIGHT))
                        .flex_shrink_0()
                        .px(px(theme.space4))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .font_weight(FontWeight::BOLD)
                                .child(t("shell.server").to_uppercase()),
                        )
                        .child(
                            div()
                                .text_size(px(theme.font_size_md))
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .child(
                                    active_server
                                        .map(|server| server.name.replace(['\r', '\n'], " "))
                                        .unwrap_or_else(|| t("shell.messages")),
                                ),
                        ),
                )
            })
            .child(channel_list)
            .children(dock)
    }
}

#[cfg(test)]
mod tests {
    use super::{server_has_voice_activity, server_unread_count, unread_label};
    use serde_json::json;
    use stonks_core::channel_preferences::{Change, ChannelPreferences, NotificationMode};
    use stonks_core::connection::{AppSnapshot, ClientServer};

    fn snapshot() -> AppSnapshot {
        AppSnapshot {
            current_user: serde_json::from_value(json!({"_id":"me","username":"me"})).unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: vec![],
        }
    }

    #[test]
    fn rail_counts_unread_viewable_unmuted_channels_not_mentions() {
        let mut snapshot = snapshot();
        snapshot.channels = serde_json::from_value(json!([
            {"_id":"unread","channel_type":"TextChannel","server":"s","last_message_id":"b"},
            {"_id":"muted","channel_type":"TextChannel","server":"s","last_message_id":"b"},
            {"_id":"hidden","channel_type":"TextChannel","server":"s","last_message_id":"b"},
            {"_id":"read","channel_type":"TextChannel","server":"s","last_message_id":"b"},
            {"_id":"elsewhere","channel_type":"TextChannel","server":"other","last_message_id":"b"}
        ]))
        .unwrap();
        snapshot.unreads = serde_json::from_value(json!([
            {"_id":{"channel":"unread","user":"me"},"last_id":"a","mentions":["b","c"]},
            {"_id":{"channel":"read","user":"me"},"last_id":"b","mentions":["c"]}
        ]))
        .unwrap();
        let mut prefs = ChannelPreferences::default();
        prefs.apply("muted", Change::Mute(Some(Some(100))));
        prefs.apply(
            "unread",
            Change::Notifications(Some(NotificationMode::None)),
        );
        let count = |now| server_unread_count(&snapshot, "s", &prefs, now, |id| id != "hidden");
        assert_eq!(count(99), 1);
        assert_eq!(count(100), 2);
        assert_eq!(unread_label(0), None);
        assert_eq!(unread_label(9).as_deref(), Some("9"));
        assert_eq!(unread_label(10).as_deref(), Some("9+"));
    }

    #[test]
    fn rail_shows_gateway_voice_even_when_only_the_same_account_is_connected() {
        let mut snapshot = snapshot();
        let server: ClientServer = serde_json::from_value(json!({
            "_id":"s","name":"Server","owner":"me","channels":["room"]
        }))
        .unwrap();
        snapshot.voice_states = serde_json::from_value(json!([
            {"id":"room","participants":[{"id":"me"}]},
            {"id":"other-room","participants":[{"id":"other"}]}
        ]))
        .unwrap();
        assert!(server_has_voice_activity(&snapshot, &server, |_| false));
        assert!(server_has_voice_activity(&snapshot, &server, |id| id == "room"));
        snapshot.voice_states[0].participants[0].id = "other".into();
        assert!(server_has_voice_activity(&snapshot, &server, |_| false));
        snapshot.voice_states.clear();
        assert!(!server_has_voice_activity(&snapshot, &server, |_| false));
        assert!(server_has_voice_activity(&snapshot, &server, |id| id == "room"));
    }
}
