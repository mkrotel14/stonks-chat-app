//! Channel-owned editor drafts, reply targets, and editor focus.
use super::{state::ReplyDraft, Composer};
#[cfg(debug_assertions)]
use gpui_kit::{component::input::TextareaState, Entity};
use gpui_kit::{Context, Focusable, Window};
use stonks_core::messages::{models::TimelineMessage, normalize::valid_id};
use stonks_ui::i18n::t;

impl Composer {
    pub(crate) fn editor_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(crate) fn shortcut_popup_open(&self) -> bool {
        self.emoji.open || self.picker.is_some()
    }

    /// Channel navigation keeps drafts. Use `reset` on logout/account changes.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.active_channel().is_none() {
            return;
        }
        self.spellcheck.invalidate();
        self.mentions = Default::default();
        self.stop_typing(cx);
        self.state
            .activate(None, self.input.read(cx).value().to_string());
        self.channel_name = None;
        self.attachment_access = None;
        self.close_emoji_picker(cx);
        self.refresh_emojis(cx);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Cancel in-flight work and discard account-owned drafts and retry keys.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_typing(cx);
        self.requests.clear();
        self.picker = None;
        self.attachment_access = None;
        self.state.reset();
        self.spellcheck.invalidate();
        self.mentions = Default::default();
        self.reset_emojis(cx);
        self.typing_enabled = false;
        self.set_spellcheck_enabled(false, cx);
        self.remote_typing.clear();
        self.remote_timer = None;
        self.channel_name = None;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.active_channel().is_some() {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Set a reply for the active channel and focus the editor. Reject stale
    /// cross-channel actions, system messages and unsent/local message IDs.
    pub fn set_reply(
        &mut self,
        message: &TimelineMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.state.active_channel() != Some(message.channel_id.as_str())
            || message.system
            || !valid_id(&message.id)
        {
            return false;
        }
        self.state.set_reply(Some(ReplyDraft {
            id: message.id.clone(),
            author: message.author_name.clone(),
            excerpt: message.content.as_deref().and_then(|content| {
                let excerpt: String = content.trim().chars().take(90).collect();
                (!excerpt.is_empty()).then(|| excerpt.replace(['\r', '\n'], " "))
            }),
        }));
        self.refresh_placeholder(window, cx);
        self.focus(window, cx);
        cx.notify();
        true
    }

    /// Clear only the active channel's reply target, leaving its text intact.
    pub fn cancel_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let changed = self.state.set_reply(None);
        if changed {
            self.refresh_placeholder(window, cx);
            cx.notify();
        }
        changed
    }

    /// Host insertion (mentions, emoji and other plain-text composer actions)
    /// uses the native editor so selection, undo and cursor behavior stay intact.
    pub fn insert_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.active_channel().is_none() || self.state.sending_here() {
            return;
        }
        self.input.update(cx, |input, cx| {
            input.replace(text, window, cx);
            input.focus(window, cx);
        });
        self.changed(cx);
        cx.notify();
    }

    /// Offline debug fixtures can seed an exact native selection without
    /// loading a saved account or sending messages/typing events.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn fixture_editor(&self) -> Entity<TextareaState> {
        self.input.clone()
    }

    pub fn set_channel(
        &mut self,
        id: String,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_channel() == Some(id.as_str()) {
            // Channel renames update the placeholder without replacing text,
            // selection, undo history or IME composition.
            if self.channel_name.as_deref() != Some(name) {
                self.channel_name = Some(name.to_owned());
                self.refresh_placeholder(window, cx);
            }
            return;
        }
        self.spellcheck.invalidate();
        self.mentions = Default::default();
        self.stop_typing(cx);
        self.state
            .activate(Some(id), self.input.read(cx).value().to_string());
        self.channel_name = Some(name.to_owned());
        self.attachment_access = None;
        self.close_emoji_picker(cx);
        self.refresh_emojis(cx);
        let draft = self.state.value().to_owned();
        self.input.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
        });
        self.refresh_placeholder(window, cx);
        cx.notify();
    }

    pub(super) fn refresh_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = if self.state.reply().is_some() {
            t("messages.replyPlaceholder")
        } else {
            t("messages.composePlaceholder").replace(
                "{channel}",
                self.channel_name.as_deref().unwrap_or_default(),
            )
        };
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
    }
}
