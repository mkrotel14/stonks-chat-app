//! Compose Home features and translate their events into shell navigation.
use super::*;
use crate::features::home::friends::{Friends, FriendsEvent, Layout};

thread_local! {
    static HOME_BRAND: Arc<Image> = Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../launcher/com.rdxlabs.stonks.gpui.png").to_vec(),
    ));
}

pub(super) struct MountedFriends {
    view: Entity<Friends>,
    _observe: Subscription,
    _events: Subscription,
}

impl ShellView {
    pub(super) fn sync_friends(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home_friends.is_some() || self.session.read(cx).snapshot.is_none() {
            return;
        }
        let view = cx.new(|cx| {
            Friends::new(
                self.policy.clone(),
                self.session.clone(),
                self.images.clone(),
                window,
                cx,
            )
        });
        let observe = cx.observe(&view, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            FriendsEvent::OpenConversation(channel) => {
                this.open_home_conversation(channel.id.clone(), cx)
            }
            FriendsEvent::OpenProfile { user_id, anchor } => {
                this.open_member_profile(String::new(), user_id.clone(), *anchor, window, cx)
            }
        });
        self.home_friends = Some(MountedFriends {
            view,
            _observe: observe,
            _events: events,
        });
    }

    pub(in crate::features) fn home_session(&self) -> &Entity<Session> {
        &self.session
    }
    pub(in crate::features) fn home_images(&self) -> &Entity<Images> {
        &self.images
    }
    pub(in crate::features) fn home_selected_channel(&self) -> Option<&str> {
        self.selected_channel.as_deref()
    }
    pub(in crate::features) fn home_channel_muted(&self, id: &str, cx: &App) -> bool {
        self.notifications
            .read(cx)
            .saved
            .is_muted(id, navigation::notifications::now())
    }

    pub(in crate::features) fn navigate_home_conversation(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) {
        self.home = true;
        self.selected_server = None;
        self.select_text_view(id, cx);
    }

    pub(super) fn show_home(&mut self, friends: bool, cx: &mut Context<Self>) {
        self.home = true;
        self.drawer = None;
        if self.focused {
            self.compact_nav.show_friends(friends);
        } else {
            self.wide_friends = friends;
        }
        self.selected_server = None;
        self.selected_channel = None;
        self.ensure_selection(cx);
    }

    pub(super) fn home_tabs(&self, friends: bool, theme: Theme, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .gap(px(theme.space2))
            .px(px(theme.space4))
            .py(px(theme.space2))
            .children(
                [(false, "shell.messages"), (true, "navigation.friends")].map(|(tab, label)| {
                    div().flex_1().child(
                        Button::new(("home-tab", tab as usize), theme)
                            .variant(ButtonVariant::Quiet)
                            .selected(friends == tab)
                            .label(t(label))
                            .on_click(cx.listener(move |this, _, _, cx| this.show_home(tab, cx))),
                    )
                }),
            )
    }

    pub(super) fn friends_controls(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.home_friends.as_ref().map(|friends| {
            friends.view.update(cx, |view, cx| {
                view.controls(&theme, false, cx).into_any_element()
            })
        })
    }

    pub(super) fn friends_body(
        &self,
        focused: bool,
        sidebar: bool,
        inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let layout = if focused {
            Layout::Focused
        } else if sidebar {
            Layout::Desktop
        } else {
            Layout::WithRail
        };
        self.home_friends.as_ref().map(|friends| {
            friends.view.update(cx, |view, cx| {
                view.body(&theme, layout, inset, cx).into_any_element()
            })
        })
    }

    pub(super) fn friends_dialog_open(&self, cx: &App) -> bool {
        self.home_friends
            .as_ref()
            .is_some_and(|friends| friends.view.read(cx).add_is_open())
    }

    pub(super) fn dismiss_friends_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(friends) = &self.home_friends {
            friends
                .view
                .update(cx, |view, cx| view.close_add(window, cx));
        }
    }

    pub(super) fn friends_overlay(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.home_friends
            .as_ref()?
            .view
            .update(cx, |view, cx| view.overlay(&theme, cx))
    }

    pub(super) fn home_landing(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let wordmark_size = (f32::from(window.viewport_size().width) * 0.07).clamp(44., 72.);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(theme.space6))
            .child(
                img(HOME_BRAND.with(Arc::clone))
                    .size(px(112.))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .text_center()
                    .text_color(theme.text)
                    .text_size(px(wordmark_size))
                    .font_weight(FontWeight(850.))
                    .child("Stonks Chat"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(theme.space2))
                    .child(
                        Button::new("home-create-group", theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t("shell.createGroup"))
                            .disabled(self.session.read(cx).snapshot.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_channels = true;
                                this.open_home_group(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("home-settings", theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t("home.settings.title"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_account_settings(window, cx)
                            })),
                    ),
            )
    }
}
