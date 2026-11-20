use super::{
    relationship_actions, relationship_kind, t, Action, ClientUser, Filter, Friends, FriendsEvent,
    RelationshipKind,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::{connection::AppSnapshot, media};
use stonks_ui::{
    components::{
        input_field, presence_avatar, BotBadge, Button, ButtonVariant, Feedback, FeedbackTone,
        SettingsInput, UiDialog,
    },
    icons::Icon,
    theme::Theme,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The shell sidebar owns filters.
    Desktop,
    /// Standalone Tauri panel, with its own 168px filter rail.
    WithRail,
    /// Content of the KMP-style Messages > Friends tab; the shell owns tabs.
    Focused,
}

impl Filter {
    const ALL: [Self; 4] = [Self::All, Self::Online, Self::Pending, Self::Blocked];

    fn people(self, snapshot: &AppSnapshot) -> impl Iterator<Item = &ClientUser> {
        snapshot
            .users
            .iter()
            .filter(move |user| user.id != snapshot.current_user.id && self.matches(user))
    }

    pub(crate) fn label(self) -> String {
        t(match self {
            Self::All => "shell.friendAll",
            Self::Online => "shell.friendOnline",
            Self::Pending => "shell.friendPending",
            Self::Blocked => "shell.friendBlocked",
        })
    }

    fn heading(self) -> String {
        t(match self {
            Self::All => "shell.friendHeading.all",
            Self::Online => "shell.friendHeading.online",
            Self::Pending => "shell.friendHeading.pending",
            Self::Blocked => "shell.friendHeading.blocked",
        })
    }
}

impl Friends {
    pub(crate) fn controls(
        &self,
        theme: &Theme,
        horizontal: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.session.read(cx).snapshot.as_ref();
        div()
            .id("friends-controls")
            .flex()
            .when(horizontal, |d| d.items_center().overflow_x_scroll())
            .when(!horizontal, |d| d.flex_col())
            .gap(px(theme.space2))
            .child(
                div()
                    .flex_shrink_0()
                    .when(!horizontal, |d| d.mb(px(theme.space2)))
                    .child(
                        Button::new("friends-add", theme.clone())
                            .leading(Icon::UserPlus.render(theme.accent_text, 18.))
                            .label(t("relationships.add"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_add(window, cx))),
                    ),
            )
            .children(Filter::ALL.into_iter().enumerate().map(|(index, filter)| {
                let count = snapshot.map_or(0, |snapshot| filter.people(snapshot).count());
                div().flex_shrink_0().child(
                    Button::new(("friends-filter", index), theme.clone())
                        .variant(ButtonVariant::Nav)
                        .selected(self.filter() == filter)
                        .label(format!("{} ({count})", filter.label()))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx))),
                )
            }))
    }

    pub(crate) fn body(
        &self,
        theme: &Theme,
        layout: Layout,
        bottom_inset: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = layout == Layout::Focused;
        let session = self.session.read(cx);
        let mut people = session
            .snapshot
            .iter()
            .flat_map(|snapshot| self.filter.people(snapshot))
            .cloned()
            .collect::<Vec<_>>();
        if focused {
            people.sort_by_cached_key(|user| (name(user).to_lowercase(), user.id.clone()));
        }
        let count = people.len();
        let rows: Vec<_> = people
            .into_iter()
            .map(|user| self.row(&user, theme, focused, cx))
            .collect();
        let content = div()
            .id("friends-list")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(theme.space5))
            .pt(px(if focused { theme.space2 } else { theme.space5 }))
            .pb(bottom_inset + px(if focused { theme.space6 } else { theme.space5 }))
            .when(!focused || self.filter != Filter::All, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .mb(px(theme.space3))
                        .child(
                            div()
                                .text_size(px(theme.font_size_lg))
                                .font_weight(FontWeight::BOLD)
                                .child(self.filter.heading()),
                        )
                        .child(
                            div()
                                .text_size(px(theme.font_size_sm))
                                .text_color(theme.text_muted)
                                .child(count.to_string()),
                        ),
                )
            })
            .children(rows)
            .when(count == 0, |d| {
                d.child(
                    div()
                        .p(px(theme.space6))
                        .text_center()
                        .text_color(theme.text_muted)
                        .child(t("relationships.empty")),
                )
            });
        div()
            .flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .text_color(theme.text)
            .when(!focused, |d| d.rounded(px(32.)).bg(theme.surface))
            .when(focused, |d| {
                d.flex_col().child(
                    div()
                        .px(px(theme.space5))
                        .py(px(theme.space2))
                        .child(self.controls(theme, true, cx)),
                )
            })
            .when(layout == Layout::WithRail, |d| {
                d.child(
                    div()
                        .w(px(168.))
                        .flex_shrink_0()
                        .p(px(theme.space3))
                        .bg(theme.sidebar)
                        .border_r_1()
                        .border_color(theme.outline)
                        .child(self.controls(theme, false, cx)),
                )
            })
            .child(content)
    }

    fn row(
        &self,
        user: &ClientUser,
        theme: &Theme,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.session.read(cx);
        let endpoints = session.server.as_ref().map(|server| &server.endpoints);
        let url = user
            .avatar
            .as_ref()
            .and_then(|file| {
                media::avatar_url(
                    endpoints.and_then(|endpoints| endpoints.autumn.as_deref()),
                    &file.tag,
                    &file.id,
                )
            })
            .or_else(|| {
                media::file_url(endpoints.map(|e| e.api.as_str()), "users", &user.id)
                    .map(|url| format!("{url}/default_avatar"))
            });
        let image = self
            .images
            .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
        let presence = user
            .status
            .as_ref()
            .and_then(|status| status.presence.as_deref());
        let state = relationship_kind(user.relationship.as_deref());
        let detail = match state {
            RelationshipKind::Friend if focused => user
                .status
                .as_ref()
                .and_then(|status| status.text.as_deref().or(status.presence.as_deref()))
                .unwrap_or(if user.online { "Online" } else { "Offline" })
                .to_owned(),
            RelationshipKind::Friend => presence_label(user.online, presence).into(),
            RelationshipKind::Incoming => t("relationships.incoming"),
            RelationshipKind::Outgoing => t("relationships.outgoing"),
            RelationshipKind::Pending => t("relationships.pending"),
            RelationshipKind::Blocked => t("shell.friendBlocked"),
            _ => t("relationships.unavailable"),
        };
        let detail = if focused {
            detail
        } else {
            let discriminator = user
                .discriminator
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!("#{s}"))
                .unwrap_or_default();
            format!("@{}{discriminator} · {detail}", user.username)
        };
        let target = user.id.clone();
        let identity = div().min_w_0().flex_1().overflow_hidden().child(
            Button::new("friend-profile", theme.clone())
                .variant(ButtonVariant::MemberRow)
                .accessibility_label(format!("Open {}'s profile", name(user)))
                .leading(div().flex_shrink_0().child(presence_avatar(
                    theme,
                    name(user),
                    user.online,
                    presence,
                    image,
                )))
                .trailing(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .min_w_0()
                        .gap(px(theme.space1))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .min_w_0()
                                .gap(px(theme.space1))
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(if focused {
                                            16.
                                        } else {
                                            theme.font_size_md
                                        }))
                                        .child(name(user).to_owned()),
                                )
                                .when(user.bot.is_some(), |d| {
                                    d.child(BotBadge::new(theme.clone()))
                                }),
                        )
                        .child(
                            div()
                                .truncate()
                                .max_w_full()
                                .text_size(px(theme.font_size_sm))
                                .text_color(theme.text_muted)
                                .child(detail),
                        ),
                )
                .on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                    cx.emit(FriendsEvent::OpenProfile {
                        user_id: target.clone(),
                        anchor: event.position(),
                    })
                })),
        );
        let pending = self.pending.get(&user.id).map(|(action, _)| *action);
        let actions = relationship_actions(user.relationship.as_deref(), user.bot.is_some());
        div()
            .id(SharedString::from(user.id.clone()))
            .flex()
            .flex_col()
            .min_w_0()
            .justify_center()
            .min_h(px(if focused { 56. } else { 68. }))
            .py(px(if focused { 0. } else { theme.space2 }))
            .when(!focused, |d| {
                d.px(px(theme.space2))
                    .rounded(px(theme.radius_md))
                    .hover(|style| style.bg(theme.surface_interactive))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .child(identity)
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap(px(if focused { theme.space1 } else { theme.space2 }))
                            .children(actions.into_iter().enumerate().filter_map(
                                |(index, action)| {
                                    let (icon, label, busy_label) = action_copy(action)?;
                                    let label = t(if pending == Some(action) {
                                        busy_label
                                    } else {
                                        label
                                    });
                                    let (variant, color) = if action.dangerous() {
                                        (ButtonVariant::DangerSubtle, theme.danger)
                                    } else if matches!(
                                        action,
                                        Action::Message | Action::Accept | Action::Add
                                    ) {
                                        (ButtonVariant::Primary, theme.accent_text)
                                    } else {
                                        (ButtonVariant::Secondary, theme.text)
                                    };
                                    let id = user.id.clone();
                                    Some(
                                        Button::new(("friend-action", index), theme.clone())
                                            .variant(variant)
                                            .icon_button_size(34.)
                                            .leading(icon.render(color, 18.))
                                            .accessibility_label(label.clone())
                                            .tooltip(label)
                                            .disabled(pending.is_some())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.perform(&id, action, cx)
                                            })),
                                    )
                                },
                            )),
                    ),
            )
            .children(self.errors.get(&user.id).map(|error| {
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.danger)
                    .child(error.clone())
            }))
    }

    pub(crate) fn overlay(&self, theme: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.add_open {
            return None;
        }
        let busy = self.add_request.is_some();
        let empty = self.add_input.read(cx).value().trim().is_empty();
        let label = t("relationships.username");
        Some(
            UiDialog::new(
                "friends-add-dialog",
                theme.clone(),
                t("relationships.addTitle"),
            )
            .width(440.)
            .viewport_inset(16.)
            .gap(theme.space4)
            .busy(busy)
            .on_close(cx.listener(|this, _, window, cx| this.close_add(window, cx)))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_sm))
                    .child(t("relationships.addDescription")),
            )
            .child(
                div().children(
                    self.add_error.iter().map(|error| {
                        Feedback::new(theme.clone(), FeedbackTone::Error, error.clone())
                    }),
                ),
            )
            .child(input_field(
                theme,
                &label,
                "",
                SettingsInput::new(&self.add_input, theme.clone())
                    .label(label.clone())
                    .disabled(busy),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .flex_wrap()
                    .gap(px(theme.space2))
                    .child(
                        Button::new("friends-add-cancel", theme.clone())
                            .variant(ButtonVariant::Secondary)
                            .label(t("common.cancel"))
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_add(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("friends-add-send", theme.clone())
                            .label(t(if busy {
                                "relationships.sending"
                            } else {
                                "relationships.sendRequest"
                            }))
                            .disabled(busy || empty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_add(window, cx)),
                            ),
                    ),
            )
            .into_any_element(),
        )
    }
}

