//! Connect live session messages and desktop activations to authenticated navigation.
use super::desktop_notifications::{self, Delivery, Route, Target};
use super::{navigation, *};
use futures_channel::{
    mpsc::{self, Sender},
    oneshot,
};
use futures_util::StreamExt;
use stonks_core::{
    connection::GatewayEvent,
    messages::{normalize::valid_id, read},
};

pub(super) struct DesktopDelivery {
    requests: Sender<Delivery>,
    _clicks: Task<()>,
    _stop: oneshot::Sender<()>,
}

impl ShellView {
    pub(super) fn notify_live_message(
        &mut self,
        event: &GatewayEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.is_window_active() || event.data["system"]["type"] == "call_started" {
            return;
        }
        let data = &event.data;
        let (Some(channel_id), Some(message), Some(author)) = (
            data["channel"].as_str(),
            data["_id"].as_str(),
            data["author"].as_str(),
        ) else {
            return;
        };
        if !valid_id(message) {
            return;
        }
        let session = self.session.read(cx);
        let Some(snapshot) = session.snapshot.as_ref() else {
            return;
        };
        let me = &snapshot.current_user.id;
        if author == me
            || !session.can_view_text_channel(channel_id)
            || read::cursor(snapshot, channel_id).is_some_and(|cursor| cursor >= message)
        {
            return;
        }
        let Some((policy, account)) = session.account_context() else {
            return;
        };
        let Some(channel) = snapshot.channels.iter().find(|ch| ch.id == channel_id) else {
            return;
        };
        // Read projection uses the same direct, role and mass mention rules as Tauri.
        let mentioned = snapshot.unreads.iter().any(|entry| {
            entry.id.user == *me
                && entry.id.channel == channel_id
                && entry.mentions.iter().any(|id| id == message)
        });
        let modes = self.notifications.read(cx);
        let prefs = cx.global::<crate::preferences::NativePreferences>();
        if !modes.ready || prefs.load_error.is_some() {
            return;
        }
        let settings = desktop_notifications::Policy::from(&prefs.saved);
        if !settings.allows(
            false,
            &modes.saved,
            channel_id,
            mentioned,
            navigation::notifications::now(),
        ) {
            return;
        }
        let server = channel
            .server
            .as_ref()
            .and_then(|id| snapshot.servers.iter().find(|server| &server.id == id));
        let user = snapshot.users.iter().find(|user| user.id == author);
        let sender = data["masquerade"]["name"]
            .as_str()
            .or_else(|| data["webhook"]["name"].as_str())
            .or_else(|| data["user"]["display_name"].as_str())
            .or_else(|| data["user"]["username"].as_str())
            .or_else(|| {
                user.and_then(|user| {
                    user.display_name
                        .as_deref()
                        .or(Some(user.username.as_str()))
                })
            })
            .unwrap_or("Someone");
        let title = server.map_or_else(
            || sender.to_owned(),
            |server| {
                format!(
                    "{} · #{}",
                    server.name,
                    navigation::view::channel_label(channel, Some(snapshot))
                )
            },
        );
        let body = desktop_notifications::preview_body(
            prefs.saved.show_notification_content,
            server.map(|_| sender),
            || {
                let content = data["content"]
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty());
                content
                    .unwrap_or_else(|| {
                        if data["attachments"]
                            .as_array()
                            .is_some_and(|files| !files.is_empty())
                        {
                            "Sent an attachment"
                        } else {
                            "New activity"
                        }
                    })
                    .to_owned()
            },
        );
        let request = Delivery {
            title,
            body,
            sound: prefs.saved.notification_sounds
                && settings.sound(channel.server.is_none(), mentioned),
            target: Target {
                user: me.clone(),
                account: Arc::new(account),
                route: Route::Channel {
                    server: channel.server.clone(),
                    channel: channel_id.into(),
                    message: Some(message.into()),
                },
            },
        };
        if self
            .desktop_delivery
            .as_ref()
            .is_none_or(|worker| worker.requests.is_closed())
        {
            let (requests, incoming) = mpsc::channel(32);
            let (clicks, mut activations) = mpsc::channel(16);
            // Dropping the worker stops delivery without draining queued requests.
            let (stop, shutdown) = oneshot::channel();
            let _worker = crate::runtime::spawn_core(async move {
                let _ = desktop_notifications::deliver(policy, incoming, clicks, shutdown).await;
            });
            let clicks = cx.spawn_in(window, async move |this, cx| {
                while let Some(target) = activations.next().await {
                    if cx
                        .update(|window, cx| {
                            let _ = this.update(cx, |this, cx| {
                                let session = this.session.read(cx);
                                if !this
                                    .policy
                                    .is_authenticated_context_current(&target.account)
                                    || session.account_context().is_none()
                                    || session.snapshot.as_ref().is_none_or(|snapshot| {
                                        snapshot.current_user.id != target.user
                                    })
                                {
                                    return;
                                }
                                if this.navigate_route(target.route, cx).is_ok() {
                                    window.activate_window();
                                }
                            });
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            self.desktop_delivery = Some(DesktopDelivery {
                requests,
                _clicks: clicks,
                _stop: stop,
            });
        }
        if let Some(worker) = self.desktop_delivery.as_mut() {
            let _ = worker.requests.try_send(request);
        }
    }

    /// Host entry point for inbound links after the active session is ready.
    /// `Ok(false)` leaves unsupported links (e.g. invites) to their owning flow.
    pub(crate) fn navigate_external_route(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, &'static str> {
        let session = self.session.read(cx);
        let Some((_, context)) = session.account_context() else {
            return Err("The session is not ready.");
        };
        if !session.can_route_external() {
            return Err("The session is not ready.");
        }
        let Some(route) = desktop_notifications::parse_route(value, &context.server.deployment_url)
        else {
            return Ok(false);
        };
        self.navigate_route(route, cx)?;
        window.activate_window();
        Ok(true)
    }

    fn navigate_route(&mut self, route: Route, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let session = self.session.read(cx);
        match route {
            Route::Server(id) => {
                if !session
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.servers.iter().any(|server| server.id == id))
                {
                    return Err("That server is not available in this account.");
                }
                if self.focused {
                    self.compact_nav.open_server(id.clone());
                }
                self.select_server(id, cx);
            }
            Route::Channel {
                server,
                channel,
                message,
            } => {
                const UNAVAILABLE: &str = "That channel is not available in this account.";
                let entry = session
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.channels.iter().find(|entry| entry.id == channel))
                    .filter(|entry| {
                        server
                            .as_ref()
                            .is_none_or(|server| entry.server.as_ref() == Some(server))
                    })
                    .filter(|_| session.can_view_text_channel(&channel))
                    .ok_or(UNAVAILABLE)?;
                self.home = entry.server.is_none();
                self.selected_server = entry.server.clone();
                self.select_text_view(channel.clone(), cx);
                if self.selected_channel.as_ref() != Some(&channel) {
                    return Err(UNAVAILABLE);
                }
                if let Some(message) = message {
                    self.timeline
                        .update(cx, |timeline, cx| timeline.jump_to(message, cx));
                }
            }
        }
        Ok(())
    }
}
