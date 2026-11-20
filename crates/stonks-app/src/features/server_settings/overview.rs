use super::*;
impl ServerSettings {
    pub(super) fn overview(
        &self,
        theme: Theme,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let draft = self.draft(cx);
        let disabled = self.busy || !self.allowed(1 << 1);
        let mut form = div().flex().flex_col().gap(px(theme.space4));
        form = form.child(
            settings_field(&theme, &t("serverSettings.icon"), "").child(
                Button::new("server-settings-icon", theme)
                    .variant(ButtonVariant::IconUpload)
                    .tooltip(t("serverSettings.changeIcon"))
                    .disabled(disabled)
                    .leading(
                        div()
                            .relative()
                            .size(px(96.))
                            .child(self.icon(&theme, 96., cx))
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .py(px(6.))
                                    .px(px(2.))
                                    .bg(theme.surface_raised)
                                    .text_color(theme.text)
                                    .text_size(px(theme.font_size_xs))
                                    .text_center()
                                    .child(t(if self.busy {
                                        "serverSettings.uploading"
                                    } else {
                                        "serverSettings.changeIcon"
                                    })),
                            )
                            .child(
                                gpui_kit::base::RoundedFrameCover::uniform(px(48.))
                                    .backdrop(theme.surface.into()),
                            ),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.pick_icon(None, window, cx))),
            ),
        );
        form = form.child(input_field(
            &theme,
            &t("serverSettings.name"),
            &format!("{}/32", draft.name.chars().count()),
            SettingsInput::new(&self.name, theme)
                .label(t("serverSettings.name"))
                .disabled(disabled),
        ));
        form = form.child(
            settings_field(&theme, &t("serverSettings.description"), "")
                .child(
                    SettingsInput::textarea(&self.description, theme)
                        .label(t("serverSettings.description"))
                        .disabled(disabled),
                )
                .child(
                    div()
                        .text_right()
                        .text_size(px(theme.font_size_sm))
                        .text_color(theme.text_muted)
                        .child(format!("{}/1024", draft.description.chars().count())),
                ),
        );
        let mut section = settings_section(&theme, &t("serverSettings.systemChannels"));
        for (index, label) in [
            "serverSettings.userJoined",
            "serverSettings.userLeft",
            "serverSettings.userKicked",
            "serverSettings.userBanned",
        ]
        .iter()
        .enumerate()
        {
            let trailing = div()
                .when(content_width <= 560., |view| view.w_full())
                .when(content_width > 560., |view| {
                    view.w(px(270_f32.min(content_width * 0.44)))
                })
                .child(
                    UiSelect::new(&self.systems[index], theme)
                        .min_height(42.)
                        .label(t(label))
                        .disabled(disabled),
                );
            section = section.child(
                settings_row(&theme, &t(label), &t("serverSettings.systemHint"), trailing)
                    .when(content_width <= 560., |row| row.flex_col().items_stretch()),
            );
        }
        form = form.child(section);
        if !self.allowed(1 << 1) {
            form = form.child(Feedback::new(
                theme,
                FeedbackTone::Error,
                t("serverSettings.noPermission"),
            ));
        }
        if draft.body().is_err() {
            form = form.child(Feedback::new(
                theme,
                FeedbackTone::Error,
                t("serverSettings.invalid"),
            ));
        }
        form.child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(theme.space2))
                .mt(px(theme.space4))
                .child(
                    Button::new("server-settings-reset", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("serverSettings.cancel"))
                        .disabled(self.busy || draft == self.initial)
                        .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                )
                .child(
                    Button::new("server-settings-save", theme)
                        .label(t(if self.busy {
                            "serverSettings.saving"
                        } else {
                            "serverSettings.save"
                        }))
                        .disabled(!can_save(&self.initial, &draft, self.can_manage, self.busy))
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
        .into_any_element()
    }
    pub(super) fn draft(&self, cx: &App) -> Overview {
        Overview {
            name: self.name.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
            system_messages: SYSTEM_EVENTS
                .iter()
                .zip(&self.systems)
                .map(|(key, select)| {
                    (
                        key.to_string(),
                        select
                            .read(cx)
                            .selected_value()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                    )
                })
                .collect(),
        }
    }
    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.name.update(cx, |input, cx| {
            input.set_value(self.initial.name.clone(), window, cx)
        });
        self.description.update(cx, |input, cx| {
            input.set_value(self.initial.description.clone(), window, cx)
        });
        for (key, select) in SYSTEM_EVENTS.iter().zip(&self.systems) {
            let value = SharedString::from(
                self.initial
                    .system_messages
                    .get(*key)
                    .cloned()
                    .unwrap_or_default(),
            );
            select.update(cx, |select, cx| {
                select.set_selected_value(&value, window, cx)
            });
        }
        self.error = None;
        self.notice = None;
        cx.notify();
    }
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        if !can_save(&self.initial, &draft, self.can_manage, self.busy) {
            return;
        }
        if !channels_valid(&draft, &self.channels, &self.server.id) {
            self.error = Some(t("serverSettings.invalidChannel").into());
            cx.notify();
            return;
        }
        let Some(authorization) = self.extra.authorization.clone() else {
            return;
        };
        if let Err(error) = authorization.require_server_permission(&self.server.id, 1 << 1) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        self.busy = true;
        self.save_generation = self.save_generation.wrapping_add(1);
        let generation = self.save_generation;
        self.error = None;
        self.notice = None;
        let policy = self.policy.clone();
        let id = self.server.id.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    authorization
                        .require_server_permission(&id, 1 << 1)
                        .map_err(|_| {
                            stonks_core::error::CommandError::new(
                                stonks_core::error::ErrorCode::AuthenticationFailed,
                                "Your account does not have permission to manage this server.",
                            )
                        })?;
                    service::save_overview(&policy, &id, &draft).await
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if this.save_generation != generation || !this.can_manage {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(server) => {
                        this.initial = Overview::from_server(&server);
                        this.icon_url = server.icon.as_ref().and_then(|file| {
                            media::file_url(this.autumn.as_deref(), &file.tag, &file.id)
                        });
                        this.server = server.clone();
                        this.reset(window, cx);
                        this.notice = Some(t("serverSettings.saved").into());
                        cx.emit(ServerSettingsEvent::Updated(server));
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        });
        self.save_request = Some(Request { abort, _task: task });
        cx.notify();
    }
}
pub(super) fn text_channel(channel: &ClientChannel, server_id: &str) -> bool {
    channel.server.as_deref() == Some(server_id)
        && channel.channel_type != "VoiceChannel"
        && channel.voice.is_none()
}
fn can_save(initial: &Overview, draft: &Overview, allowed: bool, busy: bool) -> bool {
    allowed && !busy && draft != initial && draft.body().is_ok()
}
fn channels_valid(draft: &Overview, channels: &[ClientChannel], server_id: &str) -> bool {
    draft.system_messages.values().all(|id| {
        id.is_empty()
            || channels
                .iter()
                .any(|channel| channel.id == *id && text_channel(channel, server_id))
    })
}

