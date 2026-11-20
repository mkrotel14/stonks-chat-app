//! Synchronize the composer with channel selection and attachment access.

use super::super::{navigation::view::channel_label, *};
use crate::features::conversation::composer::AttachmentAccess;
use stonks_core::connection::{AppSnapshot, ClientChannel};
use stonks_core::server::ServerSelection;

impl ShellView {
    pub(in crate::features::shell) fn refresh_emoji_consumers(&mut self, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.invalidate_emojis(cx));
        self.timeline
            .update(cx, |timeline, cx| timeline.invalidate_emojis(cx));
    }

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_channel.is_none() {
            return;
        }
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    pub(in crate::features::shell) fn sync_composer(
        &self,
        channel: Option<&ClientChannel>,
        snapshot: Option<&AppSnapshot>,
        deployment: Option<&ServerSelection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((channel, snapshot)) = channel.zip(snapshot) {
            const SEND: u64 = 1 << 22;
            const UPLOAD: u64 = 1 << 27;
            let session = self.session.read(cx);
            let allows = |mask| {
                session.can_channel_permission(&channel.id, mask)
                    && group_allows(channel, &snapshot.current_user, mask)
            };
            let access = AttachmentAccess {
                channel: channel.id.clone(),
                can_send: allows(SEND),
                can_upload: allows(SEND | UPLOAD),
                size_limit: deployment
                    .filter(|server| {
                        server.endpoints.autumn.is_some()
                            && stonks_core::server::validate_selection(server).is_ok()
                    })
                    .map_or(0, |server| server.attachment_size_limit()),
            };
            let can_mention_everyone = allows(1 << 37);
            let can_mention_roles = allows(1 << 38);
            self.composer.update(cx, |composer, cx| {
                composer.set_channel(
                    channel.id.clone(),
                    &channel_label(channel, Some(snapshot)),
                    window,
                    cx,
                );
                composer.set_attachment_access(access, cx);
                composer.sync_mentions(
                    channel,
                    snapshot,
                    deployment.and_then(|server| server.endpoints.autumn.as_deref()),
                    can_mention_everyone, // MentionEveryone also gates @online.
                    can_mention_roles,    // MentionRoles.
                    cx,
                );
            });
        } else {
            self.composer
                .update(cx, |composer, cx| composer.clear(window, cx));
        }
    }
}

// Core authorizes server channels from the live gateway state. Private groups
// additionally carry an explicit permission mask (absent means DM defaults).
fn group_allows(
    channel: &ClientChannel,
    user: &stonks_core::connection::ClientUser,
    mask: u64,
) -> bool {
    channel.server.is_some()
        || channel.channel_type != "Group"
        || user.privileged
        || channel.owner.as_deref() == Some(&user.id)
        || channel
            .permissions
            .is_none_or(|permissions| permissions & mask == mask)
}
