//! Read intent coordination. A painted, foreground-visible message can start
//! an ack, including while reading older history. Scope changes cancel owned
//! work; newer visible targets coalesce without acknowledging unseen messages.
use stonks_core::{
    connection::AppSnapshot,
    messages::{normalize::valid_id, read},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub generation: u64,
    pub channel: String,
    pub message: String,
    pub user: String,
}
#[derive(Default)]
pub(super) struct ReadState {
    generation: u64,
    view: Option<(String, String)>,
    pending: Option<String>,
    inflight: Option<Target>,
    failed: Option<String>,
    manual_unread: Option<String>,
}
pub(super) struct Effect {
    pub cancel: bool,
    pub start: Option<Target>,
}
impl ReadState {
    pub fn reset(&mut self) {
        self.manual_unread = None;
        self.reset_view();
    }

    pub fn mark_unread(&mut self, channel: &str) {
        self.reset_view();
        self.manual_unread = Some(channel.into());
    }

    pub fn suppresses(&self, channel: &str) -> bool {
        self.manual_unread.as_deref() == Some(channel)
    }

    fn reset_view(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.view = None;
        self.pending = None;
        self.inflight = None;
        self.failed = None;
    }
    pub fn observe(
        &mut self,
        snapshot: Option<&AppSnapshot>,
        channel: Option<&str>,
        latest: Option<&str>,
        foreground: bool,
    ) -> Effect {
        if channel.is_some_and(|channel| !self.suppresses(channel)) {
            self.manual_unread = None;
        }
        let view = if foreground && !channel.is_some_and(|channel| self.suppresses(channel)) {
            snapshot.and_then(|snapshot| {
                channel
                    .filter(|channel| {
                        snapshot
                            .channels
                            .iter()
                            .any(|entry| entry.id == *channel && !entry.is_voice())
                    })
                    .map(|channel| (channel.to_owned(), snapshot.current_user.id.clone()))
            })
        } else {
            None
        };
        let cancel = self.view != view;
        if cancel {
            self.reset_view();
            self.view = view;
        }
        if let (Some(snapshot), Some((channel, _)), Some(message)) = (snapshot, &self.view, latest)
        {
            if valid_id(channel)
                && valid_id(message)
                && read::cursor(snapshot, channel).is_none_or(|old| old < message)
                && self
                    .inflight
                    .as_ref()
                    .is_none_or(|target| target.message.as_str() < message)
            {
                if self.pending.as_deref().is_none_or(|old| old < message) {
                    self.pending = Some(message.into());
                }
            }
        }
        Effect {
            cancel,
            start: snapshot.and_then(|snapshot| self.next(snapshot)),
        }
    }
    pub fn next(&mut self, snapshot: &AppSnapshot) -> Option<Target> {
        if self.inflight.is_some() {
            return None;
        }
        let (channel, user) = self.view.as_ref()?;
        if &snapshot.current_user.id != user {
            return None;
        }
        let message = self.pending.as_ref()?;
        if read::cursor(snapshot, channel).is_some_and(|old| old >= message.as_str()) {
            self.pending = None;
            return None;
        }
        if self.failed.as_deref() == Some(message.as_str()) {
            return None;
        }
        let target = Target {
            generation: self.generation,
            channel: channel.clone(),
            user: user.clone(),
            message: self.pending.take()?,
        };
        self.inflight = Some(target.clone());
        Some(target)
    }
    pub fn complete(&mut self, target: &Target, success: bool) -> bool {
        if self.inflight.as_ref() != Some(target) || target.generation != self.generation {
            return false;
        }
        self.inflight = None;
        if success {
            self.failed = None;
        } else {
            self.failed = Some(target.message.clone());
            if self
                .pending
                .as_deref()
                .is_none_or(|pending| pending < target.message.as_str())
            {
                self.pending = Some(target.message.clone());
            }
        }
        true
    }
    pub fn retry(&mut self, snapshot: &AppSnapshot) -> Option<Target> {
        self.failed = None;
        self.next(snapshot)
    }

    pub fn clear_confirmed_failure(&mut self, snapshot: &AppSnapshot) -> bool {
        let confirmed = self.view.as_ref().zip(self.failed.as_deref()).is_some_and(
            |((channel, user), failed)| {
                user == &snapshot.current_user.id
                    && read::cursor(snapshot, channel).is_some_and(|cursor| cursor >= failed)
            },
        );
        if confirmed {
            self.failed = None;
        }
        confirmed
    }
}

/// Explicit section reads outlive navigation, unlike painted-message intents.
#[derive(Default)]
pub(super) struct SectionReads {
    queued: std::collections::BTreeMap<String, String>,
    active: Option<(String, String, super::ReadRequest)>,
}

impl SectionReads {
    pub(super) fn active_for(&self, channel: &str) -> bool {
        self.active.as_ref().is_some_and(|(id, _, _)| id == channel)
    }

