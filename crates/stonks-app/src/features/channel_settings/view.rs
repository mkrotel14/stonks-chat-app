use super::*;
use gpui_kit::{base::FocusTrapElement as _, prelude::FluentBuilder as _};
impl ChannelSettings {
    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.settings_index = false;
        if self.compact_layout {
            self.heading_focus.focus(window, cx);
        }
        cx.notify();
    }
    fn navigation(&self, theme: Theme, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut navigation = div()
            .id("channel-settings-navigation-host")
            .track_focus(&self.navigation_focus)
            .flex()
            .flex_col()
            .when(compact, |nav| nav.w_full())
            .when(!compact, |nav| nav.w(px(236.)))
            .h_full()
            .flex_shrink_0()
            .pt(px(22.))
            .px(px(12.))
            .pb(px(14.))
            .bg(theme.canvas)
            .rounded_tl(px(if compact { 0. } else { 20. }))
            .rounded_bl(px(if compact { 0. } else { 20. }));
        if compact {
            navigation = navigation.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .pb(px(theme.space3))
                    .child(
                        Button::new("channel-settings-index-back", theme)
                            .variant(ButtonVariant::IconSmall)
                            .label("‹")
                            .accessibility_label("Back")
                            .tooltip("Back")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child("Channel settings"),
            );
        }
        navigation = navigation.child(
            div()
                .flex()
                .items_center()
                .gap(px(11.))
                .mx(px(7.))
                .mb(px(13.))
                .p(px(theme.space2))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(40.))
                        .flex_shrink_0()
                        .rounded(px(theme.radius_pill))
                        .bg(theme.surface_interactive)
                        .text_color(theme.accent)
                        .text_size(px(theme.font_size_lg))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child("#"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_ellipsis()
                                .text_size(px(theme.font_size_md))
                                .font_weight(FontWeight::BOLD)
                                .child(self.channel.name.clone().unwrap_or_default()),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .child("Channel settings"),
                        ),
                ),
        );
        let mut group = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(theme.space1))
            .mb(px(theme.space3))
            .child(
                div()
                    .mx(px(theme.space2))
                    .mb(px(theme.space1))
                    .text_size(px(theme.font_size_xs))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(theme.text_muted)
                    .child("CHANNEL SETTINGS"),
            );
        for (index, page) in [Page::Overview, Page::Permissions].into_iter().enumerate() {
            let row = Button::new(
                ElementId::Name(format!("channel-settings-nav-{index}").into()),
                theme,
            )
            .variant(ButtonVariant::Nav)
            .track_focus(&self.page_focus[index])
            .selected(self.page == page)
            .label(page.label())
            .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)));
            group = group.child(if compact {
                row.trailing(Icon::ChevronRight.render(theme.text_muted, 16.))
            } else {
                row
            });
        }
        navigation
            .child(
                div()
                    .id("channel-settings-navigation")
                    .role(Role::Navigation)
                    .aria_label("Channel settings sections")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(4.))
                    .px(px(4.))
                    .pb(px(12.))
                    .child(group),
            )
            .into_any_element()
    }
    fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let viewport = window.viewport_size();
        let compact = f32::from(viewport.width) < 800.;
        if compact != self.compact_layout && self.confirmation.is_none() {
            if compact && self.settings_index {
                self.page_focus[self.page.index()].focus(window, cx);
            } else if compact && self.navigation_focus.contains_focused(window, cx) {
                self.heading_focus.focus(window, cx);
            }
        }
        self.compact_layout = compact;
        let width = if compact {
            f32::from(viewport.width)
        } else {
            1040_f32.min(f32::from(viewport.width) - 36.).max(1.)
        };
        let height = if compact {
            f32::from(viewport.height)
        } else {
            720_f32.min(f32::from(viewport.height) - 36.).max(1.)
        };
        let mut frame = div()
            .id("channel-settings-frame")
            .role(Role::Dialog)
            .aria_label(format!(
                "{} settings",
                self.channel.name.as_deref().unwrap_or("Channel")
            ))
            .relative()
            .flex()
            .when(compact, |frame| frame.occlude())
            .w(px(width))
            .h(px(height))
            .min_w_0()
            .overflow_hidden()
            .rounded(px(if compact { 0. } else { 20. }))
            .bg(theme.surface)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .text_color(theme.text)
            .text_size(px(theme.font_size_md))
            .line_height(relative(theme.line_height_normal))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    this.close(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));
        if !compact {
            frame = frame.child(self.navigation(theme, false, cx));
        }
        if compact && self.settings_index {
            return frame
                .child(self.navigation(theme, true, cx))
                .focus_trap("channel-settings-focus-trap", &self.focus)
                .into_any_element();
        }
        let mut body = div()
            .id("channel-settings-content")
            .flex()
            .flex_col()
            .gap(px(theme.space4))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .pt(px(if compact { 16. } else { 28. }))
            .px(px(if compact { 16. } else { 36. }))
            .pb(px(if compact { 24. } else { 40. }));
        let heading = div()
            .flex()
            .flex_col()
            .items_start()
            .min_w_0()
            .gap(px(theme.space3))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(px(theme.space1))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(theme.font_size_sm))
                            .h(px(theme.font_size_md * theme.line_height_normal))
                            .line_height(px(theme.font_size_md * theme.line_height_normal))
                            .font_weight(FontWeight(750.))
                            .child("CHANNEL SETTINGS"),
                    )
                    .child(
                        div()
                            .id("channel-settings-heading")
                            .track_focus(&self.heading_focus)
                            .tab_index(-1)
                            .role(Role::Heading)
                            .aria_level(1)
                            .text_size(px(27.))
                            .line_height(relative(theme.line_height_compact))
                            .font_weight(FontWeight::BOLD)
                            .child(self.page.label()),
                    ),
            );
        body = body.child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(px(theme.space3))
                .child(heading)
                .when(compact, |header| {
                    header.child(
                        Button::new("channel-settings-back", theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t("common.settings"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                })
                .when(!compact, |header| {
                    header.child(
                        UiCloseButton::new("channel-settings-close", theme)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                }),
        );
        if let Some(error) = &self.error {
            body = body.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if let Some(notice) = &self.notice {
            body = body.child(Feedback::new(theme, FeedbackTone::Success, notice.clone()));
        }
        body = body.child(match self.page {
            Page::Overview => self.overview(theme, cx),
            Page::Permissions => self.permissions(
                theme,
                width - if compact { 0. } else { 236. } - if compact { 32. } else { 72. },
                cx,
            ),
        });
        frame = frame.child(body);
        frame
            .focus_trap("channel-settings-focus-trap", &self.focus)
            .into_any_element()
    }
}
impl Render for ChannelSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        if let Some(target) = self.pending_target.take() {
            target.focus(window, cx);
        } else if self.pending_focus {
            self.focus(window, cx);
            self.pending_focus = false;
        }
        let mut root = div()
            .id("channel-settings-root")
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center();
        if self.frame_visible {
            root = root.child(self.frame(window, cx));
        } else if self.busy {
            root = root.track_focus(&self.focus);
        }
        if let Some(action) = self.confirmation {
            root = root.child(self.render_confirmation(action, theme, cx));
        }
        root
    }
}
