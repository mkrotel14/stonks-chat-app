//! Shell routing for the existing Tauri header controls and overlay surfaces.
use super::*;
use gpui_kit::base::FocusTrapElement;
impl ShellView {
    pub(super) fn restore_overlay_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.overlay_focus.take() {
            focus.focus(window, cx);
        }
    }
    pub(in crate::features) fn clear_account_settings(&mut self, cx: &mut Context<Self>) {
        self.account_settings = None;
        self.account_settings_sub = None;
        self.account_settings_observe = None;
        cx.notify();
    }

    pub(super) fn sync_composer_preferences(&self, cx: &mut Context<Self>) {
        let prefs = &cx.global::<crate::preferences::NativePreferences>().saved;
        let typing = self.identity.is_some() && prefs.send_typing_indicators;
        let spellcheck = self.identity.is_some() && prefs.spellcheck;
        self.composer.update(cx, |composer, cx| {
            composer.set_typing_enabled(typing, cx);
            composer.set_spellcheck_enabled(spellcheck, cx);
        });
    }

    pub(super) fn open_account_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::features::settings::{Settings, SettingsEvent};
        if self.account_settings.is_some() || self.session.read(cx).account_context().is_none() {
            return;
        }
        self.overlay_focus = self.overlay_focus.take().or_else(|| window.focused(cx));
        self.drawer = None;
        self.message_panel
            .update(cx, |panel, cx| panel.reset(window, cx));
        self.voice
            .update(cx, |voice, cx| voice.dismiss_soundboard(window, cx));
        let settings = cx.new(|cx| {
            Settings::new(
                self.session.clone(),
                self.images.clone(),
                self.voice.clone(),
                window,
                cx,
            )
        });
        self.account_settings_sub = Some(cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsEvent, window, cx| {
                this.clear_account_settings(cx);
                if this.focused && matches!(event, SettingsEvent::Close) {
                    this.compact_back(window, cx);
                }
                this.restore_overlay_focus(window, cx);
                if matches!(event, SettingsEvent::Logout) {
                    cx.emit(SignOut);
                }
                cx.notify();
            },
        ));
        self.account_settings_observe = Some(cx.observe(&settings, |_, _, cx| cx.notify()));
        self.account_settings = Some(settings);
        cx.notify();
    }
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server) = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .servers
                    .iter()
                    .find(|server| Some(&server.id) == self.selected_server.as_ref())
            })
            .cloned()
        else {
            return;
        };
        let can_manage = self.session.read(cx).can_manage_server(&server.id);
        if !can_manage
            && !self
                .session
                .read(cx)
                .can_server_permission(&server.id, 1 << 4)
        {
            return;
        }
        let channels = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .channels
                    .iter()
                    .filter(|channel| channel.server.as_deref() == Some(&server.id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if self.overlay_focus.is_none() {
            self.overlay_focus = window.focused(cx);
        }
        self.message_panel
            .update(cx, |panel, cx| panel.reset(window, cx));
        self.drawer = None;
        let authorization = self.session.read(cx).authorization();
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
        let names = self.settings_user_names(cx);
        settings.update(cx, |settings, cx| {
            settings.set_authorization(authorization, cx);
            settings.set_user_names(names, cx);
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
        settings.update(cx, |settings, cx| settings.focus(window, cx));
        self.settings = Some(settings);
        cx.notify();
    }
    pub(super) fn toggle_explorer(
        &mut self,
        explorer: Explorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focused {
            if self.drawer == Some(explorer) {
                self.drawer = None;
                self.restore_overlay_focus(window, cx);
            } else {
                if self.overlay_focus.is_none() {
                    self.overlay_focus = window.focused(cx);
                }
                self.drawer = Some(explorer);
                self.panel_entrance = self.panel_entrance.wrapping_add(1);
                self.overlay_container.focus(window, cx);
                self.message_panel
                    .update(cx, |panel, cx| panel.reset(window, cx));
            }
        } else {
            match explorer {
                Explorer::Channels => {
                    let visible = self
                        .fitted_explorer_widths(f32::from(window.viewport_size().width))
                        .0
                        > 0.;
                    self.set_channels_open(!visible, cx);
                }
                Explorer::Members => self.show_members = !self.show_members,
            }
        }
        cx.notify();
    }
    pub(super) fn has_overlay(&self, cx: &App) -> bool {
        self.message_panel.read(cx).mode().is_some()
            || self.has_overlay_other_than_message_panel(cx)
    }
    pub(super) fn has_overlay_other_than_message_panel(&self, cx: &App) -> bool {
        self.update_details_open(cx)
            || self.account_settings.is_some()
            || self.friends_dialog_open(cx)
            || self.image_viewer.read(cx).is_open()
            || self.context_menu.read(cx).is_open()
            || self.timeline.read(cx).action_menu().read(cx).is_open()
            || self.timeline.read(cx).has_action_ui(cx)
            || self.member_actions.read(cx).is_open()
            || self
                .channel_settings
                .as_ref()
                .is_some_and(|settings| settings.read(cx).visible())
            || cx
                .global::<crate::shared::file_picker::CropService>()
                .0
                .read(cx)
                .is_open()
            || self
                .settings
                .as_ref()
                .is_some_and(|settings| settings.read(cx).visible())
            || self.drawer.is_some()
            || self.timeline.read(cx).has_mentioned_user()
    }
    pub(super) fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.friends_dialog_open(cx) {
            self.dismiss_friends_dialog(window, cx);
            return;
        }
        let crop = cx
            .global::<crate::shared::file_picker::CropService>()
            .0
            .clone();
        if crop.read(cx).is_open() {
            if !crop.read(cx).busy() {
                crop.update(cx, |crop, cx| crop.cancel(window, cx));
            }
            return;
        }
        if self.image_viewer.read(cx).is_open() {
            self.image_viewer
                .update(cx, |viewer, cx| viewer.close(window, cx));
            return;
        }
        let message_menu = self.timeline.read(cx).action_menu();
        if message_menu.read(cx).is_open() {
            message_menu.update(cx, |menu, cx| menu.close(window, cx));
            return;
        }
        if self.context_menu.read(cx).is_open() {
            self.context_menu
                .update(cx, |menu, cx| menu.close(window, cx));
            return;
        }
        if self.member_actions.read(cx).is_open() {
            self.member_actions
                .update(cx, |actions, cx| actions.close(window, cx));
            return;
        }
        if let Some(settings) = self.channel_settings.as_ref() {
            if settings.read(cx).visible() {
                if !settings.read(cx).busy() {
                    settings.update(cx, |settings, cx| settings.close(cx));
                }
                return;
            }
        }
        if let Some(settings) = &self.account_settings {
            settings.update(cx, |settings, cx| settings.escape(window, cx));
            return;
        }
        if let Some(settings) = self.settings.as_ref() {
            settings.update(cx, |settings, cx| settings.close(cx));
            return;
        } else if self.message_panel.read(cx).mode().is_some() {
            self.message_panel
                .update(cx, |panel, cx| panel.reset(window, cx));
        } else if self.drawer.is_some() {
            self.drawer = None;
        } else if self.timeline.read(cx).has_mentioned_user() {
            self.timeline
                .update(cx, |timeline, cx| timeline.close_mentioned_user(window, cx));
            return;
        }
        self.restore_overlay_focus(window, cx);
        cx.notify();
    }
    pub(super) fn render_overlays(
        &mut self,
        mut root: Stateful<Div>,
        mut members: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let viewport = window.viewport_size();
        let shade = Rgba {
            a: theme.backdrop.a * 0.52,
            ..theme.backdrop
        };
        if self.message_panel.read(cx).mode().is_some() {
            root = root.child(div().absolute().size_full().bg(shade));
            let panel = div()
                .id("message-panel-layer")
                .absolute()
                .right_0()
                .top_0()
                .h_full()
                .w(px(390_f32.min(f32::from(viewport.width) - 24.)))
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // Header action triggers switch modes themselves; don't dismiss first.
                    if event.position.y < px(HEADER_HEIGHT) {
                        return;
                    }
                    this.close_message_panel(window, cx);
                }))
                .child(self.message_panel.clone());
            root = if self.focused {
                root.child(motion::entrance(
                    panel,
                    ("message-panel-entrance", self.panel_entrance),
                ))
            } else {
                root.child(panel)
            };
        }
        if let Some(drawer) = self.drawer {
            root = root.child(
                div()
                    .id("explorer-backdrop")
                    .absolute()
                    .size_full()
                    .bg(shade)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.dismiss_overlay(window, cx)),
                    ),
            );
            let content = match drawer {
                Explorer::Channels => None,
                Explorer::Members => members.take(),
            };
            if let Some(content) = content {
                let member = drawer == Explorer::Members;
                let layer = div()
                    .id("explorer-layer")
                    .focus_trap("explorer-focus", &self.overlay_container)
                    .absolute()
                    .when(member, |layer| {
                        layer
                            .right_0()
                            .top_0()
                            .h_full()
                            .w(px(390_f32.min(f32::from(viewport.width) - 24.)))
                    })
                    .when(!member, |layer| {
                        layer
                            .left(px(8.))
                            .bottom(px(8.))
                            .w(px((f32::from(viewport.width) - 16.).min(720.)))
                            .h(px((f32::from(viewport.height) * 0.7).min(620.)))
                    })
                    .rounded_tl(px(theme.radius_lg))
                    .rounded_tr(px(if member { 0. } else { theme.radius_lg }))
                    .when(!(self.focused && member), |layer| layer.bg(theme.sidebar))
                    .overflow_hidden()
                    .occlude()
                    .child(content);
                root = if self.focused && member {
                    root.child(motion::entrance(
                        layer,
                        ("members-entrance", self.panel_entrance),
                    ))
                } else {
                    root.child(layer)
                };
            }
        }
        if let Some(settings) = self.channel_settings.clone() {
            if !self.focused && settings.read(cx).is_settings_open() {
                root = root.child(
                    div()
                        .id("channel-settings-backdrop")
                        .absolute()
                        .size_full()
                        .bg(theme.backdrop)
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.dismiss_overlay(window, cx)),
                        ),
                );
            }
            root = root.child(div().absolute().inset_0().child(settings));
        }
        if let Some(settings) = self.settings.clone() {
            if !self.focused && settings.read(cx).is_settings_open() {
                root = root.child(
                    div()
                        .id("settings-backdrop")
                        .absolute()
                        .size_full()
                        .bg(theme.backdrop)
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.dismiss_overlay(window, cx)),
                        ),
                );
            }
            let entrance_id = ("server-settings-entrance", settings.entity_id());
            root = root.child(motion::entrance(
                div()
                    .id("server-settings-layer")
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id("server-settings-surface")
                            .focus_trap("server-settings-focus", &self.overlay_container)
                            .size_full()
                            .child(settings),
                    ),
                entrance_id,
            ));
        }
        if let Some(settings) = self.account_settings.as_ref().filter(|_| !self.focused) {
            root = root.child(motion::account_settings(
                div().absolute().inset_0(),
                settings,
            ));
        }
        let timeline = self.timeline.clone();
        let action_surface = timeline.update(cx, |timeline, cx| {
            timeline.render_action_overlay(window, cx)
        });
        root = root
            .child(self.member_actions.clone())
            .children(action_surface)
            .child(self.context_menu.clone())
            .child(self.timeline.read(cx).action_menu())
            .child(self.image_viewer.clone())
            .child(
                cx.global::<crate::shared::file_picker::CropService>()
                    .0
                    .clone(),
            );
        root.children(self.voice.read(cx).music_playlist(theme, cx))
    }
}