    pub(super) fn cancel(&mut self, channel: &str) {
        self.queued.remove(channel);
        if self.active_for(channel) {
            self.active = None;
        }
    }
}

fn section_targets<'a>(
    snapshot: &'a AppSnapshot,
    server: Option<&'a str>,
    can_view: impl Fn(&str) -> bool + 'a,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    snapshot.channels.iter().filter_map(move |channel| {
        let id = channel.id.as_str();
        let head = channel.last_message_id.as_deref()?;
        (channel.server.as_deref() == server
            && (server.is_some() || channel.channel_type != "SavedMessages")
            && valid_id(id)
            && valid_id(head)
            && read::cursor(snapshot, id).is_none_or(|seen| seen < head)
            && can_view(id))
        .then_some((id, head))
    })
}

impl super::Session {
    pub(crate) fn mark_section_read(
        &mut self,
        server: Option<&str>,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let targets: Vec<_> = section_targets(snapshot, server, |id| self.can_view_channel(id))
            .map(|(id, head)| (id.to_owned(), head.to_owned()))
            .collect();
        for (id, head) in targets {
            if self
                .section_reads
                .active
                .as_ref()
                .is_some_and(|(active, sent, _)| active == &id && sent >= &head)
            {
                continue;
            }
            self.section_reads
                .queued
                .entry(id)
                .and_modify(|queued| {
                    if head > *queued {
                        queued.clone_from(&head);
                    }
                })
                .or_insert(head);
        }
        self.start_section_read(cx);
    }

