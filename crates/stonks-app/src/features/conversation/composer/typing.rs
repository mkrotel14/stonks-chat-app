//! Local and remote typing state and timers. Timestamps are monotonic milliseconds.
use super::Composer;
use gpui_kit::Context;
use std::time::Duration;

pub const IDLE_MS: u64 = 3_000;
const REFRESH_MS: u64 = 2_000;
pub const REMOTE_EXPIRY_MS: u64 = 8_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typing {
    pub channel_id: String,
    pub typing: bool,
}

#[derive(Default)]
pub struct TypingState {
    channel: Option<String>,
    last_edit: u64,
    last_start: u64,
}
impl TypingState {
    pub fn edit(&mut self, channel: &str, text: &str, now: u64) -> Vec<Typing> {
        let mut events = Vec::new();
        if self.channel.as_deref() != Some(channel) || text.trim().is_empty() {
            events.extend(self.stop());
        }
        if text.trim().is_empty() {
            return events;
        }
        if self.channel.is_none() || now.saturating_sub(self.last_start) >= REFRESH_MS {
            events.push(Typing {
                channel_id: channel.into(),
                typing: true,
            });
            self.last_start = now;
        }
        self.channel = Some(channel.into());
        self.last_edit = now;
        events
    }
    pub fn stop(&mut self) -> Option<Typing> {
        self.channel.take().map(|channel_id| Typing {
            channel_id,
            typing: false,
        })
    }
    pub fn expire(&mut self, now: u64) -> Option<Typing> {
        (now.saturating_sub(self.last_edit) >= IDLE_MS)
            .then(|| self.stop())
            .flatten()
    }
}

#[derive(Default)]
pub struct RemoteTyping {
    users: std::collections::HashMap<(String, String), (String, u64)>,
}
impl RemoteTyping {
    pub fn update(&mut self, channel: &str, user: &str, name: &str, typing: bool, now: u64) {
        if channel.is_empty() || user.is_empty() {
            return;
        }
        let key = (channel.to_owned(), user.to_owned());
        if typing {
            self.users.insert(key, (name.to_owned(), now));
        } else {
            self.users.remove(&key);
        }
    }
    pub fn expire(&mut self, now: u64) {
        self.users
            .retain(|_, (_, seen)| now.saturating_sub(*seen) < REMOTE_EXPIRY_MS);
    }
    pub fn deadline(&self) -> Option<u64> {
        self.users
            .values()
            .map(|(_, seen)| seen.saturating_add(REMOTE_EXPIRY_MS))
            .min()
    }
    pub fn names(&self, channel: &str, now: u64) -> Vec<String> {
        let mut names: Vec<_> = self
            .users
            .iter()
            .filter(|((id, _), (_, seen))| {
                id == channel && now.saturating_sub(*seen) < REMOTE_EXPIRY_MS
            })
            .map(|((_, id), (name, _))| (name.clone(), id.clone()))
            .collect();
        names.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.1.cmp(&b.1))
        });
        names.into_iter().map(|(name, _)| name).collect()
    }
    pub fn clear(&mut self) {
        self.users.clear();
    }
}

impl Composer {
    /// The host enables this only after applying the account privacy setting.
    pub fn set_typing_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.typing_enabled = enabled;
        if !enabled {
            self.stop_typing(cx);
        }
    }

    /// The host resolves labels and excludes the signed-in user. A Message
    /// event from the same author should call this with `typing = false`.
    pub fn apply_remote_typing(
        &mut self,
        channel: &str,
        user: &str,
        name: &str,
        typing: bool,
        cx: &mut Context<Self>,
    ) {
        self.remote_typing
            .update(channel, user, name, typing, self.now());
        self.schedule_remote_expiry(cx);
        cx.notify();
    }

    fn schedule_remote_expiry(&mut self, cx: &mut Context<Self>) {
        self.remote_timer = None;
        let Some(deadline) = self.remote_typing.deadline() else {
            return;
        };
        let delay = deadline.saturating_sub(self.now()).max(1);
        self.remote_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            this.update(cx, |this, cx| {
                this.remote_typing.expire(this.now());
                this.schedule_remote_expiry(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn now(&self) -> u64 {
        self.typing_clock
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    pub(super) fn changed(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if self.state.value() != text {
            self.spellcheck.invalidate();
            self.mentions.open = true;
            self.mentions.selected = 0;
        }
        self.state.edit(text.clone());
        if !self.typing_enabled {
            self.stop_typing(cx);
            return;
        }
        let Some(channel) = self.state.active_channel() else {
            return;
        };
        for event in self.typing.edit(channel, &text, self.now()) {
            cx.emit(event);
        }
        self.typing_timer = None;
        if text.trim().is_empty() {
            return;
        }
        self.typing_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(IDLE_MS))
                .await;
            this.update(cx, |this, cx| {
                if let Some(event) = this.typing.expire(this.now()) {
                    cx.emit(event);
                }
            })
            .ok();
        }));
    }

    pub(super) fn stop_typing(&mut self, cx: &mut Context<Self>) {
        self.typing_timer = None;
        if let Some(event) = self.typing.stop() {
            cx.emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RemoteTyping, Typing, TypingState};
    #[test]
    fn typing_refreshes_without_sending_a_packet_for_every_key() {
        let mut state = TypingState::default();
        assert_eq!(state.edit("a", "h", 0).len(), 1);
        assert!(state.edit("a", "he", 100).is_empty());
        assert_eq!(
            state.edit("a", "hello", 2_000),
            vec![Typing {
                channel_id: "a".into(),
                typing: true
            }]
        );
        assert!(state.expire(3_000).is_none());
        assert_eq!(
            state.expire(5_000),
            Some(Typing {
                channel_id: "a".into(),
                typing: false
            })
        );
        assert!(state.expire(6_000).is_none());
    }
    #[test]
    fn clearing_or_switching_channels_stops_the_previous_channel() {
        let mut state = TypingState::default();
        state.edit("a", "hello", 0);
        assert_eq!(
            state.edit("b", "draft", 10),
            vec![
                Typing {
                    channel_id: "a".into(),
                    typing: false
                },
                Typing {
                    channel_id: "b".into(),
                    typing: true
                }
            ]
        );
        assert_eq!(
            state.edit("b", " ", 20),
            vec![Typing {
                channel_id: "b".into(),
                typing: false
            }]
        );
        assert!(state.stop().is_none());
    }
    #[test]
    fn remote_users_deduplicate_refresh_and_expire_per_channel() {
        let mut remote = RemoteTyping::default();
        remote.update("a", "u1", "Alice", true, 0);
        remote.update("a", "u1", "Alice", true, 1_000);
        remote.update("b", "u2", "Bob", true, 0);
        assert_eq!(remote.names("a", 1_001), ["Alice"]);
        remote.expire(8_000);
        assert!(remote.names("b", 8_000).is_empty());
        assert_eq!(remote.names("a", 8_000), ["Alice"]);
        assert_eq!(remote.deadline(), Some(9_000));
        remote.update("a", "u1", "Alice", false, 8_001);
        assert!(remote.names("a", 8_001).is_empty());
        assert!(remote.deadline().is_none());
    }
}
