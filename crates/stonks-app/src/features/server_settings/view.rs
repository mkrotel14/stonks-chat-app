//! Settings frame, navigation, and page composition.
use super::*;
use stonks_ui::components::UiCloseButton;

impl ServerSettings {
    pub(super) fn icon(&self, theme: &Theme, size: f32, cx: &mut App) -> AnyElement {
        let image = self.images.update(cx, |images, cx| {
            images.get(self.icon_url.as_deref(), size as u32, cx)
        });
        avatar(theme, &self.server.name, size, image).into_any_element()
    }
    fn navigation(&self, theme: Theme, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut nav = div()
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
            .text_size(px(theme.font_size_md))
            .line_height(relative(1.5))
            // SettingsShell.page clips its navigation to a 20px frame.
            // GPUI overflow only clips rectangles: the painted navigation
            // background must carry the adjoining frame corners itself.
            .rounded_tl(px(if compact { 0. } else { 20. }))
            .rounded_bl(px(if compact { 0. } else { 20. }));
        if compact {
            nav = nav.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .pb(px(theme.space3))
                    .child(
                        Button::new("server-settings-index-back", theme)
                            .variant(ButtonVariant::IconSmall)
                            .label("‹")
                            .accessibility_label("Back")
                            .tooltip("Back")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(t("serverSettings.title")),
            );
        }
        nav = nav.child(
            div()
                .flex()
                .items_center()
                .gap(px(11.))
                .mx(px(7.))
                .mb(px(13.))
                .p(px(theme.space2))
                .child(self.icon(&theme, 40., cx))
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
                                .child(self.server.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_ellipsis()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .child(t("serverSettings.title")),
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
                    .line_height(px(theme.font_size_xs * theme.line_height_normal))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(theme.text_muted)
                    .child(self.server.name.to_uppercase()),
            );
        for (index, (page, symbol)) in [
            (Page::Overview, "●"),
            (Page::Emojis, "☺"),
            (Page::Roles, "⚑"),
            (Page::Invites, "↗"),
        ]
        .into_iter()
        .enumerate()
        {
            if page == Page::Invites && !self.allowed(1 << 1) {
                continue;
            }
            let row = Button::new(
                ElementId::Name(format!("server-settings-nav-{index}").into()),
                theme,
            )
            .variant(ButtonVariant::Nav)
            .track_focus(&self.extra.navigation_focus[index])
            .selected(page == self.page)
            .disabled(self.busy)
            .leading(div().w(px(18.)).text_color(theme.text_muted).child(symbol))
            .label(t(page.key()))
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)));
            group = group.child(if compact {
                row.trailing(Icon::ChevronRight.render(theme.text_muted, 16.))
            } else {
                row
            });
        }
        nav.child(
            div()
                .id("server-settings-navigation")
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
}
impl Render for ServerSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_pending_snapshot(window, cx);
        self.sync_feedback(cx);
        if self.extra.restore_focus {
            self.extra.restore_focus = false;
            if let Some(focus) = self.extra.previous_focus.take() {
                focus.focus(window, cx);
            }
        }
        let theme = Theme::current(cx);
        let viewport = window.viewport_size();
        let compact = f32::from(viewport.width) < 800.;
        if compact
            && !self.compact_layout
            && self.extra.frame_visible
            && self.extra.dialog.is_none()
        {
            if self.settings_index {
                self.extra.navigation_focus[self.page.index()].focus(window, cx);
            } else if self
                .extra
                .navigation_focus
                .iter()
                .any(|focus| focus.is_focused(window))
            {
                self.extra.heading_focus.focus(window, cx);
            }
        }
        self.compact_layout = compact;
        let index = compact && self.settings_index;
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
        let mut sources: Vec<_> = self
            .icon_url
            .iter()
            .map(|url| (url.clone(), 96))
            .chain(self.icon_url.iter().map(|url| (url.clone(), 40)))
            .collect();
        if !index && self.page == Page::Emojis {
            if let Some(emojis) = &self.emojis {
                sources.extend(emojis.iter().filter_map(|emoji| {
                    media::file_url(self.autumn.as_deref(), "emojis", &emoji.id)
                        .map(|url| (url, 48))
                }));
            }
        }
        sources.extend(self.extra_sources());
        self.images.update(cx, |images, _| {
            images.set_sources("server-settings", sources)
        });
        let mut frame = div()
            .id("server-settings-frame")
            .relative()
            .flex()
            .when(compact, |frame| frame.occlude())
            .w(px(width))
            .h(px(height))
            .min_w_0()
            .overflow_hidden()
            .rounded(px(if compact { 0. } else { 20. }))
            .bg(theme.surface)
            .text_color(theme.text)
            // The source document body uses --line-body:1.5; controls with an
            // explicit token line-height retain their own value.
            .text_size(px(theme.font_size_md))
            .line_height(relative(1.5))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if !compact {
            frame = frame.child(self.navigation(theme, false, cx));
        }
        if index {
            frame = frame.child(self.navigation(theme, true, cx));
        } else {
            let padding = if compact { 16. } else { 36. };
            let mut body = div()
                .id(if compact {
                    self.page.key()
                } else {
                    "server-settings-content"
                })
                .flex()
                .flex_col()
                .gap(px(theme.space4))
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .pt(px(if compact { 16. } else { 28. }))
                .px(px(padding))
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
                                // CSS's inline eyebrow occupies the inherited 14px body
                                // line box even though its own glyphs are 12px.
                                .line_height(px(theme.font_size_md * 1.5))
                                .font_weight(FontWeight(750.))
                                .child(t("serverSettings.title").to_uppercase()),
                        )
                        .child(
                            div()
                                .id("server-settings-heading")
                                .track_focus(&self.extra.heading_focus)
                                .tab_index(-1)
                                .text_size(px(27.))
                                .line_height(px(27. * theme.line_height_compact))
                                .font_weight(FontWeight::BOLD)
                                .child(t(self.page.key())),
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
                            Button::new("server-settings-back", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.settings"))
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                        )
                    })
                    .when(!compact, |header| {
                        header.child(
                            UiCloseButton::new("server-settings-close", theme)
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
                Page::Overview => self.overview(
                    theme,
                    width - if compact { 0. } else { 236. } - padding * 2.,
                    cx,
                ),
                Page::Roles => self.render_roles(
                    theme,
                    window,
                    width - if compact { 0. } else { 236. } - padding * 2.,
                    cx,
                ),
                Page::Invites => self.render_invites(theme, cx),
                Page::Emojis => self.render_emojis(
                    theme,
                    width - if compact { 0. } else { 236. } - padding * 2.,
                    cx,
                ),
            });
            frame = frame.child(body);
        }
        let mut root = div()
            .relative()
            .flex()
            .w(px(f32::from(viewport.width)))
            .h(px(f32::from(viewport.height)))
            .items_center()
            .justify_center();
        if self.extra.frame_visible {
            root = root.child(frame);
        }
        if let Some(dialog) = self.render_dialog(theme, cx) {
            root = root.child(dialog);
        }
        root
    }
}
