//! Shared context-menu dispatch and live settings synchronization.
use super::super::channel_settings::ChannelDanger;
use super::navigation::notifications;
use super::*;
use std::collections::HashMap;
use stonks_core::{
    channel_actions::ChannelAccess,
    channel_preferences::{Change, NotificationMode},
    member_actions::Action,
};
use stonks_ui::components::MenuEntry;
#[derive(Clone)]
pub(super) enum MenuAction {
    ChannelSettings(String),
    ChannelDanger(String, ChannelDanger),
    Invite(String),
    Mute(String, Option<Option<u32>>),
    Notifications(String, Option<NotificationMode>),
    Member {
        server: String,
        user: String,
        action: Action,
    },
}
impl ShellView {
    pub(super) fn item(&mut self, action: MenuAction, label: impl Into<String>) -> MenuEntry {
        let id = self.menu_actions.len().to_string();
        self.menu_actions.insert(id.clone(), action);
        MenuEntry::action(id, label)
    }
    pub(super) fn run_menu_action(
        &mut self,
        action: MenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            MenuAction::ChannelSettings(channel) => {
                self.open_channel_settings(channel, None, window, cx)
            }
            MenuAction::ChannelDanger(channel, danger) => {
                self.open_channel_settings(channel, Some(danger), window, cx)
            }
            MenuAction::Invite(channel) => self.open_channel_invite(channel, window, cx),
            MenuAction::Mute(channel, minutes) => {
                let until = minutes.map(|value| {
                    value.map(|minutes| {
                        notifications::now().saturating_add(i64::from(minutes) * 60_000)
                    })
                });
                self.notifications.update(cx, |prefs, cx| {
                    prefs.change(channel, Change::Mute(until), cx)
                });
            }
            MenuAction::Notifications(channel, mode) => {
                self.notifications.update(cx, |prefs, cx| {
                    prefs.change(channel, Change::Notifications(mode), cx)
                })
            }
            MenuAction::Member {
                server,
                user,
                action,
            } => {
                if self.overlay_focus.is_none() {
                    self.overlay_focus = window.focused(cx);
                }
                self.member_actions.update(cx, |actions, cx| {
                    actions.perform(server, user, action, window, cx)
                });
            }
        }
        cx.notify();
    }
    pub(super) fn settings_user_names(&self, cx: &App) -> HashMap<String, String> {
        self.session
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .users
                    .iter()
                    .chain(std::iter::once(&snapshot.current_user))
                    .map(|user| {
                        (
                            user.id.clone(),
                            user.display_name
                                .clone()
                                .unwrap_or_else(|| user.username.clone()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn refresh_interactions(&mut self, cx: &mut Context<Self>) {
        if let Some(settings) = self.settings.clone() {
            let names = self.settings_user_names(cx);
            let id = settings.read(cx).server_id().to_owned();
            let data = self
                .session
                .read(cx)
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .servers
                        .iter()
                        .find(|server| server.id == id)
                        .map(|server| {
                            (
                                server.clone(),
                                snapshot
                                    .channels
                                    .iter()
                                    .filter(|channel| channel.server.as_deref() == Some(&id))
                                    .cloned()
                                    .collect(),
                            )
                        })
                });
            if let Some((server, channels)) = data {
                settings.update(cx, |settings, cx| {
                    settings.set_user_names(names, cx);
                    settings.refresh(server, channels, cx);
                });
            } else {
                self.settings = None;
                self.settings_sub = None;
            }
        }
        if let Some(settings) = self.channel_settings.as_ref() {
            let id = settings.read(cx).channel_id().to_owned();
            let session = self.session.read(cx);
            let access = ChannelAccess::from_manager(&session.authorization(), &id);
            let data = session.snapshot.as_ref().and_then(|snapshot| {
                let channel = snapshot.channels.iter().find(|channel| channel.id == id)?;
                let server = snapshot
                    .servers
                    .iter()
                    .find(|server| Some(&server.id) == channel.server.as_ref())?;
                Some((channel.clone(), server.clone()))
            });
            if let Some((channel, server)) = data {
                settings.update(cx, |settings, cx| {
                    settings.set_access(access, cx);
                    settings.refresh(channel, server, cx);
                });
            } else {
                settings.update(cx, |settings, cx| settings.cancel(cx));
                self.channel_settings = None;
                self.channel_sub = None;
            }
        }
    }
}
