//! Home conversation list and its desktop rail shortcuts.
mod group;
mod pins;

use crate::features::shell::ShellView;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::FluentBuilder, *};
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientUser};
use stonks_core::messages::read;
use stonks_ui::i18n::t_vars;
use stonks_ui::{
    Theme,
    components::{Button, ButtonVariant},
    i18n::t,
    icons::Icon,
};

#[derive(Default)]
pub(in crate::features) struct Messages {
    oldest: bool,
    search: Option<(Entity<InputState>, Subscription)>,
    scope: Option<pins::Scope>,
    pinned: Option<pins::Pins>,
    pin_task: Option<Task<()>>,
    pin_error: Option<&'static str>,
    group: Option<group::Group>,
}

pub(in crate::features) fn direct_channels(
    snapshot: &AppSnapshot,
) -> impl Iterator<Item = &ClientChannel> {
    snapshot.channels.iter().filter(|channel| {
        channel.server.is_none()
            && matches!(channel.channel_type.as_str(), "DirectMessage" | "Group")
    })
}

pub(in crate::features) fn peer<'a>(
    channel: &ClientChannel,
    snapshot: &'a AppSnapshot,
) -> Option<&'a ClientUser> {
    let id = channel
        .recipients
        .iter()
        .find(|id| *id != &snapshot.current_user.id)?;
    snapshot.users.iter().find(|user| &user.id == id)
}

pub(in crate::features) fn label(channel: &ClientChannel, snapshot: &AppSnapshot) -> String {
    if channel.channel_type == "SavedMessages" {
        return t("navigation.savedNotes");
    }
    if let Some(name) = &channel.name {
        return name.clone();
    }
    let names = channel
        .recipients
        .iter()
        .filter(|id| *id != &snapshot.current_user.id)
        .filter_map(|id| snapshot.users.iter().find(|user| &user.id == id))
        .map(user_name)
        .collect::<Vec<_>>();
    if names.is_empty() {
        "Direct message".into()
    } else {
        names.join(", ")
    }
}

pub(in crate::features) fn user_name(user: &ClientUser) -> &str {
    user.display_name.as_deref().unwrap_or(&user.username)
}

pub(in crate::features) fn avatar_url(user: &ClientUser, autumn: Option<&str>) -> Option<String> {
    user.avatar
        .as_ref()
        .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id))
}

fn saved_notes_icon(color: Rgba) -> impl IntoElement {
    use std::{cell::RefCell, sync::Arc};
    thread_local! {
        static IMAGE: RefCell<Option<(u32, Arc<Image>)>> = const { RefCell::new(None) };
    }
    let byte = |level: f32| (level * 255.).round() as u32;
    let rgb = (byte(color.r) << 16) | (byte(color.g) << 8) | byte(color.b);
    let image = IMAGE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((key, image)) = cache.as_ref() {
            if *key == rgb { return image.clone(); }
        }
        // Same two strokes as the Saved notes row in ChannelSidebar.tsx.
        let source = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#{rgb:06x}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 4h11a3 3 0 0 1 3 3v13l-4-3-4 3-4-3-4 3V6a2 2 0 0 1 2-2Z"/><path d="M7 4v13"/></svg>"##);
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, source.into_bytes()));
        *cache = Some((rgb, image.clone()));
        image
    });
    img(image).size(px(18.))
}

impl Messages {
    pub(in crate::features) fn pinned(&self, id: &str) -> bool {
        self.pinned.as_ref().is_some_and(|pins| pins.contains(id))
    }
}

impl ShellView {
    /// Run on snapshot changes (or before rendering the shell).
    pub(in crate::features) fn sync_home_messages(&mut self, cx: &mut Context<Self>) {
        let session = self.home_session().read(cx);
        let scope =
            session
                .server
                .as_ref()
                .zip(session.snapshot.as_ref())
                .map(|(server, snapshot)| {
                    (
                        server.deployment_url.clone(),
                        snapshot.current_user.id.clone(),
                    )
                });
        if scope != self.home_messages.scope {
            self.home_messages = Messages {
                scope,
                ..Default::default()
            };
            self.update_home_pins(None, cx);
        }
    }

