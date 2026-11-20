use super::{actions::Mutation, extra::Dialog, *};
use stonks_ui::i18n::{format_datetime_ms, t_vars, DateStyle};
pub(super) struct InviteState {
    pub invite_eligible: Vec<String>,
    pub invite_link: Entity<InputState>,
    pub invite_channel: Entity<UiSelectState>,
    pub invite_duration: Entity<UiSelectState>,
    pub invite_uses: Entity<UiSelectState>,
    pub created_invite: Option<ServerInvite>,
}

impl ServerSettings {
    pub fn open_invite(&mut self, channel_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(Dialog::CreateInvite, window, cx);
        let value = SharedString::from(channel_id);
        self.extra.invites.invite_channel.update(cx, |select, cx| {
            select.set_selected_value(&value, window, cx)
        });
        cx.notify();
    }
    pub(super) fn invite_url(&self, code: &str) -> Option<String> {
        let mut url = url::Url::parse(self.deployment.as_deref()?).ok()?;
        url.path_segments_mut()
            .ok()?
            .pop_if_empty()
            .extend(["invite", code]);
        Some(url.into())
    }
    pub(super) fn copy_invite(&mut self, code: &str, cx: &mut Context<Self>) {
        if let Some(url) = self.invite_url(code) {
            cx.write_to_clipboard(ClipboardItem::new_string(url));
            self.notice = Some(t("serverSettings.copied"));
            cx.notify();
        }
    }
    pub(super) fn invite_details(&self, invite: &ServerInvite) -> String {
        let uses = match (invite.max_uses, invite.uses) {
            (None, _) => t("invite.unlimited"),
            (Some(max), None) => t_vars("invite.maxUses", &[("count", max)]),
            (Some(max), Some(used)) => t_vars(
                "invite.remainingUses",
                &[("count", max.saturating_sub(used))],
            ),
        };
        let expiry = invite
            .expires
            .as_deref()
            .and_then(stonks_core::connection::parse_rfc3339)
            .and_then(|(seconds, nanos)| {
                seconds
                    .checked_mul(1000)?
                    .checked_add(i64::from(nanos / 1_000_000))
            })
            .map(|ms| {
                t_vars(
                    "invite.expiresAt",
                    &[("date", format_datetime_ms(ms, DateStyle::Numeric, false))],
                )
            })
            .unwrap_or_else(|| t("invite.neverExpires"));
        format!("{uses} · {expiry}")
    }
    fn create_invite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel) = self
            .extra
            .invites
            .invite_channel
            .read(cx)
            .selected_value()
            .map(ToString::to_string)
        else {
            return;
        };
        let uses = self
            .extra
            .invites
            .invite_uses
            .read(cx)
            .selected_value()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|uses| *uses > 0);
        let duration = self
            .extra
            .invites
            .invite_duration
            .read(cx)
            .selected_value()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let expires = if duration == 0 {
            None
        } else {
            let Some(expires) = stonks_ui::i18n::iso_timestamp_after(duration) else {
                self.error = Some(t("serverSettings.invalidExpiry"));
                cx.notify();
                return;
            };
            Some(expires)
        };
        self.mutate(
            Mutation::CreateInvite(
                channel,
                service::InviteOptions {
                    max_uses: uses,
                    expires,
                },
            ),
            window,
            cx,
        );
    }
    pub(super) fn can_invite(&self, channel: &str) -> bool {
        self.extra
            .authorization
            .as_ref()
            .is_some_and(|authorization| {
                authorization
                    .require_channel_permission(channel, 1 << 25)
                    .is_ok()
            })
    }
    pub(super) fn can_revoke(&self, code: &str) -> bool {
        self.allowed(1 << 1)
            || (self.allowed(0)
                && self
                    .extra
                    .invites
                    .created_invite
                    .as_ref()
                    .is_some_and(|invite| invite.code == code))
    }
    pub(super) fn eligible_invites(&self, channels: &[ClientChannel]) -> Vec<String> {
        channels
            .iter()
            .filter(|channel| {
                overview::text_channel(channel, &self.server.id)
                    && channel.channel_type == "TextChannel"
                    && self.can_invite(&channel.id)
            })
            .map(|channel| channel.id.clone())
            .collect()
    }
    pub(super) fn refresh_invite_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let eligible = self.eligible_invites(&self.channels);
        let selected = self
            .extra
            .invites
            .invite_channel
            .read(cx)
            .selected_value()
            .map(ToString::to_string)
            .filter(|id| eligible.contains(id))
            .or_else(|| eligible.first().cloned());
        let options: Vec<UiSelectOption> = eligible
            .iter()
            .map(|id| UiSelectOption::new(id.clone(), format!("#{}", self.channel_name(id))))
            .collect();
        self.extra.invites.invite_channel.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            if let Some(selected) = selected {
                select.set_selected_value(&SharedString::from(selected), window, cx);
            }
        });
        self.extra.invites.invite_eligible = eligible;
    }
    pub(super) fn channel_name(&self, id: &str) -> String {
        self.channels
            .iter()
            .find(|channel| channel.id == id)
            .and_then(|channel| channel.name.clone())
            .unwrap_or_else(|| t("serverSettings.unknownChannel"))
    }
    pub(super) fn render_invites(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.space4));
        let eligible = self.channels.iter().any(|channel| {
            overview::text_channel(channel, &self.server.id)
                && channel.channel_type == "TextChannel"
                && self.can_invite(&channel.id)
        });
        rows = rows
            .child(
                div()
                    .mt(px(theme.space1))
                    .mb(px(theme.space1))
                    .text_size(px(theme.font_size_md))
                    .line_height(relative(theme.line_height_normal))
                    .text_color(theme.text_muted)
                    .child(t("serverSettings.inviteIntro")),
            )
            .child(
                div().flex().justify_end().child(
                    Button::new("settings-create-invite", theme)
                        .label(t("serverSettings.createInvite"))
                        .disabled(self.busy || !eligible)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_dialog(Dialog::CreateInvite, window, cx)
                        })),
                ),
            );
        rows = self.resource_status(rows, theme, cx);
        if !self.allowed(1 << 1) {
            rows = rows.child(Feedback::new(
                theme,
                FeedbackTone::Error,
                t("serverSettings.noPermission"),
            ));
        } else if let Some(invites) = &self.invites {
            let mut section = settings_section(&theme, &t("serverSettings.activeInvites"));
            if invites.is_empty() {
                section = section.child(Self::resource_empty(&theme, "No active invites."));
            }
            for (index, invite) in invites.iter().enumerate() {
                let code = invite.code.clone();
                let revoke = invite.clone();
                let creator = self
                    .extra
                    .users
                    .get(&invite.creator_id)
                    .unwrap_or(&invite.creator_id);
                let details = format!(
                    "{} · {} · {}",
                    t_vars("serverSettings.createdBy", &[("name", creator)]),
                    invite.code,
                    self.invite_details(invite)
                );
                section = section.child(settings_row(
                    &theme,
                    &format!("#{}", self.channel_name(&invite.channel_id)),
                    &details,
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(theme.space2))
                        .child(
                            Button::new(
                                ElementId::Name(format!("copy-server-invite-{index}").into()),
                                theme,
                            )
                            .variant(ButtonVariant::Secondary)
                            .label(t("serverSettings.copyInvite"))
                            .disabled(self.busy || self.invite_url(&invite.code).is_none())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.copy_invite(&code, cx)),
                            ),
                        )
                        .child(
                            Button::new(
                                ElementId::Name(format!("revoke-server-invite-{index}").into()),
                                theme,
                            )
                            .variant(ButtonVariant::DangerQuiet)
                            .label(t("serverSettings.revoke"))
                            .disabled(self.busy || !self.allowed(1 << 1))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.open_dialog(
                                        Dialog::RevokeInvite(revoke.clone()),
                                        window,
                                        cx,
                                    )
                                },
                            )),
                        ),
                ));
            }
            rows = rows.child(section);
        }
        rows.into_any_element()
    }
}

