//! Member roster presentation and interactions.

use super::super::*;
use super::roster::MemberGroup;

impl ShellView {
    pub(in crate::features::shell) fn render_members(
        &mut self,
        groups: Vec<MemberGroup>,
        server_id: String,
        theme: &Theme,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let total: usize = groups.iter().map(|group| group.members.len()).sum();
        self.images.update(cx, |images, _| {
            images.set_sources(
                "members",
                groups.iter().flat_map(|group| {
                    group
                        .members
                        .iter()
                        .filter_map(|entry| entry.avatar_url.clone())
                        .map(|url| (url, 80))
                        .chain(group.icon_url.iter().cloned().map(|url| (url, 34)))
                }),
            )
        });
        let mut content = div()
            .id("member-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(px(theme.space3))
            .pt(px(theme.space4))
            .pb(px(theme.space5))
            .gap(px(theme.space5))
            .overflow_y_scroll();
        for group in groups {
            let group_id = group.id.clone();
            let collapsed = self.collapsed_groups.contains(&group.id);
            let icon = self.images.update(cx, |images, cx| {
                images.get(group.icon_url.as_deref(), 34, cx)
            });
            let mut heading = div()
                .flex()
                .items_center()
                .w_full()
                .min_w_0()
                .text_size(px(theme.font_size_xs))
                .text_color(theme.text_muted)
                .gap(px(theme.space2))
                .child(motion::disclosure(
                    format!("member-disclosure-{group_id}"),
                    collapsed,
                    theme.text_muted,
                    14.,
                    true,
                ));
            if let Some(icon) = icon {
                heading = heading.child(img(icon).size(px(17.)).rounded(px(theme.radius_pill)));
            }
            heading =
                heading.child(format!("{} — {}", group.name, group.members.len()).to_uppercase());
            let mut section = div().flex().flex_col().gap(px(theme.space2)).child(
                Button::new(
                    ElementId::Name(format!("member-group-{group_id}").into()),
                    *theme,
                )
                .variant(ButtonVariant::Heading)
                .expanded(!collapsed)
                .leading(heading)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.collapsed_groups.remove(&group_id) {
                        this.collapsed_groups.insert(group_id.clone());
                    }
                    cx.notify();
                })),
            );
            if !collapsed {
                let mut rows = div()
                    .flex()
                    .flex_col()
                    .pl(px(theme.space4))
                    .gap(px(theme.space1));
                for entry in group.members {
                    let menu_user = entry.id.clone();
                    let profile_user = entry.id.clone();
                    let bounds_user = entry.id.clone();
                    let menu_server = server_id.clone();
                    let profile_server = server_id.clone();
                    let owner = cx.entity().downgrade();
                    let image = self.images.update(cx, |images, cx| {
                        images.get(entry.avatar_url.as_deref(), 80, cx)
                    });
                    let mut name = div()
                        .flex()
                        .items_center()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(theme.font_size_md))
                        .text_color(entry.colour.map(rgb).unwrap_or(if entry.online {
                            theme.text
                        } else {
                            theme.text_muted
                        }))
                        .child(div().min_w_0().text_ellipsis().child(entry.name.clone()));
                    if entry.bot {
                        name = name.child(stonks_ui::components::BotBadge::new(*theme));
                    }
                    let row = Button::new(
                        ElementId::Name(format!("member-{}", entry.id).into()),
                        *theme,
                    )
                    .variant(ButtonVariant::MemberRow)
                    .tooltip(entry.name.clone())
                    .leading(
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap(px(theme.space3))
                            .child(stonks_ui::components::presence_avatar(
                                theme,
                                &entry.name,
                                entry.online,
                                entry.presence.as_deref(),
                                image,
                            ))
                            .child(name),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.open_member_profile(
                            profile_server.clone(),
                            profile_user.clone(),
                            event.position(),
                            window,
                            cx,
                        )
                    }))
                    .on_right_click(cx.listener(
                        move |this, event: &MouseDownEvent, window, cx| {
                            this.open_member_menu(
                                menu_server.clone(),
                                menu_user.clone(),
                                event.position,
                                window,
                                cx,
                            )
                        },
                    ));
                    rows = rows.child(
                        div()
                            .relative()
                            .w_full()
                            .opacity(if entry.online { 1. } else { 0.72 })
                            .child(
                                canvas(
                                    move |bounds, _, cx| {
                                        let owner = owner.clone();
                                        let user = bounds_user.clone();
                                        cx.defer(move |cx| {
                                            owner
                                                .update(cx, |this, _| {
                                                    this.member_bounds.insert(user, bounds);
                                                })
                                                .ok();
                                        });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .child(row),
                    );
                }
                section = section.child(rows);
            }
            content = content.child(section);
        }
        if let Some(error) = self.session.read(cx).member_error.clone() {
            content = content
                .child(div().text_color(theme.danger).child(error))
                .child(
                    Button::new("members-retry", *theme)
                        .variant(ButtonVariant::Quiet)
                        .label(t("common.retry"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.selected_server.clone() {
                                this.session
                                    .update(cx, |session, cx| session.load_members(id, true, cx));
                            }
                        })),
                );
        }
        let focused = self.focused;
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .when(focused, |panel| {
                panel
                    .rounded_tl(px(theme.radius_lg))
                    .rounded_bl(px(theme.radius_lg))
            })
            .child(
                div()
                    .h(px(if focused { 66. } else { HEADER_HEIGHT }))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .px(px(theme.space4))
                    .text_size(px(theme.font_size_sm))
                    .font_weight(FontWeight::BOLD)
                    .child(t("navigation.members"))
                    .child(
                        div()
                            .min_w(px(19.))
                            .h(px(19.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .px(px(theme.space1))
                            .rounded(px(theme.radius_pill))
                            .bg(theme.surface_interactive)
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.text_muted)
                            .child(total.to_string()),
                    )
                    .when(focused, |header| {
                        header.child(div().flex_1()).child(
                            stonks_ui::components::UiCloseButton::new(
                                "focused-members-close",
                                *theme,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.dismiss_overlay(window, cx);
                                },
                            )),
                        )
                    }),
            )
            .child(content)
    }
}
