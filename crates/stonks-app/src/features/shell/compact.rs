//! Compact home routes, independent of the wide shell's automatic selection.
use super::*;
use stonks_core::connection::{AppSnapshot, ClientServer};
use stonks_ui::components::draggable_chrome;

pub(super) const TAB_BOTTOM_MARGIN: f32 = 12.;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Messages,
    Servers,
    You,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Route<'a> {
    Messages,
    Friends,
    Conversation(&'a str),
    Servers,
    Channels(&'a str),
    Chat(&'a str, &'a str),
    You,
}

#[derive(Default)]
pub(super) struct CompactNav {
    pub tab: Tab,
    conversation: Option<String>,
    friends: bool,
    server: Option<(String, Option<String>)>,
}

impl CompactNav {
    pub fn route(&self) -> Route<'_> {
        match self.tab {
            Tab::Messages => self.conversation.as_deref().map_or(
                if self.friends {
                    Route::Friends
                } else {
                    Route::Messages
                },
                Route::Conversation,
            ),
            Tab::Servers => match &self.server {
                Some((server, Some(channel))) => Route::Chat(server, channel),
                Some((server, None)) => Route::Channels(server),
                None => Route::Servers,
            },
            Tab::You => Route::You,
        }
    }

    fn select_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            match tab {
                Tab::Messages => self.conversation = None,
                Tab::Servers => self.server = None,
                Tab::You => {}
            }
        }
        self.tab = tab;
    }

    pub fn show_friends(&mut self, friends: bool) {
        self.tab = Tab::Messages;
        self.conversation = None;
        self.friends = friends;
    }

    pub fn open_server(&mut self, server: String) {
        self.tab = Tab::Servers;
        self.server = Some((server, None));
    }

    pub fn open_chat(&mut self, server: Option<String>, channel: String) {
        if let Some(server) = server {
            self.tab = Tab::Servers;
            self.server = Some((server, Some(channel)));
        } else {
            self.tab = Tab::Messages;
            self.conversation = Some(channel);
        }
    }

    fn back(&mut self) -> bool {
        match self.route() {
            Route::Conversation(_) => self.conversation = None,
            Route::Chat(_, _) => {
                if let Some((_, channel)) = &mut self.server {
                    *channel = None;
                }
            }
            Route::Channels(_) => self.server = None,
            Route::Servers | Route::You => self.tab = Tab::Messages,
            Route::Friends => self.friends = false,
            Route::Messages => return false,
        }
        true
    }
}

