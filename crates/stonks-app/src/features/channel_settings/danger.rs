use super::*;
use stonks_ui::components::UiDialog;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelDanger {
    Reset,
    Delete,
}
impl ChannelDanger {
    pub fn title(self, name: &str) -> String {
        format!(
            "{} #{}?",
            if self == Self::Reset {
                "Reset"
            } else {
                "Delete"
            },
            name
        )
    }
    pub fn description(self) -> &'static str {
        if self == Self::Reset {
            "This permanently deletes every message in the channel."
        } else {
            "This permanently deletes the channel and all of its messages."
        }
    }
    pub fn label(self) -> &'static str {
        if self == Self::Reset {
            "Reset channel"
        } else {
            "Delete channel"
        }
    }
}

impl ChannelSettings {
    pub(super) fn allowed_danger(&self, action: ChannelDanger) -> bool {
        match action {
            ChannelDanger::Reset => self.access.manage_messages,
            ChannelDanger::Delete => self.access.manage_channel,
        }
    }
    pub fn open_confirmation(&mut self, action: ChannelDanger, cx: &mut Context<Self>) {
        if !self.busy && self.allowed_danger(action) {
            self.confirmation = Some(action);
            self.pending_focus = true;
            cx.notify();
        }
    }
    pub fn confirm(&mut self, action: ChannelDanger, cx: &mut Context<Self>) {
        if self.confirmation != Some(action) || self.busy || !self.allowed_danger(action) {
            return;
        }
        self.confirmation = None;
        self.perform(Action::Danger(action), cx);
    }
    pub(super) fn render_confirmation(
        &self,
        action: ChannelDanger,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        UiDialog::new(
            "channel-settings-confirmation",
            theme,
            action.title(self.channel.name.as_deref().unwrap_or("channel")),
        )
        .description(if !self.frame_visible && action == ChannelDanger::Delete {
            "The channel and all of its messages will be permanently deleted."
        } else {
            action.description()
        })
        .busy(self.busy)
        .show_close(false)
        .focus(&self.confirmation_focus)
        .on_close(cx.listener(|this, _, _, cx| this.close(cx)))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(theme.space2))
                .mt(px(theme.space4))
                .child(
                    Button::new("channel-confirm-cancel", theme)
                        .variant(ButtonVariant::Secondary)
                        .label("Cancel")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                )
                .child(
                    Button::new("channel-confirm-action", theme)
                        .variant(ButtonVariant::Danger)
                        .label(if self.busy {
                            "Working…"
                        } else {
                            action.label()
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm(action, cx);
                            this.focus(window, cx);
                        })),
                ),
        )
        .into_any_element()
    }
}
