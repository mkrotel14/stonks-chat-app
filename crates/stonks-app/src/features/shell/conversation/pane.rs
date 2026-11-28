//! Conversation pane, header controls, and file-drop handling.

use super::super::{
    navigation::view::{channel_icon, channel_label},
    update::header_update,
    *,
};
use crate::desktop::main_window_drag_region;
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientServer};

impl ShellView {
    pub(in crate::features::shell) fn close_message_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message_panel
            .update(cx, |panel, cx| panel.reset(window, cx));
        self.restore_overlay_focus(window, cx);
        cx.notify();
    }

    pub(in crate::features::shell) fn toggle_message_panel(
        &mut self,
        mode: PanelMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message_panel.read(cx).mode() == Some(mode) {
            self.close_message_panel(window, cx);
            return;
        }
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        if self.overlay_focus.is_none() {
            self.overlay_focus = window.focused(cx);
        }
        self.drawer = None;
        self.panel_entrance = self.panel_entrance.wrapping_add(1);
        self.message_panel
            .update(cx, |panel, cx| panel.open(mode, channel, window, cx));
        cx.notify();
    }

    pub(in crate::features::shell) fn render_conversation(
        &mut self,
        channel: Option<&ClientChannel>,
        active_server: Option<&ClientServer>,
        snapshot: Option<&AppSnapshot>,
        focused: bool,
        bottom_inset: Pixels,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let pane_header_title = channel
            .map(|channel| channel_label(channel, snapshot))
            .unwrap_or_else(|| {
                if self.home {
                    t(if self.wide_friends {
                        "navigation.friends"
                    } else {
                        "shell.messages"
                    })
                } else {
                    String::new()
                }
            });
        let pane_header_icon = channel.map(|channel| channel_icon(channel));
        let has_channel = channel.is_some();
        let server_channel = active_server.is_some() && has_channel;
        let text_channel = has_channel;
        let can_settings = active_server.is_some_and(|server| {
            self.session
                .read(cx)
                .can_server_permission(&server.id, 1 << 1)
                || self
                    .session
                    .read(cx)
                    .can_server_permission(&server.id, 1 << 4)
        });
        if let Some(settings) = self.settings.as_ref() {
            let permitted = self
                .session
                .read(cx)
                .can_manage_server(settings.read(cx).server_id());
            settings.update(cx, |settings, cx| settings.set_can_manage(permitted, cx));
        }
        let panel_mode = self.message_panel.read(cx).mode();
        if let Some(channel) = channel {
            self.message_panel.update(cx, |panel, cx| {
                panel.set_channel(channel.id.clone(), window, cx);
            });
        } else if panel_mode.is_some() {
            self.close_message_panel(window, cx);
        }

        let read_error = self
            .shell_error
            .clone()
            .or_else(|| self.notifications.read(cx).error.clone())
            .or_else(|| self.session.read(cx).read_error.clone());
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(theme.sidebar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px(px(if focused { theme.space2 } else { theme.space5 }))
                    .gap(px(if focused { theme.space1 } else { theme.space4 }))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(theme.space1))
                            .when(focused, |title| {
                                title.child(self.compact_back_button(theme, cx))
                            })
                            .when(!focused && self.rail_hidden(), |title| {
                                title.child(
                                    Button::new("restore-servers", theme)
                                        .variant(ButtonVariant::IconSmall)
                                        .tooltip(t("shell.showServers"))
                                        .accessibility_label(t("shell.showServers"))
                                        .leading(
                                            Icon::PanelLeftOpen.render_stroke(theme.text, 18., 1.8),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_rail_hidden(false, cx);
                                        })),
                                )
                            })
                            .when(!focused, |title| {
                                title.child(
                                    Button::new("toggle-channels", theme)
                                        .variant(ButtonVariant::IconSmall)
                                        .tooltip(t(
                                            if self
                                                .fitted_explorer_widths(f32::from(
                                                    window.viewport_size().width,
                                                ))
                                                .0
                                                > 0.
                                            {
                                                "shell.hideChannels"
                                            } else {
                                                "shell.showChannels"
                                            },
                                        ))
                                        .leading(
                                            Icon::PanelLeft.render_stroke(theme.text, 18., 1.8),
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_explorer(Explorer::Channels, window, cx)
                                        })),
                                )
                            })
                            .child(main_window_drag_region(
                                div()
                                    .id("pane-header-drag")
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap(px(theme.space1))
                                    .when_some(pane_header_icon, |header, icon| {
                                        header.child(if icon == Icon::Hash {
                                            div()
                                                .text_size(px(theme.font_size_xl))
                                                .text_color(theme.text_muted)
                                                .child("#")
                                                .into_any_element()
                                        } else {
                                            icon.render_stroke(theme.text_muted, 18., 1.8)
                                                .into_any_element()
                                        })
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(px(theme.font_size_md))
                                            .child(pane_header_title),
                                    ),
                                window,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(theme.space1))
                            .children(header_update(theme, cx))
                            .when(can_settings && has_channel, |actions| {
                                actions.child(
                                    Button::new("header-settings", theme)
                                        .variant(ButtonVariant::Icon)
                                        .tooltip(t("shell.openServerSettings"))
                                        .leading(Icon::Settings.render_stroke(theme.text, 18., 1.8))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_settings(window, cx)
                                        })),
                                )
                            })
                            .when(text_channel, |actions| {
                                actions
                                    .child(
                                        Button::new("header-pins", theme)
                                            .variant(ButtonVariant::Icon)
                                            .tooltip(t("messages.pinned"))
                                            .selected(panel_mode == Some(PanelMode::Pins))
                                            .leading(Icon::Pin.render_stroke(theme.text, 18., 1.8))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_message_panel(
                                                    PanelMode::Pins,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new("header-search", theme)
                                            .variant(ButtonVariant::Icon)
                                            .tooltip(t("messages.search"))
                                            .selected(panel_mode == Some(PanelMode::Search))
                                            .leading(
                                                Icon::Search.render_stroke(theme.text, 18., 1.8),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_message_panel(
                                                    PanelMode::Search,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                            })
                            .when(server_channel, |actions| {
                                actions.child(
                                    Button::new("toggle-members", theme)
                                        .variant(ButtonVariant::Icon)
                                        .tooltip(t(if self.show_members {
                                            "shell.hideMembers"
                                        } else {
                                            "shell.showMembers"
                                        }))
                                        .selected(if focused {
                                            self.drawer == Some(Explorer::Members)
                                        } else {
                                            self.show_members
                                        })
                                        .leading(Icon::Users.render_stroke(theme.text, 18., 1.8))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_explorer(Explorer::Members, window, cx)
                                        })),
                                )
                            }),
                    )
                    .children(crate::desktop::main_window_controls(&theme, window)),
            )
            .children(read_error.map(|error| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .px(px(theme.space4))
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_xs))
                    .child(error)
                    .child(
                        Button::new("read-retry", theme)
                            .variant(ButtonVariant::Quiet)
                            .label(t("common.retry"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shell_error = None;
                                this.notifications.update(cx, |preferences, cx| {
                                    preferences.retry(cx);
                                });
                                this.session
                                    .update(cx, |session, cx| session.retry_read(cx))
                            })),
                    )
            }))
            .child(if !focused && self.home && !text_channel {
                let content = if self.wide_friends {
                    self.friends_body(false, self.show_channels, px(0.), theme, cx)
                } else {
                    Some(self.home_landing(theme, window, cx).into_any_element())
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .mx(px(8.))
                    .mb(px(8.))
                    .rounded(px(32.))
                    .overflow_hidden()
                    .bg(theme.surface)
                    .children(content)
                    .into_any_element()
            } else {
                self.render_channel_frame(text_channel, focused, bottom_inset, theme, cx)
                    .into_any_element()
            })
    }

    fn render_channel_frame(
        &self,
        text_channel: bool,
        focused: bool,
        bottom_inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("channel-frame")
            .relative()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .ml(px(8.))
            .mr(px(8.))
            .mb(px(8.))
            .rounded(px(32.))
            .overflow_hidden()
            .bg(theme.surface)
            .flex_col()
            .when(text_channel, |frame| {
                let timeline = self.timeline.downgrade();
                let shell = cx.entity().downgrade();
                let drop_files = move |paths: &ExternalPaths, window: &mut Window, cx: &mut App| {
                    let _ = shell.update(cx, |this, cx| {
                        if !this.has_overlay(cx) {
                            this.composer.update(cx, |composer, cx| {
                                composer.drop_files(paths, window, cx);
                            });
                        }
                    });
                };
                frame
                    .on_drop(drop_files.clone())
                    .child(div().absolute().inset_0().child(self.timeline.clone()))
                    .when(bottom_inset > px(0.), |frame| {
                        frame.child(super::super::render::compact_bottom_fade(
                            bottom_inset,
                            theme.surface,
                        ))
                    })
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            // The focused overlay already reserves its own clearance.
                            .bottom(if focused && bottom_inset > px(0.) {
                                bottom_inset
                            } else {
                                px(8.)
                            })
                            // Keep clicks on composer padding and disabled controls off the timeline.
                            .block_mouse_except_scroll()
                            .id("composer-key-scope")
                            .on_drop(drop_files)
                            .capture_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "escape"
                                        && !this.has_overlay(cx)
                                        && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                                        && this.composer.update(cx, |composer, cx| {
                                            composer.cancel_reply(window, cx)
                                        })
                                    {
                                        cx.stop_propagation();
                                        window.prevent_default();
                                    }
                                },
                            ))
                            .child(
                                canvas(
                                    move |bounds, _, cx| {
                                        let timeline = timeline.clone();
                                        cx.defer(move |cx| {
                                            let _ = timeline.update(cx, |timeline, cx| {
                                                timeline.set_composer_height(
                                                    bounds.size.height.into(),
                                                    bottom_inset.into(),
                                                    cx,
                                                );
                                            });
                                        });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .child(self.composer.clone()),
                    )
                    .when(
                        !self.has_overlay(cx) && self.composer.read(cx).upload_allowed(),
                        |frame| {
                            frame.child(
                                div()
                                    .id("channel-file-drop-hint")
                                    .absolute()
                                    .inset_0()
                                    .m(px(theme.space3))
                                    .rounded(px(theme.radius_lg))
                                    .border_2()
                                    .border_color(theme.accent_outline)
                                    .bg(theme.surface_selected)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .opacity(0.)
                                    .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
                                    .child(
                                        div()
                                            .rounded(px(theme.radius_md))
                                            .p(px(theme.space3))
                                            .bg(theme.surface_raised)
                                            .text_color(theme.text)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t("messages.dropFilesToAttach")),
                                    ),
                            )
                        },
                    )
            })
            .when(!text_channel, |frame| {
                frame.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.text_muted)
                        .child(t("shell.noChannel")),
                )
            })
            // GPUI's overflow mask is rectangular. Keep scrolling
            // rows/images inside ShellLayout.channel's rounded edge.
            .child(
                gpui_kit::base::RoundedFrameCover::uniform(px(32.)).backdrop(theme.sidebar.into()),
            )
    }
}
