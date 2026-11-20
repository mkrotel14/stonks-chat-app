use super::*;
impl ChannelSettings {
    pub(super) fn overview(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.busy || !self.access.manage_channel;
        let draft = self.draft(cx);
        let fields = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(theme.space4))
            .child(input_field(
                &theme,
                "Channel name",
                &format!("{}/32", draft.name.encode_utf16().count()),
                SettingsInput::new(&self.name, theme)
                    .label("Channel name")
                    .disabled(disabled),
            ))
            .child(
                settings_field(&theme, "Description", "")
                    .child(
                        SettingsInput::textarea(&self.description, theme)
                            .label("Description")
                            .disabled(disabled),
                    )
                    .child(
                        div()
                            .text_size(px(theme.font_size_sm))
                            .text_color(theme.text_muted)
                            .text_right()
                            .child(format!("{}/1024", draft.description.encode_utf16().count())),
                    ),
            )
            .child(
                settings_field(&theme, "Slowmode", "").child(
                    UiSelect::new(&self.slowmode, theme)
                        .label("Slowmode")
                        .disabled(disabled),
                ),
            )
            .child(settings_row(
                &theme,
                "Age-restricted channel",
                "Ask users to confirm their age before opening it.",
                Switch::new("channel-settings-nsfw", theme)
                    .label("Age-restricted channel")
                    .checked(self.nsfw)
                    .disabled(disabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.nsfw = *value;
                        this.notice = None;
                        cx.notify();
                    })),
            ));
        let mut form = div().flex().flex_col().min_w_0().child(fields).child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(theme.space2))
                .mt(px(theme.space4))
                .child(
                    Button::new("channel-settings-save", theme)
                        .label(if self.busy {
                            "Saving…"
                        } else {
                            "Save changes"
                        })
                        .disabled(
                            disabled
                                || draft.name.trim().is_empty()
                                || draft.description.encode_utf16().count() > 1024,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.save_overview(cx))),
                ),
        );
        if self.access.manage_messages || self.access.manage_channel {
            let mut danger = settings_section(&theme, "Danger zone");
            if self.access.manage_messages {
                danger = danger.child(settings_row(
                    &theme,
                    "Reset channel",
                    "Permanently delete every message while keeping the channel and its settings.",
                    Button::new("channel-settings-reset", theme)
                        .variant(ButtonVariant::Danger)
                        .label(if self.busy {
                            "Working…"
                        } else {
                            "Reset channel"
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_confirmation(ChannelDanger::Reset, cx);
                            this.focus(window, cx);
                        })),
                ))
            }
            if self.access.manage_channel {
                danger = danger.child(settings_row(
                    &theme,
                    "Delete channel",
                    "Permanently delete this channel and all of its messages.",
                    Button::new("channel-settings-delete", theme)
                        .variant(ButtonVariant::Danger)
                        .label(if self.busy {
                            "Working…"
                        } else {
                            "Delete channel"
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_confirmation(ChannelDanger::Delete, cx);
                            this.focus(window, cx);
                        })),
                ))
            }
            form = form.child(danger);
        }
        form.into_any_element()
    }
    fn draft(&self, cx: &App) -> Overview {
        Overview {
            name: self.name.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
            slowmode: self
                .slowmode
                .read(cx)
                .selected_value()
                .and_then(|value| value.parse().ok())
                .unwrap_or(self.initial.slowmode),
            nsfw: self.nsfw,
        }
    }
    fn save_overview(&mut self, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        if let Err(error) = draft.body() {
            self.show_feedback(Some(error.to_string()), None, cx);
            return;
        }
        self.perform(Action::Overview(draft), cx);
    }
}
/// Source options plus an existing compatible deployment value, so merely
/// opening settings never silently rewrites a custom slowmode duration.
pub(super) fn slowmode_options(current: u64) -> Vec<(u64, String)> {
    let mut options = vec![
        (0, "Off".into()),
        (5, "5 seconds".into()),
        (10, "10 seconds".into()),
        (30, "30 seconds".into()),
        (60, "1 minute".into()),
        (300, "5 minutes".into()),
        (3600, "1 hour".into()),
    ];
    if !options.iter().any(|(value, _)| *value == current) {
        options.push((current, format!("{current} seconds")));
    }
    options
}
/// HTML maxlength counts UTF-16 code units; preserve that UI bound without
/// cutting a Rust UTF-8 scalar or leaving half an emoji surrogate pair.
pub(super) fn utf16_prefix(value: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, ch) in value.char_indices() {
        units += ch.len_utf16();
        if units > max {
            return &value[..index];
        }
    }
    value
}
#[cfg(test)]
mod tests {
    use super::{slowmode_options, utf16_prefix};
    #[test]
    fn preserves_custom_duration_and_source_option_order() {
        let options = slowmode_options(42);
        assert_eq!(
            options.iter().map(|(value, _)| *value).collect::<Vec<_>>(),
            [0, 5, 10, 30, 60, 300, 3600, 42]
        );
        assert_eq!(slowmode_options(300).len(), 7);
    }
    #[test]
    fn textarea_bound_matches_html_without_splitting_emoji() {
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(utf16_prefix("a😀b", 2), "a");
        assert_eq!(utf16_prefix("é中", 2), "é中");
        assert_eq!(utf16_prefix("x", 0), "");
    }
}
