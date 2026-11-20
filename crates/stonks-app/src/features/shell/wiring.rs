//! Shell construction and entity subscriptions.

use super::{navigation::notifications, people::member_actions, *};
use crate::features::conversation::composer;
use crate::features::conversation::media::image_viewer::ImageViewerEvent;
use crate::features::conversation::message_panel::PanelEvent;
use crate::features::conversation::timeline::TimelineEvent;
use crate::{session, shared::file_picker};

impl ShellView {
    pub fn new(
        session: Entity<Session>,
        timeline: Entity<Timeline>,
        images: Entity<Images>,
        composer: Entity<composer::Composer>,
        policy: Arc<HttpPolicy>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        composer.update(cx, |composer, cx| {
            composer.set_emoji_context(session.clone(), images.clone(), cx)
        });
        let image_viewer = cx.new(|cx| ImageViewer::new(images.clone(), cx));
        let image_viewer_observe = cx.observe(&image_viewer, |_, _, cx| cx.notify());
        let image_viewer_sub = cx.subscribe_in(
            &image_viewer,
            window,
            |_, _, event: &ImageViewerEvent, window, cx| {
                if let ImageViewerEvent::OpenExternal(value) = event {
                    stonks_ui::external_links::request(value, window, cx);
                }
                cx.notify();
            },
        );
        let timeline_media = cx.subscribe_in(
            &timeline,
            window,
            |this, _, event: &TimelineEvent, window, cx| {
                match event {
                    TimelineEvent::OpenImage(image) => {
                        this.context_menu
                            .update(cx, |menu, cx| menu.close(window, cx));
                        this.image_viewer
                            .update(cx, |viewer, cx| viewer.open(image.clone(), window, cx));
                    }
                    TimelineEvent::NavigateChannel(channel) => {
                        this.navigate_mentioned_channel(channel.clone(), cx);
                    }
                    TimelineEvent::Reply(message) => {
                        this.composer.update(cx, |composer, cx| {
                            composer.set_reply(message, window, cx);
                        });
                    }
                    TimelineEvent::ActionUiChanged => {}
                    TimelineEvent::ResetMedia => {
                        let timeline = this.timeline.clone();
                        timeline
                            .read(cx)
                            .action_menu()
                            .update(cx, |menu, cx| menu.close(window, cx));
                        timeline.update(cx, |timeline, cx| timeline.clear_action_ui(window, cx));
                        this.image_viewer
                            .update(cx, |viewer, cx| viewer.reset(window, cx));
                    }
                }
                cx.notify();
            },
        );
        let context_menu = cx.new(stonks_ui::components::UiContextMenu::new);
        let context_sub = cx.subscribe_in(
            &context_menu,
            window,
            |this, _, event: &stonks_ui::components::ContextMenuEvent, window, cx| {
                if let stonks_ui::components::ContextMenuEvent::Select(id) = event {
                    if let Some(action) = this.menu_actions.get(id).cloned() {
                        this.run_menu_action(action, window, cx);
                    }
                }
                cx.notify();
            },
        );
        let notifications_path = cx
            .global::<crate::preferences::NativePreferences>()
            .path
            .clone();
        let notifications = cx.new(|cx| notifications::Notifications::new(notifications_path, cx));
        let live_notifications = cx.subscribe_in(
            &session,
            window,
            |this, _, event: &session::LiveMessage, window, cx| {
                this.notify_live_message(&event.0, window, cx);
            },
        );
        let notifications_observe = cx.observe(&notifications, |_, _, cx| cx.notify());
        let member_actions = cx.new(|cx| {
            member_actions::MemberActions::new(
                policy.clone(),
                session.clone(),
                images.clone(),
                window,
                cx,
            )
        });
        let member_sub = cx.subscribe_in(
            &member_actions,
            window,
            |this, _, event: &member_actions::MemberActionsEvent, window, cx| {
                use member_actions::MemberActionsEvent as Event;
                match event {
                    Event::Navigate(channel) => {
                        let id = channel.id.clone();
                        this.session.update(cx, |session, cx| {
                            session.upsert_channel(channel.clone(), cx)
                        });
                        this.open_home_conversation(id, cx);
                    }
                    Event::UpdatedUser(user) => this
                        .session
                        .update(cx, |session, cx| session.update_user(user.clone(), cx)),
                    Event::UpdatedMember {
                        server,
                        user,
                        patch,
                    } => this.session.update(cx, |session, cx| {
                        session.update_member(server, user, patch.clone(), cx)
                    }),
                    Event::RemovedMember { server, user } => this
                        .session
                        .update(cx, |session, cx| session.remove_member(server, user, cx)),
                    Event::UpdatedServer(server) => this
                        .session
                        .update(cx, |session, cx| session.update_server(server.clone(), cx)),
                    Event::Close => this.restore_overlay_focus(window, cx),
                }
                cx.notify();
            },
        );
        let message_panel =
            cx.new(|cx| MessagePanel::new(policy.clone(), images.clone(), window, cx));
        let panel_sub = cx.subscribe_in(
            &message_panel,
            window,
            |this, _, event: &PanelEvent, window, cx| match event {
                PanelEvent::Jump(id) => {
                    this.timeline
                        .update(cx, |timeline, cx| timeline.jump_to(id.clone(), cx));
                    this.close_message_panel(window, cx);
                }
                PanelEvent::Close => this.close_message_panel(window, cx),
            },
        );
        let panel_observe = cx.observe(&message_panel, |_, _, cx| cx.notify());
        let images_observe = cx.observe(&images, |_, _, cx| cx.notify());
        let timeline_sent = timeline.clone();
        let composer_sub = cx.subscribe(&composer, move |_, _, event: &composer::Sent, cx| {
            timeline_sent.update(cx, |timeline, cx| {
                timeline.receive_sent(event.0.clone(), cx)
            })
        });
        let typing_session = session.clone();
        let typing_sub = cx.subscribe(&composer, move |_, _, event: &composer::Typing, cx| {
            let _ = typing_session
                .read(cx)
                .set_typing(event.channel_id.clone(), event.typing);
        });
        let typing_events = cx.subscribe(
            &session,
            |this, session, event: &session::SessionEvent, cx| {
                let session::SessionEvent::Gateway(event) = event;
                this.message_panel
                    .update(cx, |panel, cx| panel.apply_gateway_event(event, cx));
                let (channel, user, typing) = match event.event_type.as_str() {
                    "ChannelStartTyping" => (
                        event.data.get("id").and_then(serde_json::Value::as_str),
                        event.data.get("user").and_then(serde_json::Value::as_str),
                        true,
                    ),
                    "ChannelStopTyping" => (
                        event.data.get("id").and_then(serde_json::Value::as_str),
                        event.data.get("user").and_then(serde_json::Value::as_str),
                        false,
                    ),
                    "Message" => (
                        event
                            .data
                            .get("channel")
                            .and_then(serde_json::Value::as_str),
                        event.data.get("author").and_then(serde_json::Value::as_str),
                        false,
                    ),
                    _ => return,
                };
                let (Some(channel), Some(user)) = (channel, user) else {
                    return;
                };
                let label = session.read(cx).snapshot.as_ref().and_then(|snapshot| {
                    if snapshot.current_user.id == user {
                        return None;
                    }
                    snapshot
                        .users
                        .iter()
                        .find(|candidate| candidate.id == user)
                        .map(|user| {
                            user.display_name
                                .clone()
                                .unwrap_or_else(|| user.username.clone())
                        })
                });
                if let Some(label) = label {
                    this.composer.update(cx, |composer, cx| {
                        composer.apply_remote_typing(channel, user, &label, typing, cx)
                    });
                }
            },
        );
        let voice = cx.new(|cx| crate::features::voice::Voice::new(cx));
        composer.update(cx, |composer, _| {
            let voice = voice.downgrade();
            composer.music_route = Some(std::rc::Rc::new(move |url, cx| {
                let (call, result) = voice
                    .update(cx, |voice, cx| voice.request_music(url, cx))
                    .map_err(|_| stonks_ui::i18n::t("messages.musicQueueUnavailable"))??;
                let voice = voice.clone();
                Ok(composer::MusicSubmission {
                    result,
                    is_current: Box::new(move |cx| {
                        voice
                            .upgrade()
                            .is_some_and(|voice| voice.read(cx).music_call_is_current(call))
                    }),
                })
            }));
        });
        let voice_observe = cx.observe(&voice, |_, _, cx| cx.notify());
        let preferences_observe =
            cx.observe_global::<crate::preferences::NativePreferences>(|this, cx| {
                // Withdraw old previews when the privacy settings change.
                this.desktop_delivery = None;
                this.sync_composer_preferences(cx);
                this.timeline.update(cx, |_, cx| cx.notify());
                cx.notify();
            });
        let observe = cx.observe_in(&session, window, |this, session, window, cx| {
            let identity = session
                .read(cx)
                .snapshot
                .as_ref()
                .zip(session.read(cx).server.as_ref())
                .map(|(snapshot, server)| {
                    (
                        server.deployment_url.clone(),
                        snapshot.current_user.id.clone(),
                    )
                });
            if identity != this.identity {
                this.desktop_delivery = None;
                this.composer
                    .update(cx, |composer, cx| composer.reset(window, cx));
                this.timeline.update(cx, |timeline, cx| timeline.clear(cx));
                this.message_panel
                    .update(cx, |panel, cx| panel.reset(window, cx));
                this.context_menu
                    .update(cx, |menu, cx| menu.close(window, cx));
                this.member_actions
                    .update(cx, |actions, cx| actions.reset(window, cx));
                this.image_viewer
                    .update(cx, |viewer, cx| viewer.reset(window, cx));
                if let Some(settings) = this.channel_settings.take() {
                    settings.update(cx, |settings, cx| settings.cancel(cx));
                }
                this.channel_sub = None;
                let crop = cx.global::<file_picker::CropService>().0.clone();
                crop.update(cx, |crop, cx| crop.reset(window, cx));
                this.clear_account_settings(cx);
                this.settings = None;
                this.settings_sub = None;
                this.drawer = None;
                this.overlay_focus = None;
                this.compact_nav = Default::default();
                this.wide_selection = None;
                this.selected_channel = None;
                this.selected_server = None;
                this.voice.update(cx, |voice, cx| voice.reset(cx));
                this.identity = identity;
                this.home = true;
                this.wide_friends = false;
                this.home_friends = None;
            }
            if session.read(cx).account_context().is_none() && this.account_settings.is_some() {
                this.clear_account_settings(cx);
                this.restore_overlay_focus(window, cx);
            }
            this.sync_home_messages(cx);
            this.sync_friends(window, cx);
            this.sync_composer_preferences(cx);
            this.ensure_selection(cx);
            this.refresh_interactions(cx);
        });
        let explorer_layout = crate::desktop::explorer_layout::preferences(cx);
        let mut shell = Self {
            image_viewer,
            _image_viewer_sub: image_viewer_sub,
            _image_viewer_observe: image_viewer_observe,
            _timeline_media: timeline_media,
            context_menu,
            menu_actions: Default::default(),
            _context_sub: context_sub,
            channel_settings: None,
            channel_sub: None,
            member_actions,
            _member_sub: member_sub,
            notifications,
            _notifications_observe: notifications_observe,
            _live_notifications: live_notifications,
            desktop_delivery: None,
            member_bounds: Default::default(),
            shell_error: None,
            voice,
            _voice_observe: voice_observe,
            policy,
            message_panel,
            _panel_sub: panel_sub,
            _panel_observe: panel_observe,
            settings: None,
            settings_sub: None,
            account_settings: None,
            account_settings_sub: None,
            account_settings_observe: None,
            overlay_focus: None,
            overlay_container: cx.focus_handle(),
            drawer: None,
            focused: false,
            compact_nav: Default::default(),
            wide_selection: None,
            channel_width: explorer_layout.channel_width(f32::from(window.viewport_size().width)),
            member_width: MEMBERS_WIDTH,
            drag: None,
            identity: session
                .read(cx)
                .snapshot
                .as_ref()
                .zip(session.read(cx).server.as_ref())
                .map(|(snapshot, server)| {
                    (
                        server.deployment_url.clone(),
                        snapshot.current_user.id.clone(),
                    )
                }),
            session,
            timeline,
            images,
            _images_observe: images_observe,
            selected_server: None,
            selected_channel: None,
            _observe: observe,
            _preferences_observe: preferences_observe,
            panel_entrance: 0,
            collapsed_groups: Default::default(),
            collapsed_channels: Default::default(),
            composer,
            _composer_sub: composer_sub,
            _typing_sub: typing_sub,
            _typing_events: typing_events,
            show_members: true,
            server_rail: layout::ServerRail::new(explorer_layout.rail_hidden),
            show_channels: !explorer_layout.channel_collapsed,
            home: true,
            home_messages: Default::default(),
            home_friends: None,
            wide_friends: false,
        };
        shell.sync_home_messages(cx);
        shell.sync_friends(window, cx);
        shell.sync_composer_preferences(cx);
        shell.ensure_selection(cx);
        shell
    }
}