fn name(user: &ClientUser) -> &str {
    user.display_name.as_deref().unwrap_or(&user.username)
}

fn presence_label(online: bool, presence: Option<&str>) -> &'static str {
    if !online {
        return "offline";
    }
    match presence.unwrap_or_default().to_ascii_lowercase().as_str() {
        "idle" | "away" => "away",
        "focus" => "focus",
        "busy" => "busy",
        "offline" | "invisible" => "offline",
        _ => "online",
    }
}

fn action_copy(action: Action) -> Option<(Icon, &'static str, &'static str)> {
    Some(match action {
        Action::Message => (
            Icon::MessageCircle,
            "relationships.message",
            "relationships.opening",
        ),
        Action::Add => (Icon::UserPlus, "relationships.add", "relationships.adding"),
        Action::Accept => (
            Icon::Check,
            "relationships.accept",
            "relationships.accepting",
        ),
        Action::Decline => (Icon::X, "relationships.decline", "relationships.declining"),
        Action::Cancel => (
            Icon::CircleX,
            "relationships.cancel",
            "relationships.cancelling",
        ),
        Action::Remove => (
            Icon::UserMinus,
            "relationships.remove",
            "relationships.removing",
        ),
        Action::Block => (Icon::Ban, "relationships.block", "relationships.blocking"),
        Action::Unblock => (
            Icon::LockOpen,
            "relationships.unblock",
            "relationships.unblocking",
        ),
        _ => return None,
    })
}
