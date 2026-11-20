//! Channel context menu and its settings and invite entry points.

use super::super::{interactions::MenuAction, *};
use super::notifications;
use crate::features::channel_settings::{ChannelDanger, ChannelSettings, ChannelSettingsEvent};
use stonks_core::{channel_actions::ChannelAccess, channel_preferences::NotificationMode};
use stonks_ui::components::MenuEntry;

impl ShellView {
    pub(in crate::features) fn open_channel_menu(
        &mut self,
        channel: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session.read(cx).can_view_channel(&channel) {
            return;
        }
        let access = ChannelAccess::from_manager(&self.session.read(cx).authorization(), &channel);
        if !access.settings() && !access.manage_messages {
            return;
        }
        self.menu_actions.clear();
        let prefs = self.notifications.read(cx);
        let ready = prefs.ready;
        let muted = prefs.saved.is_muted(&channel, notifications::now());
        let mode = prefs
            .saved
            .channel_notification_modes
            .get(&channel)
            .copied();
        let mut entries = vec![];
        if muted {
            entries.push(self.item(MenuAction::Mute(channel.clone(), None), "Unmute channel"));
        } else {
            let mut items = [
                (15, "For 15 minutes"),
                (60, "For 1 hour"),
                (180, "For 3 hours"),
                (480, "For 8 hours"),
                (1440, "For 24 hours"),
            ]
            .into_iter()
            .map(|(minutes, label)| {
                self.item(
                    MenuAction::Mute(channel.clone(), Some(Some(minutes))),
                    label,
                )
            })
            .collect::<Vec<_>>();
            items.push(self.item(
                MenuAction::Mute(channel.clone(), Some(None)),
                "Until I turn it back on",
            ));
            entries.push(MenuEntry::Submenu {
                label: "Mute channel".into(),
                disabled: !ready,
                items,
            });
        }
        let items = [
            (None, "Default"),
            (Some(NotificationMode::All), "All messages"),
            (Some(NotificationMode::Mention), "Mentions only"),
            (Some(NotificationMode::None), "None"),
        ]
        .into_iter()
        .map(|(value, label)| {
            self.item(MenuAction::Notifications(channel.clone(), value), label)
                .checked(mode == value)
        })
        .collect();
        entries.push(MenuEntry::Submenu {
            label: "Notifications".into(),
            disabled: !ready,
            items,
        });
        entries.push(MenuEntry::Separator);
        let access = ChannelAccess::from_manager(&self.session.read(cx).authorization(), &channel);
        if access.invite {
            entries.push(self.item(MenuAction::Invite(channel.clone()), "Create invite"));
        }
        if access.settings() {
            entries.push(self.item(
                MenuAction::ChannelSettings(channel.clone()),
                "Open channel settings",
            ));
        }
        if access.manage_messages {
            entries.push(
                self.item(
                    MenuAction::ChannelDanger(channel.clone(), ChannelDanger::Reset),
                    "Reset channel",
                )
                .danger(),
            );
        }
        if access.manage_channel {
            entries.push(
                self.item(
                    MenuAction::ChannelDanger(channel.clone(), ChannelDanger::Delete),
                    "Delete channel",
                )
                .danger(),
            );
        }
        self.context_menu
            .update(cx, |menu, cx| menu.open(position, entries, window, cx));
        cx.notify();
    }
    pub(in crate::features::shell) fn open_channel_settings(
        &mut self,
        id: String,
        danger: Option<ChannelDanger>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        let Some((channel, server)) = session.snapshot.as_ref().and_then(|snapshot| {
            let channel = snapshot.channels.iter().find(|channel| channel.id == id)?;
            let server = snapshot
                .servers
                .iter()
                .find(|server| Some(&server.id) == channel.server.as_ref())?;
            Some((channel.clone(), server.clone()))
        }) else {
            return;
        };
        let authorization = session.authorization();
        let access = ChannelAccess::from_manager(&authorization, &id);
        if danger.is_none() && !access.settings() {
            return;
        }
        if self.overlay_focus.is_none() {
            self.overlay_focus = window.focused(cx);
        }
        self.message_panel
            .update(cx, |panel, cx| panel.reset(window, cx));
        self.drawer = None;
        let settings = cx.new(|cx| {
            ChannelSettings::new(
                channel,
                server,
                self.policy.clone(),
                authorization,
                access,
                window,
                cx,
            )
        });
        self.channel_sub = Some(cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &ChannelSettingsEvent, window, cx| {
                match event {
                    ChannelSettingsEvent::Updated(channel) => {
                        this.session.update(cx, |session, cx| {
                            session.upsert_channel(channel.clone(), cx)
                        })
                    }
                    ChannelSettingsEvent::Deleted(id) => {
                        this.timeline
                            .update(cx, |timeline, cx| timeline.forget_channel(id, cx));
                        this.session
                            .update(cx, |session, cx| session.remove_channel(id, cx));
                        this.channel_settings = None;
                        this.channel_sub = None;
                        this.restore_overlay_focus(window, cx);
                    }
                    ChannelSettingsEvent::Reset {
                        channel_id,
                        deleted: _deleted,
                    } => {
                        this.timeline
                            .update(cx, |timeline, cx| timeline.reload_channel(channel_id, cx));
                        this.session
                            .update(cx, |session, cx| session.reset_channel(channel_id, cx));
                    }
                    ChannelSettingsEvent::Error(error) => this.shell_error = Some(error.clone()),
                    ChannelSettingsEvent::Close => {
                        this.channel_settings = None;
                        this.channel_sub = None;
                        this.restore_overlay_focus(window, cx);
                    }
                }
                cx.notify();
            },
        ));
        if let Some(danger) = danger {
            settings.update(cx, |settings, cx| {
                settings.set_frame_visible(false, cx);
                settings.open_confirmation(danger, cx);
                settings.focus(window, cx);
            });
        }
        self.channel_settings = Some(settings);
        cx.notify();
    }
    pub(in crate::features::shell) fn open_channel_invite(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        if !session.can_channel_permission(&id, 1 << 25) {
            return;
        }
        let Some((server, channels)) = session.snapshot.as_ref().and_then(|snapshot| {
            let channel = snapshot.channels.iter().find(|channel| channel.id == id)?;
            let server = snapshot
                .servers
                .iter()
                .find(|server| Some(&server.id) == channel.server.as_ref())?;
            Some((
                server.clone(),
                snapshot
                    .channels
                    .iter()
                    .filter(|channel| channel.server.as_deref() == Some(&server.id))
                    .cloned()
                    .collect::<Vec<_>>(),
            ))
        }) else {
            return;
        };
        let authorization = session.authorization();
        let can_manage = session.can_manage_server(&server.id);
        if self.overlay_focus.is_none() {
            self.overlay_focus = window.focused(cx);
        }
        let settings = cx.new(|cx| {
            ServerSettings::new(
                server,
                channels,
                can_manage,
                self.policy.clone(),
                self.images.clone(),
                window,
                cx,
            )
        });
        settings.update(cx, |settings, cx| {
            settings.set_authorization(authorization, cx);
            settings.set_frame_visible(false, cx);
            settings.open_invite(id, window, cx);
        });
        self.settings_sub = Some(cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &ServerSettingsEvent, window, cx| {
                match event {
                    ServerSettingsEvent::Close => {
                        this.settings = None;
                        this.settings_sub = None;
                        this.restore_overlay_focus(window, cx);
                    }
                    ServerSettingsEvent::EmojisChanged => this.refresh_emoji_consumers(cx),
                    ServerSettingsEvent::Updated(server) => this
                        .session
                        .update(cx, |session, cx| session.update_server(server.clone(), cx)),
                }
                cx.notify();
            },
        ));
        let names = self.settings_user_names(cx);
        settings.update(cx, |settings, cx| settings.set_user_names(names, cx));
        self.settings = Some(settings);
        cx.notify();
    }
}
