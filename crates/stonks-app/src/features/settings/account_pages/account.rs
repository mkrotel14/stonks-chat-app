use super::*;

impl AccountPage {
    pub(super) fn account(
        &self,
        user: &ClientUser,
        info: &AccountInfo,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let busy = self.pending.is_some();
        let handle = match &user.discriminator {
            Some(tag) => format!("{}#{tag}", user.username),
            None => user.username.clone(),
        };
        let masked = "••••••••";
        let email = if self.email_revealed {
            &info.email
        } else {
            masked
        };
        let mut section = settings_section(&theme, &t("settings.account.signIn"));
        for (id, label, description, action) in [
            (
                "username",
                "settings.account.username",
                handle.as_str(),
                Action::Username,
            ),
            ("email", "settings.account.email", email, Action::Email),
            (
                "password",
                "settings.account.password",
                masked,
                Action::Password,
            ),
        ] {
            let mut trailing = div().flex().flex_wrap().gap(px(theme.space2));
            if id == "email" {
                trailing = trailing.child(
                    Button::new("account-reveal-email", theme)
                        .variant(ButtonVariant::Quiet)
                        .label(t(if self.email_revealed {
                            "settings.account.hide"
                        } else {
                            "settings.account.reveal"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.email_revealed = !this.email_revealed;
                            cx.notify();
                        })),
                );
            }
            let title = t(action.label());
            trailing = trailing.child(
                Button::new(id, theme)
                    .variant(ButtonVariant::Secondary)
                    .label(title)
                    .disabled(busy || self.editor.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let methods = match &this.data {
                            Some(Data::Account(_, info))
                                if matches!(&action, Action::Email)
                                    && info.authenticator_enabled =>
                            {
                                info.methods()
                            }
                            _ => &[],
                        };
                        this.open_editor(action.clone(), methods, window, cx);
                    })),
            );
            section = section.child(settings_row(&theme, &t(label), description, trailing));
        }
        section
            .child(
                settings_section(&theme, &t("settings.account.security"))
                    .child(settings_row(
                        &theme,
                        &t("settings.account.recoveryCodes"),
                        &t(if info.recovery_enabled {
                            "settings.account.configured"
                        } else {
                            "settings.account.notConfigured"
                        }),
                        div(),
                    ))
                    .child(settings_row(
                        &theme,
                        &t("settings.account.authenticator"),
                        &t(if info.authenticator_enabled {
                            "settings.account.enabled"
                        } else {
                            "settings.account.notConfigured"
                        }),
                        div(),
                    ))
                    .child(settings_row(
                        &theme,
                        &t("settings.pages.sessions"),
                        &t("settings.account.sessionsDescription"),
                        Button::new("account-sessions", theme)
                            .variant(ButtonVariant::Quiet)
                            .label(t("settings.pages.sessions"))
                            .disabled(busy || self.editor.is_some() || self.open_sessions.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(open) = this.open_sessions.clone() {
                                    window.defer(cx, move |window, cx| open(window, cx));
                                }
                            })),
                    )),
            )
            .child(
                settings_section(&theme, &t("settings.account.management")).children(
                    [AccountManagement::Disable, AccountManagement::Delete]
                        .into_iter()
                        .map(|management| {
                            let action = Action::Manage(management);
                            let description = match management {
                                AccountManagement::Disable => "settings.account.disableDescription",
                                AccountManagement::Delete => "settings.account.deleteDescription",
                            };
                            let label = action.label();
                            let methods = info.methods();
                            settings_row(
                                &theme,
                                &t(label),
                                &t(description),
                                Button::new(label, theme)
                                    .variant(ButtonVariant::DangerQuiet)
                                    .label(t(label))
                                    .disabled(
                                        busy || self.editor.is_some()
                                            || self.account_ended.is_none(),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.account_ended.is_some() {
                                            this.open_editor(action.clone(), methods, window, cx);
                                        }
                                    })),
                            )
                        }),
                ),
            )
    }
    pub(super) fn open_editor(
        &mut self,
        action: Action,
        methods: &'static [&'static str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() || self.editor.is_some() {
            return;
        }
        let value = action
            .field()
            .map(|_| cx.new(|cx| InputState::new(window, cx)));
        if let (Some(input), Action::Rename(row)) = (&value, &action) {
            input.update(cx, |input, cx| {
                input.set_value(row.name.clone(), window, cx)
            });
        }
        let password = matches!(&action, Action::Username | Action::Email | Action::Password)
            .then(|| cx.new(|cx| InputState::new(window, cx)));
        let verification = (!methods.is_empty()).then(|| Verification::new(methods, window, cx));
        let first_input = value
            .as_ref()
            .or(password.as_ref())
            .or(verification.as_ref().map(|v| &v.input));
        let focus = cx.focus_handle();
        first_input
            .map_or_else(
                || focus.clone(),
                |state| Focusable::focus_handle(state.read(cx), cx),
            )
            .focus(window, cx);
        let subscriptions = value
            .iter()
            .chain(password.iter())
            .chain(verification.iter().map(|v| &v.input))
            .map(|state| cx.subscribe(state, |_, _, _: &InputEvent, cx| cx.notify()))
            .collect();
        self.editor = Some(Editor {
            action,
            value,
            password,
            verification,
            focus,
            _subscriptions: subscriptions,
        });
        self.notice = None;
        self.error = None;
        cx.notify();
    }
    pub(super) fn editor(
        &self,
        editor: &Editor,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> stonks_ui::components::UiDialog {
        let busy = self.pending.is_some();
        let field = |label: &str, state: &Entity<InputState>, password| {
            let label = t(label);
            input_field(
                &theme,
                &label,
                "",
                SettingsInput::new(state, theme)
                    .label(label.clone())
                    .password(password)
                    .disabled(busy),
            )
        };
        let title = match editor.action {
            Action::Manage(AccountManagement::Disable) => "settings.account.confirmDisableTitle",
            Action::Manage(AccountManagement::Delete) => "settings.account.confirmDeleteTitle",
            _ => editor.action.label(),
        };
        let mut section = stonks_ui::components::UiDialog::new("account-editor", theme, t(title))
            .focus(&editor.focus)
            .busy(busy)
            .on_close(cx.listener(|this, _, _, cx| this.cancel_editor(cx)));
        if let Some(error) = &self.error {
            section = section.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        match &editor.action {
            Action::Manage(action) => {
                section = section.description(t(match action {
                    AccountManagement::Disable => "settings.confirm.disableAccount",
                    AccountManagement::Delete => "settings.confirm.deleteAccount",
                }));
            }
            Action::Rename(_) => section = section.child(t("settings.sessions.renameDescription")),
            Action::Revoke(row) => {
                section = section.child(t_vars(
                    "settings.confirm.revokeSession",
                    &[("name", &row.name)],
                ))
            }
            Action::RevokeOthers => {
                section = section.child(t("settings.confirm.revokeOtherSessions"))
            }
            _ => {}
        }
        if let (Some(key), Some(state)) = (editor.action.field(), &editor.value) {
            section = section.child(field(
                key,
                state,
                matches!(&editor.action, Action::Password),
            ));
        }
        if let Some(password) = &editor.password {
            section = section.child(field("settings.account.currentPassword", password, true));
        }
        if let Some(v) = &editor.verification {
            section = section.child(v.render(self, theme, cx));
        }
        section.child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(theme.space2))
                .child(
                    Button::new("account-editor-cancel", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("common.cancel"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_editor(cx))),
                )
                .child(
                    Button::new("account-editor-submit", theme)
                        .variant(
                            if matches!(
                                &editor.action,
                                Action::Revoke(_) | Action::RevokeOthers | Action::Manage(_)
                            ) {
                                ButtonVariant::Danger
                            } else {
                                ButtonVariant::Primary
                            },
                        )
                        .label(t(if busy {
                            "common.working"
                        } else if matches!(&editor.action, Action::Rename(_)) {
                            "common.save"
                        } else {
                            editor.action.label()
                        }))
                        .disabled(busy || !editor.ready(cx))
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
        )
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        if !editor.ready(cx)
            || (matches!(editor.action, Action::Manage(_)) && self.account_ended.is_none())
        {
            return;
        }
        let action = editor.action.clone();
        let read = |state: &Option<Entity<InputState>>| {
            state
                .as_ref()
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default()
        };
        let value = read(&editor.value);
        let password = read(&editor.password);
        let verification = editor.verification.as_ref().map(|v| v.value(cx));
        let service = self.service.clone();
        self.run(
            window,
            cx,
            async move {
                let confirmation = value.as_str();
                let verification = verification
                    .as_ref()
                    .map(|(method, value)| (method.as_str(), value.as_str()));
                match &action {
                    Action::Username => service.change_username(&value, &password).await?,
                    Action::Email => {
                        service
                            .change_email(&value, &password, verification)
                            .await?
                    }
                    Action::Password => service.change_password(&password, &value).await?,
                    Action::Rename(row) => service.rename_session(&row.id, &value).await?,
                    Action::Revoke(row) => service.revoke_session(&row.id).await?,
                    Action::RevokeOthers | Action::Manage(_) => {
                        let (method, value) = verification.ok_or_else(|| {
                            CommandError::new(
                                stonks_core::error::ErrorCode::InvalidInput,
                                "Verification is required.",
                            )
                        })?;
                        match action {
                            Action::Manage(management) => {
                                service
                                    .manage_account(management, &confirmation, method, value)
                                    .await?
                            }
                            _ => service.revoke_other_sessions(method, value).await?,
                        }
                    }
                }
                // The mutation already succeeded. A failed follow-up GET must not
                // leave the confirmation open and invite a duplicate submission.
                let refreshed_user = if matches!(&action, Action::Username) {
                    service.me().await.ok()
                } else {
                    None
                };
                let refreshed_sessions = if matches!(&action, Action::RevokeOthers) {
                    Some(service.sessions().await)
                } else {
                    None
                };
                let projected_value = if matches!(
                    &action,
                    Action::Username | Action::Email | Action::Rename(_)
                ) {
                    value.trim().to_owned()
                } else {
                    String::new()
                };
                Ok((action, projected_value, refreshed_user, refreshed_sessions))
            },
            |this, (action, value, refreshed_user, refreshed_sessions), window, cx| {
                this.editor = None;
                this.notice = match &action {
                    Action::Username => Some(t("settings.notices.usernameUpdated")),
                    Action::Email => Some(t("settings.notices.emailUpdated")),
                    Action::Password => Some(t("settings.notices.passwordUpdated")),
                    _ => None,
                };
                if matches!(action, Action::Manage(_)) {
                    this.ended = true;
                    this.data = None;
                    if let Some(ended) = this.account_ended.take() {
                        window.defer(cx, move |window, cx| ended(window, cx));
                    }
                    return;
                }
                match (&mut this.data, action) {
                    (Some(Data::Account(user, _)), Action::Username) => {
                        if let Some(fresh) = refreshed_user {
                            *user = fresh;
                        } else {
                            user.username = value.trim().to_owned();
                        }
                        cx.emit(AccountPageEvent::UsernameChanged);
                    }
                    (Some(Data::Account(_, info)), Action::Email) => {
                        info.email = value;
                        this.email_revealed = true;
                    }
                    (Some(Data::Sessions(rows)), Action::Rename(target)) => {
                        if let Some(row) = rows.iter_mut().find(|row| row.id == target.id) {
                            row.name = value.trim().into();
                        }
                    }
                    (Some(Data::Sessions(rows)), Action::Revoke(target)) => {
                        rows.retain(|row| row.id != target.id)
                    }
                    (Some(Data::Sessions(rows)), Action::RevokeOthers) => {
                        match refreshed_sessions {
                            Some(Ok(fresh)) => *rows = fresh,
                            Some(Err(error)) => {
                                this.data = None;
                                this.error = Some(error.to_string());
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
            },
        );
    }
}
impl Action {
    fn label(&self) -> &'static str {
        match self {
            Self::Username => "settings.account.updateUsername",
            Self::Email => "settings.account.updateEmail",
            Self::Password => "settings.account.updatePassword",
            Self::Rename(_) => "settings.sessions.rename",
            Self::Revoke(_) => "settings.sessions.revoke",
            Self::Manage(AccountManagement::Disable) => "settings.account.disable",
            Self::Manage(AccountManagement::Delete) => "settings.account.delete",
            Self::RevokeOthers => "settings.sessions.revokeOthers",
        }
    }
    fn field(&self) -> Option<&'static str> {
        Some(match self {
            Self::Username => "settings.account.newUsername",
            Self::Email => "settings.account.newEmail",
            Self::Password => "settings.account.newPassword",
            Self::Rename(_) => "settings.sessions.name",
            Self::Manage(AccountManagement::Delete) => "settings.account.typeDeleteLabel",
            _ => return None,
        })
    }
}

impl Editor {
    fn ready(&self, cx: &App) -> bool {
        let nonblank = |state: &Entity<InputState>| !state.read(cx).value().trim().is_empty();
        self.value.as_ref().is_none_or(nonblank)
            && self.password.as_ref().is_none_or(nonblank)
            && self
                .verification
                .as_ref()
                .is_none_or(|v| nonblank(&v.input))
            && (!matches!(self.action, Action::Manage(AccountManagement::Delete))
                || self
                    .value
                    .as_ref()
                    .is_some_and(|v| v.read(cx).value().as_str() == "DELETE"))
    }
}
