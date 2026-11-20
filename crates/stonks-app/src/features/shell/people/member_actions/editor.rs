use super::profile::{profile_avatar_url, role_colour};
use super::*;
impl MemberActions {
    pub(super) fn start_edit(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let member = self.member(cx);
        self.nickname.update(cx, |input, cx| {
            input.set_value(
                member
                    .as_ref()
                    .and_then(|member| member.nickname.clone())
                    .unwrap_or_default(),
                window,
                cx,
            );
        });
        self.pronouns.update(cx, |input, cx| {
            input.set_value(
                member
                    .as_ref()
                    .and_then(|member| member.pronouns.clone())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.roles = member
            .as_ref()
            .map(|member| member.roles.iter().cloned().collect())
            .unwrap_or_default();
        self.initial_identity = IdentityDraft {
            nickname: self.nickname.read(cx).value().to_string(),
            pronouns: self.pronouns.read(cx).value().to_string(),
        };
        self.avatar_file = None;
        self.avatar_preview = None;
        self.preview_task = None;
        self.preview_revision = self.preview_revision.wrapping_add(1);
        let user = self.user(cx);
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
        self.avatar_url = user
            .as_ref()
            .and_then(|user| profile_avatar_url(autumn, api, user, member.as_ref()));
        self.stage = if action == Action::Identity {
            Stage::Identity
        } else {
            Stage::Roles
        };
        if self.stage == Stage::Identity && self.name_editable(cx) {
            self.nickname
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus.focus(window, cx);
        }
    }
    fn name_editable(&self, cx: &App) -> bool {
        self.can(
            if self.self_target(cx) {
                service::CHANGE_NICKNAME
            } else {
                service::MANAGE_NICKNAMES
            },
            cx,
        )
    }
    fn avatar_editable(&self, cx: &App) -> bool {
        self.can(
            if self.self_target(cx) {
                service::CHANGE_AVATAR
            } else {
                service::MANAGE_NICKNAMES
            },
            cx,
        )
    }
    fn pick_avatar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !self.avatar_editable(cx) {
            return;
        }
        let generation = self.gate.generation();
        let task = crate::shared::file_picker::pick_image(
            window,
            cx,
            move |this: &mut Self, result, _, cx| {
                if this.gate.generation() != generation || this.stage != Stage::Identity {
                    return;
                }
                match result {
                    Ok(Some(file)) => {
                        if file.bytes.len() > 8_000_000 {
                            this.error = Some("Choose a valid image smaller than 8 MB.".into());
                            cx.notify();
                            return;
                        }
                        this.preview_revision = this.preview_revision.wrapping_add(1);
                        let revision = this.preview_revision;
                        let bytes = file.bytes.clone();
                        this.avatar_file = Some(file);
                        this.avatar_preview = None;
                        this.error = None;
                        this.preview_task = Some(cx.spawn(async move |this, cx| {
                            let frames = crate::shared::images::preview_frames(bytes, 152).await;
                            this.update(cx, |this, cx| {
                                if this.gate.generation() != generation
                                    || this.preview_revision != revision
                                {
                                    return;
                                }
                                match frames {
                                    Some(frames) => {
                                        this.avatar_preview =
                                            Some(Arc::new(RenderImage::new(frames)))
                                    }
                                    None => {
                                        this.avatar_file = None;
                                        this.error =
                                            Some("Could not preview the selected image.".into());
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                        }));
                        cx.notify();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            },
        );
        self.picker = Some(task);
    }
    pub(super) fn submit_edit(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        let patch = if self.stage == Stage::Roles {
            if !self.can(service::ASSIGN_ROLES, cx) {
                self.error = Some("You no longer have permission to assign roles.".into());
                cx.notify();
                return;
            }
            let mut roles: Vec<_> = self.roles.iter().cloned().collect();
            roles.sort();
            MemberPatch {
                roles: Some(roles),
                ..Default::default()
            }
        } else {
            let names = self.name_editable(cx);
            let avatar = self.avatar_editable(cx);
            if let Err(error) = self.initial_identity.authorize(
                &self.nickname.read(cx).value(),
                &self.pronouns.read(cx).value(),
                names,
                self.avatar_file.is_some(),
                avatar,
            ) {
                self.error = Some(error.into());
                cx.notify();
                return;
            }
            MemberPatch {
                nickname: names.then(|| self.nickname.read(cx).value().trim().to_owned()),
                pronouns: names.then(|| {
                    let value = self.pronouns.read(cx).value().trim().to_owned();
                    (!value.is_empty()).then_some(value)
                }),
                avatar: (avatar && self.avatar_file.is_some())
                    .then(|| Some("pendingUpload".into())),
                ..Default::default()
            }
        };
        if let Err(error) = patch.body() {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let policy = self.policy.clone();
        let authorization = self.session.read(cx).authorization();
        let request_target = target.clone();
        let mut request_patch = patch;
        let file = request_patch
            .avatar
            .is_some()
            .then(|| self.avatar_file.clone())
            .flatten();
        let self_edit = self.self_target(cx);
        self.run(
            async move {
                service::authorize_edit(
                    &authorization,
                    &request_target.server,
                    self_edit,
                    &request_patch,
                )?;
                if let Some(file) = file {
                    request_patch.avatar = Some(Some(
                        stonks_core::uploads::upload_image(&policy, "avatars", &file).await?,
                    ));
                }
                service::edit_member(
                    &policy,
                    &authorization,
                    &request_target.server,
                    &request_target.user,
                    self_edit,
                    &request_patch,
                )
                .await?;
                Ok(request_patch)
            },
            move |this, result, cx| match result {
                Ok(patch) => {
                    cx.emit(MemberActionsEvent::UpdatedMember {
                        server: target.server,
                        user: target.user,
                        patch,
                    });
                    this.finish_close(cx);
                }
                Err(error) => this.error = Some(error),
            },
            cx,
        );
    }
    pub(super) fn render_editor(
        &mut self,
        theme: Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let server = self.server(cx);
        let roles_mode = self.stage == Stage::Roles;
        let title = if roles_mode {
            "Edit roles".into()
        } else {
            format!(
                "Edit identity in {}",
                server
                    .as_ref()
                    .map(|server| server.name.as_str())
                    .unwrap_or("this server")
            )
        };
        let mut content = div().flex().flex_col().gap(px(theme.space4));
        if let Some(error) = &self.error {
            content = content.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if roles_mode {
            let mut roles = server
                .as_ref()
                .map(|server| server.roles.iter().collect::<Vec<_>>())
                .unwrap_or_default();
            roles.sort_by_key(|(_, role)| role.rank.unwrap_or(0));
            let mut list = div()
                .id("member-role-list")
                .flex()
                .flex_col()
                .max_h(px(340.))
                .overflow_y_scroll()
                .gap(px(theme.space2));
            if roles.is_empty() {
                list = list.child(
                    div()
                        .text_size(px(theme.font_size_sm))
                        .text_color(theme.text_muted)
                        .child("This server has no roles."),
                );
            }
            for (id, role) in roles {
                let role_id = id.clone();
                let checked = self.roles.contains(id);
                let disabled = self.busy() || !self.can(service::ASSIGN_ROLES, cx);
                list = list.child(
                    div().min_h(px(48.)).child(
                        Button::new(
                            ElementId::Name(format!("member-role-row-{id}").into()),
                            theme,
                        )
                        .variant(ButtonVariant::MemberRole)
                        .disabled(disabled)
                        .label(role.name.clone())
                        .leading(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme.space3))
                                .child(
                                    Checkbox::new(
                                        ElementId::Name(format!("member-role-{id}").into()),
                                        theme,
                                    )
                                    .checked(checked)
                                    .disabled(disabled),
                                )
                                .child(
                                    div()
                                        .size(px(12.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(role_colour(role.colour.as_deref(), theme.text_muted)),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !checked {
                                this.roles.insert(role_id.clone());
                            } else {
                                this.roles.remove(&role_id);
                            }
                            cx.notify();
                        })),
                    ),
                );
            }
            content = content.child(list);
        } else {
            self.images.update(cx, |images, _| {
                images.set_sources(
                    "member-profile",
                    self.avatar_url.iter().cloned().map(|url| (url, 152)),
                )
            });
            let image = self.avatar_preview.clone().or_else(|| {
                self.images.update(cx, |images, cx| {
                    images.get(self.avatar_url.as_deref(), 152, cx)
                })
            });
            let nickname = self.nickname.read(cx).value().to_string();
            let pronouns = self.pronouns.read(cx).value().to_string();
            content = content
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme.space2))
                        .text_size(px(theme.font_size_sm))
                        .text_color(theme.text_muted)
                        .child("Server avatar")
                        .child(
                            Button::new("member-avatar-change", theme)
                                .variant(ButtonVariant::MemberAvatar)
                                .leading(avatar(&theme, &nickname, 76., image))
                                .label("Change")
                                .disabled(self.busy() || !self.avatar_editable(cx))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.pick_avatar(window, cx)),
                                ),
                        ),
                )
                .child(input_field(
                    &theme,
                    "Server nickname",
                    &format!("{}/32", field_length(&nickname)),
                    SettingsInput::new(&self.nickname, theme)
                        .label("Server nickname")
                        .disabled(self.busy() || !self.name_editable(cx)),
                ))
                .child(input_field(
                    &theme,
                    "Pronouns",
                    &format!("{}/24", field_length(&pronouns)),
                    SettingsInput::new(&self.pronouns, theme)
                        .label("Pronouns")
                        .disabled(self.busy() || !self.name_editable(cx)),
                ));
        }
        let footer = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(theme.space2))
            .child(
                Button::new("member-editor-cancel", theme)
                    .variant(ButtonVariant::Secondary)
                    .label("Cancel")
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .child(
                Button::new("member-editor-save", theme)
                    .label(if self.busy() { "Saving…" } else { "Save" })
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, _, cx| this.submit_edit(cx))),
            );
        UiDialog::new("member-editor", theme, title)
            .width(480.)
            .show_close(false)
            .focus(&self.focus)
            .busy(self.busy())
            .on_close(cx.listener(|this, _, window, cx| this.close(window, cx)))
            .child(content.child(footer))
            .into_any_element()
    }
}