#[cfg(test)]
mod tests {
    use super::{can_save, channels_valid, ClientChannel, Overview};
    use std::collections::BTreeMap;
    fn overview() -> Overview {
        Overview {
            name: "Server".into(),
            description: String::new(),
            system_messages: BTreeMap::new(),
        }
    }
    #[test]
    fn edits_require_permission_valid_content_and_idle_state() {
        let initial = overview();
        let mut draft = initial.clone();
        assert!(!can_save(&initial, &draft, true, false));
        draft.name = "Edited".into();
        assert!(can_save(&initial, &draft, true, false));
        assert!(!can_save(&initial, &draft, false, false));
        assert!(!can_save(&initial, &draft, true, true));
        draft.name = " ".into();
        assert!(!can_save(&initial, &draft, true, false));
        assert_eq!(initial.name, "Server");
    }
    #[test]
    fn system_channels_must_belong_to_server_and_allow_text() {
        let mut draft = overview();
        draft
            .system_messages
            .insert("user_joined".into(), "channel".into());
        let mut channel: ClientChannel = serde_json::from_value(
            serde_json::json!({"_id":"channel","channel_type":"TextChannel","server":"server"}),
        )
        .unwrap();
        assert!(channels_valid(&draft, &[channel.clone()], "server"));
        assert!(!channels_valid(&draft, &[channel.clone()], "other"));
        channel.voice = Some(serde_json::json!({}));
        assert!(!channels_valid(&draft, &[channel], "server"));
        draft
            .system_messages
            .insert("user_joined".into(), String::new());
        assert!(channels_valid(&draft, &[], "server"));
    }
}
