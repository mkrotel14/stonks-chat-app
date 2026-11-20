//! Send eligibility and revision-checked completion of text and attachment sends.
use super::{attachments::route::SendFailure, emoji::tokens, Composer, Request, Sent};
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::{Context, Window};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use stonks_core::messages::{models::SendMessageRequest, service};

impl Composer {
    pub(super) fn can_send(&self) -> bool {
        self.state.can_send()
            && self
                .attachment_access
                .as_ref()
                .is_some_and(|access| access.can_send)
            && (self.state.files().is_empty()
                || (self.picker.is_none()
                    && self.attachment_route.is_some()
                    && self.upload_allowed()
                    && self
                        .state
                        .files()
                        .iter()
                        .all(|file| file.payload.bytes.len() as u64 <= self.attachment_limit())))
    }

    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Synchronize text in case a widget edit notification is still queued.
        self.state.edit(self.input.read(cx).value().to_string());
        if !self.can_send() {
            return;
        }
        let content =
            super::mentions::serialize(self.state.value(), self.state.draft_mentions(), |text| {
                tokens::serialize(text, &self.emoji.custom)
            });

        if let Some(attempt) = self.state.begin_music(&content) {
            self.mentions.open = false;
            self.stop_typing(cx);
            cx.notify();
            let unavailable = || stonks_ui::i18n::t("messages.musicQueueUnavailable");
            let request = self
                .music_route
                .as_ref()
                .ok_or_else(unavailable)
                .and_then(|route| route(attempt.url.clone(), cx));
            cx.spawn_in(window, async move |this, cx| {
                let (is_current, result) = match request {
                    Ok(submission) => (
                        Some(submission.is_current),
                        submission
                            .result
                            .await
                            .unwrap_or_else(|_| Err(unavailable())),
                    ),
                    Err(error) => (None, Err(error)),
                };
                let _ = this.update_in(cx, |this, window, cx| {
                    this.state.edit(this.input.read(cx).value().to_string());
                    let current = is_current.is_none_or(|check| check(cx));
                    if this
                        .state
                        .complete_music(attempt, current.then_some(result))
                    {
                        this.spellcheck.invalidate();
                        this.input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.refresh_placeholder(window, cx);
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }

        let route = if self.state.files().is_empty() {
            None
        } else {
            let Some(route) = self.attachment_route.clone() else {
                return;
            };
            Some(route)
        };
        let Some(attempt) = self.state.begin_with_content(content, new_key) else {
            cx.notify();
            return;
        };
        self.mentions.open = false;
        self.stop_typing(cx);
        cx.notify();
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let id = attempt.id;
        let channel = attempt.channel.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    if let Some(route) = route {
                        return route(attempt).await;
                    }
                    service::send_message(
                        &policy,
                        SendMessageRequest {
                            channel_id: attempt.channel,
                            content: Some(attempt.content),
                            attachments: Vec::new(),
                            reply_to: attempt.reply_to,
                            idempotency_key: attempt.key,
                        },
                    )
                    .await
                    .map_err(|error| SendFailure {
                        message: error.to_string(),
                        uploaded: Vec::new(),
                    })
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if !this.requests.contains_key(&id) {
                    return;
                }
                this.state.edit(this.input.read(cx).value().to_string());
                let (error, uploaded, message) = match result {
                    Ok(message) => (None, Vec::new(), Some(message)),
                    Err(error) => (Some(error.message), error.uploaded, None),
                };
                if let Some(completion) = this.state.complete(&channel, id, error, uploaded) {
                    if completion.cleared
                        && this.state.active_channel() == Some(completion.channel.as_str())
                    {
                        this.spellcheck.invalidate();
                        this.input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.refresh_placeholder(window, cx);
                    }
                    if let Some(message) = message {
                        cx.emit(Sent(message));
                    }
                }
                this.requests.remove(&id);
                cx.notify();
            })
            .ok();
        });
        self.requests.insert(id, Request { abort, _task: task });
    }
}

fn new_key() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