impl ServerSettings {
    pub(super) fn invite_dialog(
        &self,
        dialog: &Dialog,
        theme: Theme,
        cx: &mut Context<Self>,
        mut content: Div,
        mut footer: Div,
    ) -> (Div, Div) {
        match dialog {
            Dialog::RevokeInvite(invite) => {
                let code = invite.code.clone();
                footer = footer.child(self.dialog_cancel(theme, cx)).child(
                    Button::new("revoke-invite-confirm", theme)
                        .variant(ButtonVariant::Danger)
                        .label(t("serverSettings.revokeInvite"))
                        .disabled(self.busy || !self.can_revoke(&invite.code))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.mutate(Mutation::RevokeInvite(code.clone()), window, cx)
                        })),
                );
            }
            Dialog::CreateInvite => {
                if let Some(invite) = &self.extra.invites.created_invite {
                    content = content
                        .child(input_field(
                            &theme,
                            &t("serverSettings.inviteLink"),
                            &t("serverSettings.inviteLinkHint"),
                            SettingsInput::new(&self.extra.invites.invite_link, theme)
                                .label(t("serverSettings.inviteLink"))
                                .readonly(true),
                        ))
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .text_size(px(theme.font_size_sm))
                                .child(self.invite_details(invite)),
                        );
                    let code = invite.code.clone();
                    let copied = code.clone();
                    let confirmed = invite.clone();
                    footer =
                        footer
                            .child(
                                Button::new("created-invite-revoke", theme)
                                    .variant(ButtonVariant::DangerSubtle)
                                    .label(t("serverSettings.revokeInvite"))
                                    .disabled(self.busy || !self.can_revoke(&invite.code))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_dialog(
                                            Dialog::RevokeInvite(confirmed.clone()),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(self.dialog_cancel(theme, cx))
                            .child(
                                Button::new("created-invite-copy", theme)
                                    .label(t("serverSettings.copyInvite"))
                                    .disabled(self.busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_invite(&copied, cx)
                                    })),
                            );
                } else {
                    content = content
                        .child(
                            settings_field(&theme, &t("serverSettings.destinationChannel"), "")
                                .child(
                                    UiSelect::new(&self.extra.invites.invite_channel, theme)
                                        .label(t("serverSettings.destinationChannel"))
                                        .disabled(self.busy),
                                ),
                        )
                        .child(
                            settings_field(&theme, &t("invite.expiresAfter"), "").child(
                                UiSelect::new(&self.extra.invites.invite_duration, theme)
                                    .label(t("invite.expiresAfter"))
                                    .disabled(self.busy),
                            ),
                        )
                        .child(
                            settings_field(&theme, &t("invite.maximumUses"), "").child(
                                UiSelect::new(&self.extra.invites.invite_uses, theme)
                                    .label(t("invite.maximumUses"))
                                    .disabled(self.busy),
                            ),
                        );
                    if let Some(channel) =
                        self.extra.invites.invite_channel.read(cx).selected_value()
                    {
                        content = content.child(
                            div()
                                .text_size(px(theme.font_size_sm))
                                .text_color(theme.text_muted)
                                .child(t_vars(
                                    "serverSettings.inviteDestinationHint",
                                    &[("channel", self.channel_name(channel))],
                                )),
                        );
                    }
                    let selected = self
                        .extra
                        .invites
                        .invite_channel
                        .read(cx)
                        .selected_value()
                        .map(ToString::to_string);
                    if selected.is_none() {
                        content = content.child(Feedback::new(
                            theme,
                            FeedbackTone::Error,
                            t("serverSettings.noInviteChannels"),
                        ));
                    }
                    footer = footer.child(self.dialog_cancel(theme, cx)).child(
                        Button::new("create-invite-confirm", theme)
                            .label(t(if self.busy {
                                "serverSettings.creating"
                            } else {
                                "serverSettings.createInvite"
                            }))
                            .disabled(
                                self.busy
                                    || selected.as_ref().is_none_or(|id| !self.can_invite(id)),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.create_invite(window, cx)),
                            ),
                    );
                }
            }
            _ => unreachable!("dialog routed to its workflow"),
        }
        (content, footer)
    }
}
