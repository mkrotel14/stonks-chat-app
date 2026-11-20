//! Reconcile gateway snapshots without discarding an administrator's draft.
use super::extra::Dialog;
use super::*;
use stonks_core::connection::PermissionOverride;
impl ServerSettings {
    /// The host can forward snapshots without borrowing a Window. Apply the last
    /// queued snapshot atomically on the next render, when inputs/selects can sync.
    pub fn refresh(
        &mut self,
        server: ClientServer,
        channels: Vec<ClientChannel>,
        cx: &mut Context<Self>,
    ) {
        if server.id != self.server.id {
            return;
        }
        let invite_changed = matches!(self.extra.dialog, Some(Dialog::CreateInvite))
            && self.extra.invites.created_invite.is_none()
            && self.eligible_invites(&channels) != self.extra.invites.invite_eligible;
        if !invite_changed
            && serde_json::to_value(&server).ok() == serde_json::to_value(&self.server).ok()
            && serde_json::to_value(&channels).ok() == serde_json::to_value(&self.channels).ok()
        {
            self.extra.pending_snapshot = None;
            return;
        }
        self.extra.pending_snapshot = Some((server, channels));
        cx.notify();
    }
    pub(super) fn apply_pending_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((server, channels)) = self.extra.pending_snapshot.take() else {
            return;
        };
        if server.id != self.server.id {
            return;
        }
        let draft = self.draft(cx);
        let incoming = Overview::from_server(&server);
        if draft.name == self.initial.name && draft.name != incoming.name {
            self.name.update(cx, |input, cx| {
                input.set_value(incoming.name.clone(), window, cx)
            });
        }
        if draft.description == self.initial.description
            && draft.description != incoming.description
        {
            self.description.update(cx, |input, cx| {
                input.set_value(incoming.description.clone(), window, cx)
            });
        }
        for (key, select) in SYSTEM_EVENTS.iter().zip(&self.systems) {
            let empty = String::new();
            let old = self.initial.system_messages.get(*key).unwrap_or(&empty);
            let local = draft.system_messages.get(*key).unwrap_or(&empty);
            let desired = if local == old {
                incoming.system_messages.get(*key).unwrap_or(&empty)
            } else {
                local
            }
            .clone();
            let mut options = vec![UiSelectOption::new("", t("serverSettings.disabled"))];
            options.extend(
                channels
                    .iter()
                    .filter(|channel| overview::text_channel(channel, &server.id))
                    .map(|channel| {
                        UiSelectOption::new(
                            channel.id.clone(),
                            format!("# {}", channel.name.as_deref().unwrap_or("")),
                        )
                    }),
            );
            if !desired.is_empty()
                && !channels.iter().any(|channel| {
                    channel.id == desired && overview::text_channel(channel, &server.id)
                })
            {
                options.push(UiSelectOption::new(
                    desired.clone(),
                    t("serverSettings.unknownChannel"),
                ));
            }
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(options), window, cx);
                select.set_selected_value(&SharedString::from(desired), window, cx);
            });
        }
        self.extra
            .roles
            .role_drafts
            .retain(|id, _| id.is_empty() || server.roles.contains_key(id));
        self.extra
            .roles
            .expanded
            .retain(|id| id.is_empty() || server.roles.contains_key(id));
        for (id, draft) in &mut self.extra.roles.role_drafts {
            if id.is_empty() {
                draft.permissions = roles::merge_permissions(
                    &draft.permissions,
                    &PermissionOverride {
                        a: self.server.default_permissions,
                        d: 0,
                    },
                    &PermissionOverride {
                        a: server.default_permissions,
                        d: 0,
                    },
                    true,
                );
                continue;
            }
            let (Some(before), Some(after)) = (self.server.roles.get(id), server.roles.get(id))
            else {
                continue;
            };
            if draft.name.read(cx).value().as_ref() == before.name && before.name != after.name {
                draft.name.update(cx, |input, cx| {
                    input.set_value(after.name.clone(), window, cx)
                });
            }
            let previous = before.colour.as_deref().unwrap_or("#5865f2");
            let dirty_colour =
                draft.colour_changed && draft.colour.read(cx).value().as_ref() != previous;
            if !dirty_colour && before.colour != after.colour {
                let colour = after.colour.clone().unwrap_or_else(|| "#5865f2".into());
                draft
                    .colour
                    .update(cx, |input, cx| input.set_value(colour.clone(), window, cx));
                draft.picker.update(cx, |picker, cx| {
                    picker.set_value(
                        super::roles::colour(&colour).unwrap_or_else(|| rgb(0x5865f2)),
                        window,
                        cx,
                    )
                });
                draft.colour_changed = after.colour.is_none();
            }
            if draft.hoist == before.hoist {
                draft.hoist = after.hoist;
            }
            draft.permissions = roles::merge_permissions(
                &draft.permissions,
                &before.permissions,
                &after.permissions,
                false,
            );
        }
        self.initial = incoming;
        self.channels = channels;
        self.icon_url = server
            .icon
            .as_ref()
            .and_then(|file| media::file_url(self.autumn.as_deref(), &file.tag, &file.id));
        self.server = server;
        if matches!(self.extra.dialog, Some(Dialog::CreateInvite))
            && self.extra.invites.created_invite.is_none()
        {
            self.refresh_invite_options(window, cx);
        }
    }
}
