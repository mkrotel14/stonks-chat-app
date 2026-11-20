//! Responsive layout and shell composition.

use super::{people::roster, *};
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientServer};
use stonks_ui::fonts;

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.overlay_focus.is_some() && !self.has_overlay(cx) {
            self.restore_overlay_focus(window, cx);
        }
        let theme = Theme::current(cx);
        let viewport = f32::from(window.viewport_size().width);
        let focused = viewport < 800.;
        if focused != self.focused {
            self.drawer = None;
            self.reset_rail_preview();
            self.drag = None;
            if focused {
                self.wide_selection = Some((
                    self.home,
                    self.selected_server.clone(),
                    self.selected_channel.clone(),
                ));
            } else {
                if let Some((home, server, channel)) = self.wide_selection.take() {
                    self.home = home;
                    self.selected_server = server;
                    self.selected_channel = channel;
                }
                if self
                    .account_settings
                    .as_ref()
                    .is_some_and(|settings| settings.read(cx).is_home())
                {
                    self.clear_account_settings(cx);
                    self.restore_overlay_focus(window, cx);
                }
            }
            self.focused = focused;
            if focused && self.compact_nav.tab == compact::Tab::You {
                self.open_account_settings(window, cx);
            }
            self.ensure_selection(cx);
        }
        let rail_width = self.rail_width();
        let (channel_width, member_width) = self.fitted_explorer_widths(viewport);
        let wide_channels = !focused && channel_width > 0.;
        let wide_members = !focused && member_width > 0.;
        // Clone the snapshot so listeners can borrow `cx` mutably below.
        let snapshot = self.session.read(cx).snapshot.clone();
        let snapshot = snapshot.as_ref();

        let servers: &[ClientServer] = snapshot.map_or(&[], |snapshot| &snapshot.servers);
        let active_server = self
            .selected_server
            .as_ref()
            .and_then(|id| servers.iter().find(|server| &server.id == id))
            .or_else(|| {
                if focused || self.home {
                    None
                } else {
                    servers.first()
                }
            });
        let channels: Vec<&ClientChannel> = match (snapshot, active_server) {
            (Some(snapshot), Some(server)) => server
                .channels
                .iter()
                .filter_map(|id| snapshot.channels.iter().find(|channel| &channel.id == id))
                .filter(|channel| self.session.read(cx).can_view_channel(&channel.id))
                .collect(),
            (Some(snapshot), None) if self.home => snapshot
                .channels
                .iter()
                .filter(|channel| {
                    channel.server.is_none() && self.session.read(cx).can_view_channel(&channel.id)
                })
                .collect(),
            _ => Vec::new(),
        };
        let channel = self
            .selected_channel
            .as_ref()
            .and_then(|id| channels.iter().copied().find(|channel| &channel.id == id))
            .filter(|channel| !channel.is_voice());

        let deployment = self.session.read(cx).server.clone();
        let preview = if focused {
            0.
        } else {
            self.rail_preview(window)
        };
        let rail = (!focused && (!self.rail_hidden() || preview > 0.)).then(|| {
            self.render_server_rail(
                servers,
                active_server,
                snapshot,
                deployment.as_ref(),
                theme,
                cx,
            )
        });
        if rail.is_none() {
            self.images
                .update(cx, |images, _| images.set_sources("rail", []));
        }
        let dock_side_inset = if focused {
            (16. - theme.space2).max(0.)
        } else {
            0.
        };
        let sidebar_width = if focused {
            (viewport - 2. * dock_side_inset).max(1.)
        } else {
            channel_width
        };
        let docked = wide_channels || focused;
        if !focused {
            self.images
                .update(cx, |images, _| images.set_sources("voice-dock", []));
        }
        let (dock, voice_error) = self.voice.update(cx, |voice, cx| {
            (
                voice.dock(
                    docked.then_some(sidebar_width),
                    focused.then_some((
                        snapshot,
                        deployment
                            .as_ref()
                            .and_then(|server| server.endpoints.autumn.as_deref()),
                        &self.images,
                    )),
                    snapshot,
                    &self.images,
                    window,
                    cx,
                ),
                voice.error_notice(cx),
            )
        });
        let (compact_dock, dock, voice_error) = if focused {
            (
                dock.or_else(|| voice_error.map(|notice| notice.m(px(theme.space2)))),
                None,
                None,
            )
        } else {
            (None, dock, voice_error)
        };
        let (sidebar_footer, floating_dock, floating_error) = if docked {
            (
                dock.or_else(|| voice_error.map(|notice| notice.m(px(theme.space2)))),
                None,
                None,
            )
        } else {
            (
                None,
                dock,
                voice_error.map(|notice| {
                    notice
                        .absolute()
                        .right(px(12.))
                        .bottom(px(64.))
                        .w(px(276.).min(px((viewport - 24.).max(1.))))
                }),
            )
        };
        self.sync_composer(channel, snapshot, deployment.as_ref(), window, cx);
        self.timeline.update(cx, |timeline, cx| {
            timeline.set_insets(
                (if focused { 0. } else { rail_width })
                    + if self.show_channels && wide_channels {
                        channel_width
                    } else {
                        0.
                    }
                    + if self.show_members && !self.home && wide_members {
                        member_width
                    } else {
                        0.
                    }
                    + 111.,
                cx,
            )
        });

        let deployment = self.session.read(cx).server.clone();
        let groups = match (active_server, snapshot) {
            (Some(server), Some(snapshot)) => roster::groups(
                server,
                channel,
                snapshot,
                deployment
                    .as_ref()
                    .and_then(|server| server.endpoints.autumn.as_deref()),
                deployment
                    .as_ref()
                    .map(|server| server.endpoints.api.as_str()),
            ),
            _ => Vec::new(),
        };
        let member_panel_width = if focused {
            390_f32.min(viewport - 24.)
        } else {
            member_width
        };
        let members_panel = if (self.show_members && !self.home && wide_members)
            || self.drawer == Some(Explorer::Members)
        {
            Some(
                self.render_members(
                    groups,
                    active_server
                        .map(|server| server.id.clone())
                        .unwrap_or_default(),
                    &theme,
                    member_panel_width,
                    cx,
                )
                .into_any_element(),
            )
        } else {
            self.images.update(cx, |images, _| {
                images.set_sources("members", Vec::<(String, u32)>::new())
            });
            None
        };
        use compact::{Route, Tab};
        let route = self.compact_nav.route();
        let detail = matches!(route, Route::Conversation(_) | Route::Chat(_, _));
        let home_list = matches!(route, Route::Messages | Route::Friends);
        let focused_friends = route == Route::Friends;
        let channel_list = matches!(route, Route::Channels(_)) && active_server.is_some();
        let you = self.compact_nav.tab == Tab::You;
        let settings_home = self
            .account_settings
            .as_ref()
            .is_some_and(|settings| settings.read(cx).is_home());
        let immersive = focused
            && ((self.account_settings.is_some() && !settings_home)
                || self
                    .settings
                    .as_ref()
                    .is_some_and(|settings| settings.read(cx).is_settings_open())
                || self
                    .channel_settings
                    .as_ref()
                    .is_some_and(|settings| settings.read(cx).is_settings_open()));
        let (bottom_inset, compact_overlay) = if focused {
            self.compact_overlay(compact_dock, immersive, snapshot, theme, window, cx)
        } else {
            (px(0.), None)
        };
        let sidebar_width = if focused { viewport } else { sidebar_width };
        let sidebar = if !focused && self.home {
            div()
                .w(px(sidebar_width))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(HEADER_HEIGHT))
                        .flex_shrink_0()
                        .child(self.home_tabs(self.wide_friends, theme, cx).w_full()),
                )
                .child(if self.wide_friends {
                    div()
                        .flex_1()
                        .min_h_0()
                        .px(px(theme.space4))
                        .children(self.friends_controls(theme, cx))
                } else {
                    self.render_home_messages(snapshot, false, px(0.), theme, cx)
                })
                .children(sidebar_footer)
        } else {
            self.render_channel_sidebar(
                &channels,
                active_server,
                snapshot,
                sidebar_width,
                sidebar_footer,
                bottom_inset,
                theme,
                cx,
            )
        };
        let mut pane = (!focused || (detail && channel.is_some())).then(|| {
            self.render_conversation(
                channel,
                active_server,
                snapshot,
                focused,
                bottom_inset,
                theme,
                window,
                cx,
            )
        });
        let mut compact_body = if focused {
            let content = if let Some(settings) = &self.account_settings {
                if settings_home {
                    settings.update(cx, |settings, cx| {
                        settings.set_home_bottom_inset(bottom_inset, cx);
                    });
                }
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .when(settings_home, |body| {
                        body.child(self.compact_header(
                            "You".into(),
                            false,
                            false,
                            window,
                            theme,
                            cx,
                        ))
                    })
                    .child(motion::account_settings(
                        div().relative().flex_1().min_h_0(),
                        settings,
                    ))
            } else if let Some(chat) = pane.take() {
                chat
            } else {
                let title = match self.compact_nav.route() {
                    Route::Messages | Route::Friends => t("shell.messages"),
                    Route::Servers => "Servers".into(),
                    Route::You => "You".into(),
                    Route::Channels(_) if channel_list => active_server
                        .map(|server| server.name.clone())
                        .unwrap_or_default(),
                    _ => "Unavailable".into(),
                };
                let back = matches!(
                    self.compact_nav.route(),
                    Route::Channels(_) | Route::Chat(_, _) | Route::Conversation(_)
                );
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(self.compact_header(
                        title,
                        back,
                        active_server.is_some(),
                        window,
                        theme,
                        cx,
                    ))
                    .when(home_list, |body| {
                        body.child(self.home_tabs(focused_friends, theme, cx))
                            .child(if focused_friends {
                                div().flex_1().min_h_0().children(self.friends_body(
                                    true,
                                    false,
                                    bottom_inset,
                                    theme,
                                    cx,
                                ))
                            } else {
                                self.render_home_messages(snapshot, true, bottom_inset, theme, cx)
                            })
                    })
                    .when(self.compact_nav.tab == Tab::Servers && !back, |body| {
                        body.child(self.compact_servers(servers, bottom_inset, theme, cx))
                    })
            };
            Some(content)
        } else {
            None
        };

        let mut sidebar = Some(sidebar.into_any_element());
        let mut members_panel = members_panel;
        let mut rail = rail.map(IntoElement::into_any_element);
        let mut root = div()
            .id("shell-root")
            .relative()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.voice.read(cx).music_playlist_open(cx) {
                    this.voice
                        .update(cx, |voice, cx| voice.dismiss_music_playlist(window, cx));
                    cx.stop_propagation();
                    window.prevent_default();
                } else if event.keystroke.key == "escape" && this.voice.read(cx).camera_setup_open()
                {
                    this.voice
                        .update(cx, |voice, cx| voice.dismiss_camera_setup(window, cx));
                    cx.stop_propagation();
                    window.prevent_default();
                } else if event.keystroke.key == "escape" && this.voice.read(cx).screen_setup_open()
                {
                    this.voice
                        .update(cx, |voice, cx| voice.dismiss_screen_setup(window, cx));
                    cx.stop_propagation();
                    window.prevent_default();
                } else if event.keystroke.key == "escape"
                    && this.account_settings.is_none()
                    && this.voice.read(cx).soundboard_open()
                {
                    this.voice
                        .update(cx, |voice, cx| voice.dismiss_soundboard(window, cx));
                    cx.stop_propagation();
                    window.prevent_default();
                } else if event.keystroke.key == "escape"
                    && this.has_overlay(cx)
                    && !this.context_menu.read(cx).is_open()
                    && !this.timeline.read(cx).action_menu().read(cx).is_open()
                    && !this.timeline.read(cx).has_action_ui(cx)
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    this.dismiss_overlay(window, cx);
                    cx.stop_propagation();
                    window.prevent_default();
                } else if event.keystroke.key == "escape"
                    && this.focused
                    && !this.has_overlay(cx)
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    if !this.shortcut(event, window, cx)
                        && super::shortcuts::chord(&event.keystroke).as_deref() == Some("Escape")
                        && this.compact_back(window, cx)
                    {
                        cx.stop_propagation();
                        window.prevent_default();
                    }
                } else {
                    this.shortcut(event, window, cx);
                }
            }))
            .size_full()
            .flex()
            .bg(theme.sidebar)
            .text_color(theme.text)
            .font_family(fonts::FAMILY)
            .when(!focused && !self.rail_hidden(), |root| {
                root.children(rail.take())
            })
            .when(self.show_channels && wide_channels, |root| {
                root.child(sidebar.take().unwrap())
            })
            .children(pane)
            .when(focused, |root| {
                let mut body = compact_body.take().unwrap();
                if channel_list && !you && self.account_settings.is_none() {
                    body = body.child(div().flex_1().min_h_0().child(sidebar.take().unwrap()));
                }
                root.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .relative()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .when(self.account_settings.is_some() && !settings_home, |body| {
                                    body.pb(bottom_inset)
                                })
                                .child(body),
                        )
                        .when(
                            !(detail && channel.is_some())
                                && self.account_settings.is_none()
                                && bottom_inset > px(0.),
                            |column| column.child(compact_bottom_fade(bottom_inset, theme.sidebar)),
                        )
                        .when(settings_home && bottom_inset > px(0.), |column| {
                            column.child(compact_bottom_fade(bottom_inset, theme.canvas))
                        })
                        .children(compact_overlay),
                )
            })
            .when(self.show_members && !self.home && wide_members, |root| {
                root.child(members_panel.take().unwrap())
            })
            .children(floating_dock)
            .children(floating_error);
        if !focused {
            root = self.resize_handles(
                root,
                channel_width,
                member_width,
                wide_channels && self.show_channels,
                wide_members && self.show_members && !self.home,
                &theme,
                cx,
            );
        }
        if !focused && self.rail_hidden() {
            root = root.child(self.rail_edge(rail, preview, theme, cx));
        }
        let root = root.children(self.friends_overlay(theme, cx));
        let root = self.render_overlays(root, members_panel, window, cx);
        root.children(
            self.voice
                .update(cx, |voice, cx| voice.soundboard_surface(window, cx)),
        )
        .children(
            self.voice
                .update(cx, |voice, cx| voice.screen_setup_surface(cx)),
        )
        .children(
            self.voice
                .update(cx, |voice, cx| voice.camera_setup_surface(window, cx)),
        )
    }
}