impl ShellView {
    pub(super) fn compact_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.compact_nav.back() {
            self.apply_compact_route(window, cx);
            true
        } else {
            false
        }
    }

    pub(super) fn apply_compact_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drawer = None;
        self.message_panel
            .update(cx, |panel, cx| panel.reset(window, cx));
        if self.compact_nav.tab == Tab::You {
            self.open_account_settings(window, cx);
        } else if self
            .account_settings
            .as_ref()
            .is_some_and(|settings| settings.read(cx).is_home())
        {
            self.clear_account_settings(cx);
            self.restore_overlay_focus(window, cx);
        }
        self.ensure_selection(cx);
    }

    pub(super) fn compact_header(
        &self,
        title: String,
        back: bool,
        server: bool,
        window: &mut Window,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .px(px(theme.space2))
            .gap(px(theme.space2))
            .when(back, |header| {
                header.child(self.compact_back_button(theme, cx))
            })
            .child(draggable_chrome(
                div()
                    .id("compact-title")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .child(div().truncate().font_weight(FontWeight::BOLD).child(title)),
                window,
                cx,
            ))
            .children(super::update::header_update(theme, cx))
            .when(server, |header| {
                header.child(
                    Button::new("compact-members", theme)
                        .variant(ButtonVariant::Icon)
                        .leading(Icon::Users.render(theme.text, 18.))
                        .tooltip(t("shell.showMembers"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_explorer(Explorer::Members, window, cx)
                        })),
                )
            })
            .when(
                window.window_controls().minimize || cfg!(target_os = "windows"),
                |header| header.child(crate::desktop::main_window_controls(&theme, window)),
            )
    }

    pub(super) fn compact_back_button(&self, theme: Theme, cx: &mut Context<Self>) -> Button {
        Button::new("compact-back", theme)
            .variant(ButtonVariant::IconSmall)
            .label("‹")
            .accessibility_label("Back")
            .tooltip("Back")
            .on_click(cx.listener(|this, _, window, cx| {
                this.compact_back(window, cx);
            }))
    }

    pub(super) fn compact_servers(
        &self,
        servers: &[ClientServer],
        bottom_inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div()
            .id("compact-servers")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(theme.space2))
            .pb(px(theme.space2) + bottom_inset);
        let autumn = self
            .session
            .read(cx)
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.clone());
        for server in servers {
            let id = server.id.clone();
            let url = server.icon.as_ref().and_then(|file| {
                stonks_core::media::file_url(autumn.as_deref(), &file.tag, &file.id)
            });
            let image = self
                .images
                .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
            list = list.child(
                Button::new(
                    ElementId::Name(format!("compact-server-{id}").into()),
                    theme,
                )
                .variant(ButtonVariant::Nav)
                .label(server.name.clone())
                .leading(stonks_ui::components::avatar(
                    &theme,
                    &server.name,
                    32.,
                    image,
                ))
                .trailing(Icon::ChevronRight.render(theme.text_muted, 16.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.compact_nav.open_server(id.clone());
                    this.apply_compact_route(window, cx);
                })),
            );
        }
        div().flex().flex_col().flex_1().min_h_0().child(list)
    }

    pub(super) fn compact_tabs(
        &self,
        snapshot: Option<&AppSnapshot>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let user = snapshot.map(|snapshot| &snapshot.current_user);
        let autumn = self
            .session
            .read(cx)
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let url = user
            .and_then(|user| user.avatar.as_ref())
            .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id));
        // The rail and You tab share the same decoded image, including animation frames.
        let image = self
            .images
            .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
        let name = user.map_or("?", |user| {
            user.display_name.as_deref().unwrap_or(&user.username)
        });

        let mut bar = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .py(px(4.))
            .mb(px(TAB_BOTTOM_MARGIN))
            .rounded(px(theme.radius_pill))
            .bg(theme.surface_raised)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .block_mouse_except_scroll();
        for (tab, id, label, icon) in [
            (
                Tab::Messages,
                "tab-messages",
                t("shell.messages"),
                Some(Icon::MessageCircle),
            ),
            (
                Tab::Servers,
                "tab-servers",
                "Servers".into(),
                Some(Icon::Server),
            ),
            (Tab::You, "tab-you", "You".into(), None),
        ] {
            let selected = self.compact_nav.tab == tab;
            let tint = if selected {
                theme.accent
            } else {
                theme.text_muted
            };
            let mut button = Button::new(id, theme)
                .variant(ButtonVariant::FloatingTab)
                .accessibility_label(label.clone())
                .tooltip(label.clone())
                .label(label)
                .selected(selected);
            button = if let Some(icon) = icon {
                button.leading(icon.render(tint, 20.))
            } else {
                button.leading(stonks_ui::components::avatar(
                    &theme,
                    name,
                    20.,
                    image.clone(),
                ))
            };

            bar = bar.child(button.on_click(cx.listener(move |this, _, window, cx| {
                this.compact_nav.select_tab(tab);
                this.apply_compact_route(window, cx);
            })));
        }
        bar
    }
}

#[cfg(test)]
mod tests {
    use super::{CompactNav, Route, Tab};

    #[test]
    fn tabs_restore_details_and_reselect_resets_only_current_tab() {
        let mut nav = CompactNav::default();
        assert_eq!(nav.route(), Route::Messages);
        nav.open_chat(None, "dm".into());
        nav.select_tab(Tab::Servers);
        nav.open_server("s".into());
        nav.open_chat(Some("s".into()), "a".into());
        nav.select_tab(Tab::You);
        assert_eq!(nav.route(), Route::You);
        nav.select_tab(Tab::Servers);
        assert_eq!(nav.route(), Route::Chat("s", "a"));
        nav.select_tab(Tab::Servers);
        assert_eq!(nav.route(), Route::Servers);
        assert!(nav.back());
        assert_eq!(nav.route(), Route::Conversation("dm"));
    }

    #[test]
    fn friend_conversation_returns_to_friends_then_messages() {
        let mut nav = CompactNav::default();
        nav.show_friends(true);
        assert_eq!(nav.route(), Route::Friends);
        nav.open_chat(None, "dm".into());
        assert_eq!(nav.route(), Route::Conversation("dm"));
        assert!(nav.back());
        assert_eq!(nav.route(), Route::Friends);
        assert!(nav.back());
        assert_eq!(nav.route(), Route::Messages);
    }

    #[test]
    fn switching_channels_replaces_detail_and_back_restores_parent() {
        let mut nav = CompactNav::default();
        nav.open_server("s".into());
        nav.open_chat(Some("s".into()), "a".into());
        nav.open_chat(Some("s".into()), "b".into());
        assert_eq!(nav.route(), Route::Chat("s", "b"));
        assert!(nav.back());
        assert_eq!(nav.route(), Route::Channels("s"));
        assert!(nav.back());
        assert_eq!(nav.route(), Route::Servers);
        assert!(nav.back());
        assert!(!nav.back());
    }
}
