use super::extra::Dialog;
use super::*;
use stonks_core::{
    connection::{ClientRole, ConnectionManager, PermissionOverride},
    uploads::{self, UploadFile},
};

pub enum Mutation {
    Icon(UploadFile),
    RoleIcon(String, UploadFile),
    CreateRole(String),
    SaveRole(String, service::RolePatch, Option<PermissionOverride>),
    RolePermissions(String, PermissionOverride),
    DefaultPermissions(PermissionOverride),
    DeleteRole(String),
    CreateEmoji(String, UploadFile),
    RenameEmoji(String, String),
    DeleteEmoji(String),
    CreateInvite(String, service::InviteOptions),
    RevokeInvite(String),
}
enum Completion {
    Server(ClientServer, Option<String>),
    CreatedRole(String, ClientRole),
    DeletedRole(String),
    RoleIcon(String, String),
    Emoji(ServerEmoji, bool),
    DeletedEmoji(String),
    Invite(ServerInvite),
    Revoked(String),
}
impl Mutation {
    fn permission(&self) -> u64 {
        match self {
            Self::Icon(_) => 1 << 1,
            Self::RevokeInvite(_) => 0,
            Self::RoleIcon(..) | Self::CreateRole(_) | Self::DeleteRole(_) => 1 << 3,
            Self::SaveRole(_, _, permissions) => {
                (1 << 3) | if permissions.is_some() { 1 << 2 } else { 0 }
            }
            Self::RolePermissions(..) => 1 << 2,
            Self::DefaultPermissions(_) => 1 << 2,
            Self::CreateEmoji(..) | Self::RenameEmoji(..) | Self::DeleteEmoji(_) => 1 << 4,
            Self::CreateInvite(..) => 0,
        }
    }
}
impl ServerSettings {
    pub fn set_authorization(
        &mut self,
        authorization: Arc<ConnectionManager>,
        cx: &mut Context<Self>,
    ) {
        self.extra.authorization = Some(authorization);
        cx.notify();
    }
    pub fn set_user_names(
        &mut self,
        users: std::collections::HashMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.extra.users = users;
        cx.notify();
    }
    pub(super) fn allowed(&self, mask: u64) -> bool {
        self.extra
            .authorization
            .as_ref()
            .is_some_and(|authorization| {
                authorization
                    .require_server_permission(&self.server.id, mask)
                    .is_ok()
            })
    }
    pub(super) fn mutate(
        &mut self,
        mutation: Mutation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        if let Mutation::RevokeInvite(code) = &mutation {
            if !self.can_revoke(code) {
                self.error = Some(t("serverSettings.noPermission"));
                cx.notify();
                return;
            }
        }
        let Some(authorization) = self.extra.authorization.clone() else {
            self.error = Some(t("serverSettings.noPermission"));
            cx.notify();
            return;
        };
        let required = mutation.permission();
        let guard = if let Mutation::CreateInvite(channel, _) = &mutation {
            authorization.require_channel_permission(channel, 1 << 25)
        } else {
            authorization.require_server_permission(&self.server.id, required)
        };
        if let Err(error) = guard {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let role_upload = if let Mutation::RoleIcon(key, _) = &mutation {
            Some(key.clone())
        } else {
            None
        };
        self.extra.roles.uploading_role = role_upload.clone();
        if let Some(key) = &role_upload {
            if let Some(draft) = self.extra.roles.role_drafts.get_mut(key) {
                draft.upload_error = None;
                draft.icon = None;
            }
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        self.save_generation = self.save_generation.wrapping_add(1);
        let generation = self.save_generation;
        let policy = self.policy.clone();
        let server_id = self.server.id.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn_in(window, async move |this, cx| {
            let operation = async move {
                let server_permission =
                    |mask| authorization.require_server_permission(&server_id, mask);
                if !matches!(mutation, Mutation::CreateInvite(..)) {
                    server_permission(required)?;
                }
                let result: Result<Completion, String> = match mutation {
                    Mutation::Icon(file) => {
                        let id = uploads::upload_image(&policy, "icons", &file)
                            .await
                            .map_err(|error| error.to_string())?;
                        server_permission(1 << 1)?;
                        service::set_icon(&policy, &server_id, &id)
                            .await
                            .map(|server| Completion::Server(server, None))
                            .map_err(|error| error.to_string())
                    }
                    Mutation::RoleIcon(key, file) => uploads::upload_image(&policy, "icons", &file)
                        .await
                        .map(|id| Completion::RoleIcon(key, id))
                        .map_err(|error| error.to_string()),
                    Mutation::CreateRole(name) => service::create_role(&policy, &server_id, &name)
                        .await
                        .map(|(id, role)| Completion::CreatedRole(id, role))
                        .map_err(|error| error.to_string()),
                    Mutation::SaveRole(id, patch, permissions) => {
                        service::edit_role(&policy, &server_id, &id, &patch)
                            .await
                            .map_err(|error| error.to_string())?;
                        if let Some(permissions) = permissions {
                            server_permission(1 << 2)?;
                            service::set_permissions(&policy, &server_id, Some(&id), &permissions)
                                .await
                                .map(|server| Completion::Server(server, Some(id)))
                                .map_err(|error| error.to_string())
                        } else {
                            service::refresh_server(&policy, &server_id)
                                .await
                                .map(|server| Completion::Server(server, Some(id)))
                                .map_err(|error| error.to_string())
                        }
                    }
                    Mutation::RolePermissions(id, permissions) => {
                        service::set_permissions(&policy, &server_id, Some(&id), &permissions)
                            .await
                            .map(|server| Completion::Server(server, Some(id)))
                            .map_err(|error| error.to_string())
                    }
                    Mutation::DefaultPermissions(permissions) => {
                        service::set_permissions(&policy, &server_id, None, &permissions)
                            .await
                            .map(|server| Completion::Server(server, Some(String::new())))
                            .map_err(|error| error.to_string())
                    }
                    Mutation::DeleteRole(id) => service::delete_role(&policy, &server_id, &id)
                        .await
                        .map(|_| Completion::DeletedRole(id))
                        .map_err(|error| error.to_string()),
                    Mutation::CreateEmoji(name, file) => {
                        file.validate(2_000_000)
                            .map_err(|error| error.to_string())?;
                        let id = uploads::upload_image(&policy, "emojis", &file)
                            .await
                            .map_err(|error| error.to_string())?;
                        server_permission(1 << 4)?;
                        service::register_emoji(&policy, &server_id, &id, &name)
                            .await
                            .map(|emoji| Completion::Emoji(emoji, true))
                            .map_err(|error| error.to_string())
                    }
                    Mutation::RenameEmoji(id, name) => {
                        service::rename_emoji(&policy, &server_id, &id, &name)
                            .await
                            .map(|emoji| Completion::Emoji(emoji, false))
                            .map_err(|error| error.to_string())
                    }
                    Mutation::DeleteEmoji(id) => service::delete_emoji(&policy, &server_id, &id)
                        .await
                        .map(|_| Completion::DeletedEmoji(id))
                        .map_err(|error| error.to_string()),
                    Mutation::CreateInvite(channel, options) => {
                        authorization.require_channel_permission(&channel, 1 << 25)?;
                        service::create_invite(&policy, &server_id, &channel, &options)
                            .await
                            .map(Completion::Invite)
                            .map_err(|error| error.to_string())
                    }
                    Mutation::RevokeInvite(code) => service::revoke_invite(&policy, &code)
                        .await
                        .map(|_| Completion::Revoked(code))
                        .map_err(|error| error.to_string()),
                };
                result
            };
            let Ok(result) = spawn_core(Abortable::new(operation, registration)).await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if this.save_generation != generation {
                    return;
                }
                this.busy = false;
                this.extra.roles.uploading_role = None;
                match result {
                    Ok(result) => this.complete_mutation(result, window, cx),
                    Err(error) => {
                        if let Some(key) = &role_upload {
                            if let Some(draft) = this.extra.roles.role_drafts.get_mut(key) {
                                draft.upload_error = Some(error);
                            } else {
                                this.error = Some(error);
                            }
                        } else {
                            this.error = Some(error);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.save_request = Some(Request { abort, _task: task });
        cx.notify();
    }
    fn complete_mutation(
        &mut self,
        result: Completion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let notice = match result {
            Completion::Server(server, role) => {
                let notice = match role.as_deref() {
                    None => "serverSettings.serverIconUpdated",
                    Some("") => "serverSettings.defaultsUpdated",
                    Some(_) => "serverSettings.roleUpdated",
                };
                if let Some(role) = role {
                    self.extra.roles.role_drafts.remove(&role);
                }
                self.publish(server, cx);
                notice
            }
            Completion::CreatedRole(id, role) => {
                self.server.roles.insert(id.clone(), role);
                self.extra.roles.expanded.insert(id);
                cx.emit(ServerSettingsEvent::Updated(self.server.clone()));
                self.close_dialog(window, cx);
                self.extra
                    .roles
                    .role_name
                    .update(cx, |input, cx| input.set_value("", window, cx));
                "serverSettings.roleCreated"
            }
            Completion::DeletedRole(id) => {
                self.server.roles.remove(&id);
                self.extra.roles.role_drafts.remove(&id);
                self.extra.roles.expanded.remove(&id);
                cx.emit(ServerSettingsEvent::Updated(self.server.clone()));
                self.close_dialog(window, cx);
                "serverSettings.roleDeleted"
            }
            Completion::RoleIcon(key, id) => {
                if let Some(draft) = self.extra.roles.role_drafts.get_mut(&key) {
                    draft.icon_url = media::file_url(self.autumn.as_deref(), "icons", &id);
                    draft.icon = Some(id);
                    draft.upload_error = None;
                }
                self.notice = None;
                return;
            }
            Completion::Emoji(emoji, created) => {
                let items = self.emojis.get_or_insert_with(Vec::new);
                if created {
                    items.push(emoji);
                } else {
                    if let Some(old) = items.iter_mut().find(|old| old.id == emoji.id) {
                        *old = emoji.clone();
                    }
                    self.extra.dialog = Some(Dialog::Emoji(emoji));
                }
                if created {
                    self.extra.emoji.emoji_file = None;
                    self.extra.emoji.emoji_preview = None;
                    self.extra
                        .emoji
                        .emoji_name
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                cx.emit(ServerSettingsEvent::EmojisChanged);
                if created {
                    "serverSettings.emojiCreated"
                } else {
                    "serverSettings.emojiRenamed"
                }
            }
            Completion::DeletedEmoji(id) => {
                if let Some(items) = &mut self.emojis {
                    items.retain(|emoji| emoji.id != id);
                }
                self.close_dialog(window, cx);
                cx.emit(ServerSettingsEvent::EmojisChanged);
                "serverSettings.emojiDeleted"
            }
            Completion::Invite(invite) => {
                self.invites
                    .get_or_insert_with(Vec::new)
                    .push(invite.clone());
                let url = self.invite_url(&invite.code).unwrap_or_default();
                self.extra
                    .invites
                    .invite_link
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                self.extra.invites.created_invite = Some(invite);
                "serverSettings.inviteCreated"
            }
            Completion::Revoked(code) => {
                if let Some(items) = &mut self.invites {
                    items.retain(|invite| invite.code != code);
                }
                self.extra.invites.created_invite = None;
                self.close_dialog(window, cx);
                "serverSettings.inviteRevoked"
            }
        };
        self.notice = Some(t(notice));
    }
    fn publish(&mut self, server: ClientServer, cx: &mut Context<Self>) {
        self.icon_url = server
            .icon
            .as_ref()
            .and_then(|file| media::file_url(self.autumn.as_deref(), &file.tag, &file.id));
        self.server = server.clone();
        cx.emit(ServerSettingsEvent::Updated(server));
    }
    pub(super) fn pick_icon(
        &mut self,
        role: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.allowed(if role.is_some() { 1 << 3 } else { 1 << 1 }) {
            return;
        }
        self.busy = true;
        self.extra.picker_generation = self.extra.picker_generation.wrapping_add(1);
        let generation = self.extra.picker_generation;
        let task = crate::shared::file_picker::pick_cropped_image(
            window,
            cx,
            move |this, result, window, cx| {
                if this.extra.picker_generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(Some(file)) => {
                        if let Some(key) = role.clone() {
                            let bytes = file.bytes.clone();
                            this.extra.emoji.preview_task =
                                Some(cx.spawn(async move |this, cx| {
                                    let frames = spawn_core(crate::shared::images::preview_frames(
                                        bytes, 72,
                                    ))
                                    .await;
                                    this.update(cx, |this, cx| {
                                        if this.extra.picker_generation != generation {
                                            return;
                                        }
                                        if let (Some(frames), Some(draft)) =
                                            (frames, this.extra.roles.role_drafts.get_mut(&key))
                                        {
                                            draft.preview =
                                                Some(Arc::new(RenderImage::new(frames)));
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                                }));
                        }
                        this.mutate(
                            if let Some(key) = role {
                                Mutation::RoleIcon(key, file)
                            } else {
                                Mutation::Icon(file)
                            },
                            window,
                            cx,
                        );
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            },
        );
        self.extra.picker = Some(task);
        cx.notify();
    }
}
