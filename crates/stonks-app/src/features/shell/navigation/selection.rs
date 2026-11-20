//! Keep server and text channel selection in sync with the session.

use super::super::*;
use stonks_core::connection::{ClientChannel, ClientServer};

impl ShellView {
    /// Keep selection valid and sticky: reset only when the selection
    /// vanished from the snapshot, then sync the timeline. Called whenever
    /// the session snapshot changes and on user navigation.
    pub(in crate::features::shell) fn ensure_selection(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.session.read(cx).snapshot.clone();
        let servers: &[ClientServer] = snapshot.as_ref().map_or(&[], |snapshot| &snapshot.servers);
        if self.focused {
            use super::super::compact::Route;
            let (server, channel) = match self.compact_nav.route() {
                Route::Conversation(channel) => (None, Some(channel)),
                Route::Channels(server) => (Some(server), None),
                Route::Chat(server, channel) => (Some(server), Some(channel)),
                _ => (None, None),
            };
            self.home = server.is_none();
            self.selected_server = server
                .filter(|id| servers.iter().any(|server| server.id == *id))
                .map(str::to_owned);
            self.selected_channel = channel
                .filter(|id| {
                    snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.channels.iter().any(|channel| {
                            channel.id == *id
                                && !channel.is_voice()
                                && channel.server.as_deref() == server
                        })
                    }) && self.session.read(cx).can_view_channel(id)
                })
                .map(str::to_owned);
        } else {
            if self
                .selected_server
                .as_ref()
                .is_some_and(|id| !servers.iter().any(|server| &server.id == id))
            {
                self.selected_server = None;
            }
            if self.selected_server.is_none() && !self.home {
                self.selected_server = servers.first().map(|server| server.id.clone());
            }

            let channels: Vec<&ClientChannel> = match (snapshot.as_ref(), &self.selected_server) {
                (Some(snapshot), Some(server_id)) => snapshot
                    .servers
                    .iter()
                    .find(|server| &server.id == server_id)
                    .map(|server| {
                        server
                            .channels
                            .iter()
                            .filter_map(|id| {
                                snapshot.channels.iter().find(|channel| &channel.id == id)
                            })
                            .filter(|channel| self.session.read(cx).can_view_channel(&channel.id))
                            .collect()
                    })
                    .unwrap_or_default(),
                (Some(snapshot), None) if self.home => snapshot
                    .channels
                    .iter()
                    .filter(|channel| {
                        channel.server.is_none()
                            && self.session.read(cx).can_view_channel(&channel.id)
                    })
                    .collect(),
                _ => Vec::new(),
            };
            self.selected_channel = if self.home {
                self.selected_channel.take().filter(|id| {
                    channels
                        .iter()
                        .any(|channel| channel.id == *id && !channel.is_voice())
                })
            } else {
                select_text_channel(&channels, self.selected_channel.as_deref())
                    .map(|channel| channel.id.clone())
            };
        }
        if let Some(channel_id) = self.selected_channel.clone() {
            self.timeline
                .update(cx, |timeline, cx| timeline.set_channel(channel_id, cx));
        } else {
            self.timeline.update(cx, |timeline, cx| timeline.clear(cx));
        }
        if let Some(server_id) = self.selected_server.clone() {
            self.session
                .update(cx, |session, cx| session.load_members(server_id, false, cx));
        }
        cx.notify();
    }

    pub(in crate::features::shell) fn select_server(
        &mut self,
        server_id: String,
        cx: &mut Context<Self>,
    ) {
        self.drawer = None;
        self.home = false;
        self.selected_server = Some(server_id);
        self.selected_channel = None;
        self.ensure_selection(cx);
    }

    pub(in crate::features::shell) fn navigate_mentioned_channel(
        &mut self,
        channel_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.session.read(cx).can_view_channel(&channel_id) {
            return;
        }
        let channel = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
            })
            .cloned();
        let Some(channel) = channel else { return };
        if channel.is_voice() {
            self.join_voice(&channel_id, cx);
            return;
        }
        self.home = channel.server.is_none();
        self.selected_server = channel.server;
        self.select_text_view(channel_id, cx);
    }

    pub(in crate::features::shell) fn join_voice(&mut self, id: &str, cx: &mut Context<Self>) {
        self.voice.update(cx, |voice, cx| {
            voice.join(id, &self.session, self.policy.clone(), cx);
        });
    }

    pub(in crate::features) fn suspend_voice(&mut self, suspended: bool, cx: &mut Context<Self>) {
        self.voice
            .update(cx, |voice, cx| voice.set_suspended(suspended, cx));
    }

    pub(in crate::features::shell) fn select_text_view(
        &mut self,
        channel_id: String,
        cx: &mut Context<Self>,
    ) {
        self.drawer = None;
        if !self.focused && self.home {
            self.wide_friends = false;
        }
        if self.focused {
            self.compact_nav
                .open_chat(self.selected_server.clone(), channel_id.clone());
        }
        self.selected_channel = Some(channel_id);
        cx.notify();
        self.ensure_selection(cx);
    }
}

fn select_text_channel<'a>(
    channels: &[&'a ClientChannel],
    selected: Option<&str>,
) -> Option<&'a ClientChannel> {
    let mut text = channels
        .iter()
        .copied()
        .filter(|channel| !channel.is_voice());
    selected
        .and_then(|id| text.clone().find(|channel| channel.id == id))
        .or_else(|| text.next())
}

#[cfg(test)]
mod tests {
    use super::{ClientChannel, select_text_channel};

    #[test]
    fn text_selection_is_sticky_and_never_falls_back_to_voice() {
        let channel = |id, kind, voice: serde_json::Value| {
            serde_json::from_value::<ClientChannel>(
                serde_json::json!({"_id": id, "channel_type": kind, "voice": voice}),
            )
            .unwrap()
        };
        let voice = channel("voice", "VoiceChannel", serde_json::Value::Null);
        let configured = channel("configured", "TextChannel", serde_json::json!({}));
        let general = channel("general", "TextChannel", serde_json::Value::Null);
        let other = channel("other", "TextChannel", serde_json::Value::Null);
        let channels = [&voice, &configured, &general, &other];
        assert!(select_text_channel(&channels[..2], None).is_none());
        for selected in [None, Some("voice"), Some("configured"), Some("missing")] {
            assert_eq!(
                select_text_channel(&channels, selected).unwrap().id,
                "general"
            );
        }
        assert_eq!(
            select_text_channel(&channels, Some("other")).unwrap().id,
            "other"
        );
    }
}