    fn update_home_pins(&mut self, toggle: Option<String>, cx: &mut Context<Self>) {
        let Some(scope) = self.home_messages.scope.clone() else {
            return;
        };
        let Some(path) = cx
            .try_global::<crate::preferences::NativePreferences>()
            .map(|prefs| prefs.path.with_file_name("conversation-pins.json"))
        else {
            return;
        };
        if self.home_messages.pin_task.is_some() {
            return;
        }
        let saving = toggle.is_some();
        self.home_messages.pin_error = None;
        self.home_messages.pin_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match toggle {
                        Some(id) => pins::toggle(&path, &scope, id),
                        None => pins::read(&path, &scope),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.home_messages.pin_task = None;
                match result {
                    Ok(pins) => this.home_messages.pinned = Some(pins),
                    Err(_) => {
                        this.home_messages.pin_error = Some(if saving {
                            "Could not save pinned conversations."
                        } else {
                            "Could not load pinned conversations."
                        })
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(in crate::features) fn open_home_conversation(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) {
        self.navigate_home_conversation(id, cx);
    }

    pub(in crate::features) fn render_home_messages(
        &self,
        snapshot: Option<&AppSnapshot>,
        focused: bool,
        bottom_inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = &self.home_messages;
        let mut panel = div().flex().flex_col().flex_1().min_h_0().min_w_0();
        if let Some(group) = &state.group {
            return panel.child(self.render_home_group(group, snapshot, bottom_inset, theme, cx));
        }
        let inset = if focused { 20. } else { theme.space4 };
        let sort_key = if state.oldest {
            "shell.oldest"
        } else {
            "shell.newest"
        };
        let sort = Button::new("home-message-sort", theme)
            .variant(if focused {
                ButtonVariant::Quiet
            } else {
                ButtonVariant::Secondary
            })
            .label(t(sort_key))
            .tooltip(t(if state.oldest {
                "shell.sortOldest"
            } else {
                "shell.sortNewest"
            }))
            .when(!focused, |button| {
                button.leading(Icon::ArrowUpDown.render(theme.text, 18.))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.home_messages.oldest = !this.home_messages.oldest;
                cx.notify();
            }));
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(px(theme.space2))
                .px(px(inset))
                .py(px(theme.space2))
                .child(div().flex_1().child(sort))
                .child(
                    Button::new("home-message-search", theme)
                        .variant(ButtonVariant::Icon)
                        .selected(state.search.is_some())
                        .tooltip(t("messages.search"))
                        .leading(Icon::Search.render(theme.text, 18.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.home_messages.search.take().is_none() {
                                let input = cx.new(|cx| {
                                    InputState::new(window, cx)
                                        .placeholder(t("shell.searchConversations"))
                                });
                                let sub =
                                    cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
                                input.update(cx, |input, cx| input.focus(window, cx));
                                this.home_messages.search = Some((input, sub));
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("home-message-create", theme)
                        .variant(ButtonVariant::Icon)
                        .tooltip(if focused {
                            "New conversation".into()
                        } else {
                            t("shell.createGroup")
                        })
                        .leading(Icon::Plus.render(theme.text, 18.))
                        .disabled(snapshot.is_none())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_home_group(window, cx)),
                        ),
                ),
        );
        if let Some((input, _)) = &state.search {
            panel = panel.child(div().px(px(inset)).child(Input::new(input)));
        }
        if let Some(error) = state.pin_error {
            panel = panel.child(div().px(px(inset)).text_color(theme.danger).child(error));
        }
        panel.child(
            div()
                .id("home-conversation-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(inset))
                .pb(bottom_inset + px(if focused { 24. } else { 8. }))
                .children(
                    snapshot
                        .map(|snapshot| self.home_conversation_rows(snapshot, focused, theme, cx)),
                ),
        )
    }

    fn home_conversation_rows(
        &self,
        snapshot: &AppSnapshot,
        focused: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let session = self.home_session().read(cx);
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.clone());
        let query = self
            .home_messages
            .search
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        let saved = snapshot
            .channels
            .iter()
            .find(|c| c.channel_type == "SavedMessages" && session.can_view_channel(&c.id));
        let mut entries: Vec<_> = direct_channels(snapshot)
            .filter(|c| session.can_view_channel(&c.id))
            .map(|c| (c, label(c, snapshot)))
            .filter(|(_, name)| name.to_lowercase().contains(&query))
            .collect();
        entries.sort_by(|(a, _), (b, _)| {
            let order = a
                .last_message_id
                .as_ref()
                .unwrap_or(&a.id)
                .cmp(b.last_message_id.as_ref().unwrap_or(&b.id))
                .then_with(|| a.id.cmp(&b.id));
            if self.home_messages.oldest {
                order
            } else {
                order.reverse()
            }
        });
        let mut rows = div().flex().flex_col().gap(px(4.));
        if let Some(channel) = saved {
            let id = channel.id.clone();
            rows = rows.child(
                Button::new("home-saved-notes", theme)
                    .variant(ButtonVariant::Channel)
                    .leading(
                        div()
                            .min_h(px(32.))
                            .flex()
                            .items_center()
                            .child(saved_notes_icon(theme.text_muted)),
                    )
                    .label(t("navigation.savedNotes"))
                    .selected(self.home_selected_channel() == Some(id.as_str()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_home_conversation(id.clone(), cx)
                    })),
            );
        }
        if entries.is_empty() {
            rows = rows.child(
                div()
                    .p(px(8.))
                    .text_color(theme.text_muted)
                    .child(t("shell.noDirectMessages")),
            );
        }
        for (channel, name) in entries {
            let peer = peer(channel, snapshot);
            let url = peer.and_then(|user| avatar_url(user, autumn.as_deref()));
            let image = self
                .home_images()
                .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
            let muted = self.home_channel_muted(&channel.id, cx);
            let unread = read::unread_badge(snapshot, channel);
            let id = channel.id.clone();
            let pin_id = id.clone();
            let menu_id = id.clone();
            let pinned = self.home_messages.pinned(&id);
            let pin_label = t_vars(
                if pinned {
                    "shell.unpinConversation"
                } else {
                    "shell.pinConversation"
                },
                &[("name", &name)],
            );
            let content = div()
                .flex()
                .items_center()
                .gap(px(if focused { 12. } else { 8. }))
                .w_full()
                .min_w_0()
                .child(div().flex_shrink_0().child(stonks_ui::components::avatar(
                    &theme,
                    peer.map_or(&name, user_name),
                    if focused { 40. } else { 30. },
                    image,
                )))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.replace(['\r', '\n'], " ")),
                )
                .when(peer.is_some_and(|user| user.bot.is_some()), |row| {
                    row.child(
                        div()
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.text_muted)
                            .child("BOT"),
                    )
                })
                .when_some(unread, |row, count| {
                    if count > 0 && !muted {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.))
                                .rounded(px(theme.radius_pill))
                                .bg(theme.danger)
                                .text_color(theme.danger_text)
                                .text_size(px(theme.font_size_xs))
                                .child(if count > 99 {
                                    "99+".into()
                                } else {
                                    count.to_string()
                                }),
                        )
                    } else {
                        row.child(
                            div()
                                .size(px(7.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(theme.text),
                        )
                    }
                });
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div().flex_1().min_w_0().child(
                            Button::new(
                                ElementId::Name(format!("home-conversation-{id}").into()),
                                theme,
                            )
                            .variant(ButtonVariant::Channel)
                            .selected(self.home_selected_channel() == Some(id.as_str()))
                            .accessibility_label(name)
                            .leading(content)
                            .on_right_click(cx.listener(
                                move |this, event: &MouseDownEvent, window, cx| {
                                    this.open_channel_menu(
                                        menu_id.clone(),
                                        event.position,
                                        window,
                                        cx,
                                    )
                                },
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.open_home_conversation(id.clone(), cx),
                            )),
                        ),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("home-pin-{pin_id}").into()), theme)
                            .variant(ButtonVariant::Quiet)
                            .icon_button_size(28.)
                            .selected(pinned)
                            .tooltip(pin_label.clone())
                            .accessibility_label(pin_label)
                            .leading(Icon::Pin.render(
                                if pinned {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                },
                                18.,
                            ))
                            .disabled(
                                self.home_messages.pinned.is_none()
                                    || self.home_messages.pin_task.is_some(),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_home_pins(Some(pin_id.clone()), cx)
                            })),
                    ),
            );
        }
        rows
    }
}