const FOCUSED_TABS_HEIGHT: Pixels = px(44. + compact::TAB_BOTTOM_MARGIN);

impl ShellView {
    fn compact_overlay(
        &self,
        dock: Option<Div>,
        immersive: bool,
        snapshot: Option<&AppSnapshot>,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Pixels, Option<Div>) {
        if immersive && dock.is_none() {
            return (px(0.), None);
        }
        let tabs_height = if immersive {
            px(0.)
        } else {
            FOCUSED_TABS_HEIGHT
        };
        let dock_height = window.use_keyed_state("focused-dock-height", cx, |_, _| px(0.));
        let bottom_inset = tabs_height
            + if dock.is_some() {
                *dock_height.read(cx)
            } else {
                px(0.)
            };
        let overlay = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .when_some(dock, |stack, dock| {
                stack.child(
                    div()
                        .relative()
                        .w_full()
                        .px(px((16. - theme.space2).max(0.)))
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    let dock_height = dock_height.clone();
                                    cx.defer(move |cx| {
                                        dock_height.update(cx, |height, cx| {
                                            if (*height - bounds.size.height).abs() >= px(0.5) {
                                                *height = bounds.size.height;
                                                cx.notify();
                                            }
                                        })
                                    });
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .child(dock.block_mouse_except_scroll()),
                )
            })
            .when(!immersive, |stack| {
                stack.child(self.compact_tabs(snapshot, theme, cx))
            });
        (bottom_inset, Some(overlay))
    }
}

pub(super) fn compact_bottom_fade(bottom_inset: Pixels, color: Rgba) -> Div {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(bottom_inset + px(56.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(Rgba { a: 0., ..color }, 0.),
            linear_color_stop(color, 1.),
        ))
}
