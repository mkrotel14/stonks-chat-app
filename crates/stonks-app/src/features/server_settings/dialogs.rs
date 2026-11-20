use super::extra::Dialog;
use super::*;
use stonks_ui::{components::UiDialog, i18n::t_vars};
impl ServerSettings {
    pub fn set_frame_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.extra.frame_visible = visible;
        cx.notify();
    }
    pub fn visible(&self) -> bool {
        self.extra.frame_visible || self.extra.dialog.is_some()
    }
    pub fn is_settings_open(&self) -> bool {
        self.extra.frame_visible
    }
    pub(super) fn open_dialog(
        &mut self,
        dialog: Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        if self.extra.dialog.is_none() {
            self.extra.previous_focus = window.focused(cx);
        }
        self.error = None;
        self.notice = None;
        match &dialog {
            Dialog::CreateRole => {
                self.extra.roles.role_name.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.focus(window, cx);
                });
            }
            Dialog::Emoji(emoji) => {
                self.extra.emoji.rename_emoji.update(cx, |input, cx| {
                    input.set_value(emoji.name.clone(), window, cx);
                    input.focus(window, cx);
                });
            }
            Dialog::CreateInvite => {
                self.refresh_invite_options(window, cx);
                self.extra.invites.created_invite = None;
                for select in [
                    &self.extra.invites.invite_duration,
                    &self.extra.invites.invite_uses,
                ] {
                    select.update(cx, |select, cx| {
                        select.set_selected_value(&SharedString::from("0"), window, cx)
                    });
                }
                self.extra.dialog_focus.focus(window, cx);
            }
            _ => self.extra.dialog_focus.focus(window, cx),
        }
        self.extra.dialog = Some(dialog);
        cx.notify();
    }
    pub(super) fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.extra.dialog = None;
        if !self.extra.frame_visible {
            cx.emit(ServerSettingsEvent::Close);
        }
        self.error = None;
        if let Some(focus) = self.extra.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(super) fn render_dialog(&self, theme: Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.extra.dialog.as_ref()?;
        let (title, description) = match dialog {
            Dialog::CreateRole => (t("serverSettings.createRole"), String::new()),
            Dialog::DeleteRole(_, name) => (
                t_vars("serverSettings.deleteRoleTitle", &[("name", name)]),
                t("serverSettings.deleteRoleHint"),
            ),
            Dialog::Emoji(emoji) => (format!(":{}:", emoji.name), String::new()),
            Dialog::CreateInvite => (
                t_vars("serverSettings.inviteTitle", &[("name", &self.server.name)]),
                String::new(),
            ),
            Dialog::RevokeInvite(invite) => (
                t("serverSettings.revokeInviteTitle"),
                t_vars(
                    "serverSettings.revokeInviteHint",
                    &[("channel", self.channel_name(&invite.channel_id))],
                ),
            ),
        };
        let mut panel = UiDialog::new("server-settings-dialog", theme, title)
            .show_close(false)
            .focus(&self.extra.dialog_focus)
            .busy(self.busy)
            .on_close(cx.listener(|this, _, window, cx| this.close_dialog(window, cx)));
        if !description.is_empty() {
            panel = panel.description(description);
        }
        let content = div().flex().flex_col().gap(px(theme.space4));
        let footer = div().flex().flex_wrap().justify_end().gap(px(theme.space2));
        let (mut content, footer) = match dialog {
            Dialog::CreateRole | Dialog::DeleteRole(..) => {
                self.role_dialog(dialog, theme, cx, content, footer)
            }
            Dialog::Emoji(emoji) => self.emoji_dialog(emoji, theme, cx, content, footer),
            Dialog::CreateInvite | Dialog::RevokeInvite(_) => {
                self.invite_dialog(dialog, theme, cx, content, footer)
            }
        };
        if let Some(error) = &self.error {
            content = content.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if let Some(notice) = &self.notice {
            content = content.child(Feedback::new(theme, FeedbackTone::Success, notice.clone()));
        }
        panel = panel.child(content).child(footer);
        Some(panel.into_any_element())
    }
    pub(super) fn dialog_cancel(&self, theme: Theme, cx: &mut Context<Self>) -> Button {
        Button::new("settings-dialog-cancel", theme)
            .variant(ButtonVariant::Secondary)
            .label(t(
                if matches!(&self.extra.dialog, Some(Dialog::Emoji(_)))
                    || (matches!(&self.extra.dialog, Some(Dialog::CreateInvite))
                        && self.extra.invites.created_invite.is_some())
                {
                    "common.close"
                } else {
                    "common.cancel"
                },
            ))
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, window, cx| this.close_dialog(window, cx)))
    }
}