    pub(super) fn start_section_read(&mut self, cx: &mut gpui_kit::Context<Self>) {
        use super::{spawn_core, ReadRequest};
        use futures_util::future::{AbortHandle, Abortable};

        if self.section_reads.active.is_some() {
            return;
        }
        let Some((policy, context)) = self.account_context() else {
            self.section_reads.queued.clear();
            return;
        };
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let blocked = self
            .read_state
            .inflight
            .as_ref()
            .map(|target| target.channel.as_str());
        let (channel, message) = loop {
            let Some(id) = self
                .section_reads
                .queued
                .keys()
                .find(|id| Some(id.as_str()) != blocked)
                .cloned()
            else {
                return;
            };
            let Some(message) = self.section_reads.queued.remove(&id) else {
                return;
            };
            // Never substitute a newer head that arrived after the keypress.
            if snapshot.channels.iter().any(|channel| {
                channel.id == id
                    && channel
                        .last_message_id
                        .as_ref()
                        .is_some_and(|head| head >= &message)
            }) && self.can_view_channel(&id)
                && read::cursor(snapshot, &id).is_none_or(|seen| seen < message.as_str())
            {
                break (id, message);
            }
        };
        let user = snapshot.current_user.id.clone();
        let (request_channel, request_message) = (channel.clone(), message.clone());
        let (active_channel, active_message) = (channel.clone(), message.clone());
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let request_context = context.clone();
            let request_policy = policy.clone();
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    read::acknowledge_in_context(
                        &request_policy,
                        &request_context,
                        &request_channel,
                        &request_message,
                    )
                    .await
                },
                registration,
            ))
            .await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.section_reads.active = None;
                if result.is_ok()
                    && policy.is_authenticated_context_current(&context)
                    && this.can_view_channel(&channel)
                {
                    if let Some(snapshot) = this
                        .snapshot
                        .as_mut()
                        .filter(|snapshot| snapshot.current_user.id == user)
                    {
                        read::advance(snapshot, &channel, &message);
                    }
                }
                this.start_section_read(cx);
                cx.notify();
            });
        });
        self.section_reads.active = Some((
            active_channel,
            active_message,
            ReadRequest { abort, _task: task },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
    fn id(n: u8) -> String {
        format!("{n:026}")
    }
    fn snapshot() -> AppSnapshot {
        AppSnapshot {
            current_user: serde_json::from_value(serde_json::json!({"_id":"me","username":"Me"}))
                .unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![serde_json::from_value(
                serde_json::json!({"_id":CHANNEL,"channel_type":"TextChannel"}),
            )
            .unwrap()],
            members: vec![],
            unreads: vec![],
            voice_states: vec![],
        }
    }
    #[test]
    fn section_targets_match_tauri_scope_and_ignore_unread_mentions_without_a_head() {
        let mut snapshot = snapshot();
        let head = id(20);
        snapshot.channels.clear();
        let mut add =
            |n, kind, server, last: Option<&str>| {
                snapshot.channels.push(serde_json::from_value(serde_json::json!({
                "_id": id(n), "channel_type": kind, "server": server, "last_message_id": last
            })).unwrap());
            };
        add(1, "TextChannel", Some("current"), Some(&head));
        add(2, "VoiceChannel", Some("current"), Some(&head));
        add(3, "TextChannel", Some("other"), Some(&head));
        add(4, "TextChannel", Some("current"), Some(&head));
        add(5, "TextChannel", Some("current"), Some(&head));
        add(6, "DirectMessage", None, Some(&head));
        add(7, "SavedMessages", None, Some(&head));
        add(8, "Group", None, None);
        add(9, "Group", None, Some("invalid"));
        read::advance(&mut snapshot, &id(5), &head);
        read::advance(&mut snapshot, &id(8), &id(10));
        snapshot
            .unreads
            .last_mut()
            .unwrap()
            .mentions
            .push(head.clone());

        let hidden = id(4);
        let targets = |server| {
            section_targets(&snapshot, server, |id| id != hidden)
                .map(|(id, message)| (id.to_owned(), message.to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            targets(Some("current")),
            [(id(1), head.clone()), (id(2), head.clone())]
        );
        assert_eq!(targets(None), [(id(6), head)]);
    }

    #[test]
    fn manual_unread_cancels_old_intent_and_survives_render_and_blur() {
        let mut snapshot = snapshot();
        let other = id(1);
        let mut channel = snapshot.channels[0].clone();
        channel.id = other.clone();
        snapshot.channels.push(channel);
        let newest = id(9);
        let mut state = ReadState::default();
        let old = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&newest), true)
            .start
            .unwrap();
        state.mark_unread(CHANNEL);
        assert!(!state.complete(&old, true));
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&newest), true)
            .start
            .is_none());
        state.observe(Some(&snapshot), None, None, false);
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&newest), true)
            .start
            .is_none());
        state.observe(Some(&snapshot), Some(&other), None, true);
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&newest), true)
            .start
            .is_some());
    }
    #[test]
    fn voice_channels_never_start_read_intents() {
        let mut snapshot = snapshot();
        let mut state = ReadState::default();
        snapshot.channels[0].voice = Some(serde_json::json!({}));
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .is_none());
        snapshot.channels[0].voice = None;
        snapshot.channels[0].channel_type = "VoiceChannel".into();
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .is_none());
    }

    #[test]
    fn background_and_unmeasured_rows_never_ack() {
        let snapshot = snapshot();
        let mut state = ReadState::default();
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), false)
            .start
            .is_none());
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), None, true)
            .start
            .is_none());
        assert!(state
            .observe(Some(&snapshot), Some("missing"), Some(&id(1)), true)
            .start
            .is_none());
    }
    #[test]
    fn visible_history_acknowledges_seen_message_without_advancing_to_unseen_channel_latest() {
        let mut snapshot = snapshot();
        snapshot.channels[0].last_message_id = Some(id(9));
        let mut state = ReadState::default();
        let seen = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(3)), true)
            .start
            .unwrap();
        assert_eq!(seen.message, id(3));
        assert!(state.complete(&seen, true));
        read::advance(&mut snapshot, CHANNEL, &seen.message);
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(2)), true)
            .start
            .is_none());
        assert_eq!(read::cursor(&snapshot, CHANNEL), Some(id(3).as_str()));
    }
    #[test]
    fn duplicate_observations_noop_and_newer_visible_messages_coalesce() {
        let mut snapshot = snapshot();
        let mut state = ReadState::default();
        let first = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .unwrap();
        let duplicate = state.observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true);
        assert!(!duplicate.cancel && duplicate.start.is_none());
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(2)), true)
            .start
            .is_none());
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(3)), true)
            .start
            .is_none());
        assert!(state.complete(&first, true));
        read::advance(&mut snapshot, CHANNEL, &first.message);
        let next = state.next(&snapshot).unwrap();
        assert_eq!(next.message, id(3));
    }
    #[test]
    fn replacement_and_logout_reject_stale_completions() {
        let snapshot = snapshot();
        let mut state = ReadState::default();
        let first = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .unwrap();
        assert!(
            state
                .observe(Some(&snapshot), Some(CHANNEL), None, false)
                .cancel
        );
        let second = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(2)), true)
            .start
            .unwrap();
        assert!(!state.complete(&first, true));
        state.reset();
        assert!(!state.complete(&second, true));
    }
    #[test]
    fn failure_preserves_unread_and_requires_explicit_retry_or_newer_target() {
        let snapshot = snapshot();
        let mut state = ReadState::default();
        let first = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .unwrap();
        assert!(state.complete(&first, false));
        assert!(read::cursor(&snapshot, CHANNEL).is_none());
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .is_none());
        assert_eq!(state.retry(&snapshot).unwrap().message, id(1));
    }

    #[test]
    fn later_remote_confirmation_clears_a_failed_ack_without_resending() {
        let mut snapshot = snapshot();
        let mut state = ReadState::default();
        let target = state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .unwrap();
        state.complete(&target, false);
        read::advance(&mut snapshot, CHANNEL, &id(2));
        assert!(state.clear_confirmed_failure(&snapshot));
        assert!(state.next(&snapshot).is_none());
        assert!(state
            .observe(Some(&snapshot), Some(CHANNEL), Some(&id(1)), true)
            .start
            .is_none());
    }
}
