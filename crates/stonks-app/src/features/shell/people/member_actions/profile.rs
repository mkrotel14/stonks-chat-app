use super::*;
use stonks_ui::i18n::{t, t_vars};
pub(super) fn profile_avatar_url(
    autumn: Option<&str>,
    api: Option<&str>,
    user: &ClientUser,
    member: Option<&ClientMember>,
) -> Option<String> {
    member
        .and_then(|member| member.avatar.as_ref())
        .or(user.avatar.as_ref())
        .and_then(|file| media::avatar_url(autumn, &file.tag, &file.id))
        .or_else(|| {
            media::file_url(api, "users", &user.id).map(|url| format!("{url}/default_avatar"))
        })
}

#[cfg(test)]
mod tests {
    use super::{profile_avatar_url, ClientMember, ClientUser};
    #[test]
    fn default_profile_avatar_and_member_animation_match_shell_sources() {
        let mut user: ClientUser =
            serde_json::from_value(serde_json::json!({"_id":"member","username":"Member"}))
                .unwrap();
        assert_eq!(
            profile_avatar_url(None, Some("https://chat.example/api/"), &user, None).as_deref(),
            Some("https://chat.example/api/users/member/default_avatar")
        );
        user.avatar = Some(stonks_core::connection::ClientFileReference {
            id: "accountGif".into(),
            tag: "avatars".into(),
            content_type: None,
        });
        let member:ClientMember=serde_json::from_value(serde_json::json!({"_id":{"server":"server","user":"member"},"avatar":{"_id":"serverGif","tag":"avatars"}})).unwrap();
        let avatar = profile_avatar_url(
            Some("https://cdn.example/media"),
            Some("https://chat.example/api"),
            &user,
            Some(&member),
        )
        .unwrap();
        assert!(avatar.contains("serverGif"));
        assert!(!avatar.contains("accountGif"));
        // Native decoding needs the original resource; the source's animated=1
        // parameter is solely a WebKit thumbnail-cache workaround.
        assert!(avatar.ends_with("/original"));
    }
}
fn profile_action_label(action: Action) -> String {
    t(match action {
        Action::Message => "relationships.message",
        Action::Add => "relationships.add",
        Action::Accept => "relationships.accept",
        Action::Decline => "relationships.decline",
        Action::Cancel => "relationships.cancel",
        Action::Remove => "relationships.remove",
        Action::Block => "relationships.block",
        Action::Unblock => "relationships.unblock",
        Action::Transfer => "servers.transfer",
        _ => return action.label().into(),
    })
}
fn action_icon(action: Action) -> Icon {
    match action {
        Action::Message => Icon::MessageCircle,
        Action::Add => Icon::UserPlus,
        Action::Accept => Icon::Check,
        Action::Decline => Icon::X,
        Action::Cancel => Icon::CircleX,
        Action::Remove => Icon::UserMinus,
        Action::Block => Icon::Ban,
        Action::Unblock => Icon::LockOpen,
        Action::Transfer => Icon::ArrowLeftRight,
        _ => Icon::Ellipsis,
    }
}
pub(super) fn role_colour(value: Option<&str>, fallback: Rgba) -> Rgba {
    value
        .and_then(|value| value.strip_prefix('#'))
        .filter(|value| value.len() == 6)
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .map(rgb)
        .unwrap_or(fallback)
}
impl MemberActions {
    pub(super) fn render_profile(
        &mut self,
        theme: Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(user) = self.user(cx) else {
            return div().into_any_element();
        };
        let member = self.member(cx);
        let server = self.server(cx);
        let name = member
            .as_ref()
            .and_then(|member| member.nickname.clone())
            .or_else(|| user.display_name.clone())
            .unwrap_or_else(|| user.username.clone());
        let autumn = self
            .session
            .read(cx)
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let api = self
            .session
            .read(cx)
            .server
            .as_ref()
            .map(|server| server.endpoints.api.as_str());
        let avatar_url = profile_avatar_url(autumn, api, &user, member.as_ref());
        let background_url = self
            .profile
            .background
            .as_ref()
            .and_then(|file| media::file_url(autumn, &file.tag, &file.id));
        self.images.update(cx, |images, _| {
            images.set_sources(
                "member-profile",
                avatar_url
                    .iter()
                    .cloned()
                    .map(|url| (url, 152))
                    .chain(background_url.iter().cloned().map(|url| (url, 1280))),
            )
        });
        let image = self
            .images
            .update(cx, |images, cx| images.get(avatar_url.as_deref(), 152, cx));
        let background = self.images.update(cx, |images, cx| {
            images.get(background_url.as_deref(), 1280, cx)
        });
        let mut title = div()
            .flex()
            .flex_col()
            .w_full()
            .items_center()
            .gap(px(theme.space3));
        if let Some(background) = background {
            title = title.child(
                img(background)
                    .w_full()
                    .h(px(80.))
                    .object_fit(ObjectFit::Cover)
                    .rounded(px(theme.radius_md)),
            );
        }
        title = title.child(avatar(&theme, &name, 76., image)).child(
            div()
                .flex()
                .items_center()
                .gap(px(theme.space1))
                .text_size(px(theme.font_size_xl))
                .line_height(relative(theme.line_height_compact))
                .font_weight(FontWeight::BOLD)
                .child(name.clone())
                .when(
                    user.bot.as_ref().is_some_and(|bot| {
                        !bot.is_null() && bot != &serde_json::Value::Bool(false)
                    }),
                    |view| view.child(stonks_ui::components::BotBadge::new(theme)),
                ),
        );
        let username = format!(
            "@{}{}",
            user.username,
            user.discriminator
                .as_ref()
                .map(|value| format!("#{value}"))
                .unwrap_or_default()
        );
        let mut content = div()
            .flex()
            .flex_col()
            .w_full()
            .items_center()
            .text_color(theme.text)
            .child(title)
            .child(
                div()
                    .mt(px(theme.space1))
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child(username),
            );
        if let Some(pronouns) = member
            .as_ref()
            .and_then(|member| member.pronouns.as_ref())
            .or(user.pronouns.as_ref())
            .filter(|text| !text.is_empty())
        {
            content = content.child(
                div()
                    .mt(px(theme.space2))
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(pronouns.clone()),
            );
        }
        if let Some(status) = user
            .status
            .as_ref()
            .and_then(|status| status.text.as_ref())
            .filter(|text| !text.is_empty())
        {
            content = content.child(
                div()
                    .w_full()
                    .mt(px(theme.space4))
                    .p(px(theme.space3))
                    .rounded(px(theme.radius_md))
                    .bg(theme.surface_interactive)
                    .text_size(px(theme.font_size_sm))
                    .child(status.clone()),
            );
        }
        if let Some((seconds, _)) = member
            .as_ref()
            .and_then(|member| member.joined_at.as_deref())
            .and_then(stonks_core::connection::parse_rfc3339)
        {
            let date = stonks_ui::i18n::format_date_ms(
                seconds.saturating_mul(1000),
                stonks_ui::i18n::DateStyle::Long,
                true,
                false,
            );
            content = content.child(
                div()
                    .mt(px(theme.space4))
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_xs))
                    .child(format!("Joined this server {date}")),
            );
        }
        if let (Some(member), Some(server)) = (&member, &server) {
            let mut roles: Vec<_> = member
                .roles
                .iter()
                .filter_map(|id| server.roles.get(id))
                .collect();
            roles.sort_by_key(|role| role.rank.unwrap_or(0));
            if !roles.is_empty() {
                let mut chips = div()
                    .flex()
                    .flex_wrap()
                    .w_full()
                    .justify_center()
                    .gap(px(theme.space2))
                    .mt(px(theme.space3));
                for role in roles {
                    chips = chips.child(
                        div()
                            .flex()
                            .items_center()
                            .min_h(px(27.))
                            .gap(px(theme.space2))
                            .py(px(theme.space1))
                            .px(px(theme.space2))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.outline)
                            .bg(theme.surface_interactive)
                            .text_size(px(theme.font_size_xs))
                            .child(
                                div()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(role_colour(role.colour.as_deref(), theme.text_muted)),
                            )
                            .child(role.name.clone()),
                    );
                }
                content = content.child(chips);
            }
        }
        if let Some(error) = &self.error {
            content = content.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        let actions = self.profile_actions(cx);
        if !actions.is_empty() {
            let mut footer = div()
                .flex()
                .flex_wrap()
                .w_full()
                .justify_center()
                .mt(px(theme.space4))
                .gap(px(theme.space2));
            for action in actions {
                footer = footer.child(
                    Button::new(
                        ElementId::Name(format!("profile-action-{action:?}").into()),
                        theme,
                    )
                    .icon_only(true)
                    .variant(if action.dangerous() {
                        ButtonVariant::DangerSubtle
                    } else if matches!(action, Action::Message | Action::Add | Action::Accept) {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .tooltip(if self.busy() && self.profile_pending == Some(action) {
                        format!("{}…", profile_action_label(action))
                    } else {
                        profile_action_label(action)
                    })
                    .leading(action_icon(action).render(
                        if action.dangerous() {
                            theme.danger
                        } else if matches!(action, Action::Message | Action::Add | Action::Accept) {
                            theme.accent_text
                        } else {
                            theme.text
                        },
                        18.,
                    ))
                    .disabled(self.busy())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.perform_profile_action(action, window, cx)
                    })),
                );
            }
            content = content.child(footer);
        }
        UiDialog::new("member-profile", theme, "")
            .width(360.)
            .padding(theme.space6)
            .position(self.position)
            .title_content(content)
            .focus(&self.focus)
            .busy(self.busy())
            .on_close(cx.listener(|this, _, window, cx| this.close(window, cx)))
            .into_any_element()
    }
    pub(super) fn render_prompt(
        &mut self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.stage == Stage::Mfa && self.mfa_focus_pending {
            self.mfa_focus_pending = false;
            self.secret.update(cx, |input, cx| input.focus(window, cx));
        }
        let name = self
            .user(cx)
            .map(|user| user.display_name.unwrap_or(user.username))
            .unwrap_or_default();
        let (title, description, action) = match self.stage {
            Stage::Confirm(action) => {
                let (title, description) = match action {
                    Action::Kick => (
                        format!("Kick {name}?"),
                        "This will kick the member from this server.".into(),
                    ),
                    Action::Ban => (
                        format!("Ban {name}?"),
                        "This will ban the member from this server.".into(),
                    ),
                    Action::Block => (
                        format!("Block {name}?"),
                        "They will no longer be able to contact you.".into(),
                    ),
                    Action::Transfer => {
                        let server = self
                            .server(cx)
                            .map(|server| server.name)
                            .unwrap_or_default();
                        (
                            t_vars("servers.transferTitle", &[("name", &name)]),
                            t_vars(
                                "servers.transferDescription",
                                &[("server", &server), ("name", &name)],
                            ),
                        )
                    }
                    _ => (action.label().into(), String::new()),
                };
                (title, description, action.label())
            }
            Stage::Report => (
                "Report content".into(),
                "Describe why you are reporting this content (optional).".into(),
                "Submit",
            ),
            Stage::Mfa => (
                t("settings.mfa.confirmAction"),
                t("settings.mfa.confirmDescription"),
                "Confirm",
            ),
            _ => return div().into_any_element(),
        };
        let mut content = div().flex().flex_col().gap(px(theme.space4));
        if let Some(error) = self.error.as_ref().filter(|_| self.stage != Stage::Mfa) {
            content = content.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if self.stage == Stage::Report {
            content = content.child(
                SettingsInput::textarea(&self.context, theme)
                    .label("Report content")
                    .disabled(self.busy()),
            );
        }
        if self.stage == Stage::Mfa {
            let mut choices = div().grid().grid_cols(3).gap(px(7.));
            for method in self.mfa_methods.clone() {
                let selected = self.mfa_method == method;
                let label = t(&format!("settings.mfa.methods.{method}"));
                choices = choices.child(
                    Button::new(
                        ElementId::Name(format!("member-mfa-{method}").into()),
                        theme,
                    )
                    .variant(ButtonVariant::MfaMethod)
                    .label(label)
                    .selected(selected)
                    .disabled(self.busy())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.mfa_method = method.clone();
                        this.secret.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    })),
                );
            }
            if self.mfa_methods.len() > 1 {
                content = content.child(choices);
            }
        }
        let footer = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(if self.stage == Stage::Mfa {
                theme.space2
            } else {
                10.
            }))
            .mt(px(if self.stage == Stage::Mfa {
                theme.space1
            } else {
                2.
            }))
            .child(
                Button::new("member-prompt-cancel", theme)
                    .variant(ButtonVariant::Secondary)
                    .label(if self.stage == Stage::Mfa {
                        t("common.cancel")
                    } else {
                        "Cancel".into()
                    })
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_prompt(window, cx);
                    })),
            )
            .child(
                Button::new("member-prompt-submit", theme)
                    .variant(if matches!(self.stage, Stage::Report | Stage::Mfa) {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::DangerSubtle
                    })
                    .label(if self.stage == Stage::Mfa {
                        t(if self.busy() {
                            "common.verifying"
                        } else {
                            "common.confirm"
                        })
                    } else if self.busy() {
                        "Working…".into()
                    } else if matches!(self.stage, Stage::Mfa | Stage::Confirm(Action::Transfer)) {
                        t("servers.transfer")
                    } else {
                        action.to_string()
                    })
                    .disabled(
                        self.busy()
                            || (self.stage == Stage::Mfa
                                && self.secret.read(cx).value().trim().is_empty()),
                    )
                    .on_click(cx.listener(|this, _, window, cx| match this.stage {
                        Stage::Confirm(action) => this.confirm(action, cx),
                        Stage::Report => this.submit_report(cx),
                        Stage::Mfa => this.submit_transfer(window, cx),
                        _ => {}
                    })),
            );
        if self.stage == Stage::Mfa {
            let label = t(&format!("settings.mfa.fields.{}", self.mfa_method));
            let mut form = div()
                .flex()
                .flex_col()
                .gap(px(theme.space2))
                .child(input_field(
                    &theme,
                    &label,
                    "",
                    SettingsInput::new(&self.secret, theme)
                        .label(label.clone())
                        .password(self.mfa_method == "password")
                        .disabled(self.busy()),
                ));
            if let Some(error) = &self.error {
                form = form.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
            }
            content = content.child(form.child(footer));
        } else {
            content = content.child(footer);
        }
        UiDialog::new("member-action-prompt", theme, title)
            .width(if self.stage == Stage::Mfa { 430. } else { 460. })
            .padding(if self.stage == Stage::Mfa {
                theme.space5
            } else {
                28.
            })
            .gap(if self.stage == Stage::Mfa {
                theme.space4
            } else {
                theme.space5
            })
            .show_close(false)
            .description(description)
            .focus(&self.focus)
            .busy(self.busy())
            .on_close(cx.listener(|this, _, window, cx| this.cancel_prompt(window, cx)))
            .child(content)
            .into_any_element()
    }
}
