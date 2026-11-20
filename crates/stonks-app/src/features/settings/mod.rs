//! Authenticated client settings frame and page navigation.
use super::voice::Voice;
use crate::{
    preferences::NativePreferences, runtime::spawn_core, session::Session, shared::images::Images,
};
use account_pages::{
    AccountPage, AccountPageEvent, AccountPageKind, PrivacyChange, PrivacyPreferences,
};
use gpui_kit::{base::FocusTrapElement, prelude::FluentBuilder, *};
use std::rc::Rc;
use stonks_core::{account_settings::AccountSettingsService, preferences::Change};
use stonks_ui::{
    components::{avatar, Button, ButtonVariant, UiCloseButton, UiDialog},
    i18n::t,
    Theme,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Account,
    Profile,
    Sessions,
    Voice,
    Soundboard,
    Appearance,
    Notifications,
    Privacy,
    Keybinds,
    Language,
    Desktop,
}

// The Tauri shell inherits the document’s 1.5 line height, except for its h1.
const SHELL_LINE_HEIGHT: f32 = 1.5;
const DESKTOP_FRAME_RADIUS: f32 = 20.;

const USER_PAGES: &[Page] = &[Page::Account, Page::Profile, Page::Sessions];
const CLIENT_PAGES: &[Page] = &[
    Page::Voice,
    Page::Soundboard,
    Page::Appearance,
    Page::Notifications,
    Page::Privacy,
    Page::Keybinds,
    Page::Language,
    Page::Desktop,
];

impl Page {
    fn account_kind(self) -> Option<AccountPageKind> {
        Some(match self {
            Self::Account => AccountPageKind::Account,
            Self::Profile => AccountPageKind::Profile,
            Self::Sessions => AccountPageKind::Sessions,
            Self::Privacy => AccountPageKind::Privacy,
            _ => return None,
        })
    }

    fn key(self) -> &'static str {
        match self {
            Self::Account => "settings.pages.account",
            Self::Profile => "settings.pages.profile",
            Self::Sessions => "settings.pages.sessions",
            Self::Voice => "settings.pages.voice",
            Self::Soundboard => "settings.pages.soundboard",
            Self::Appearance => "settings.pages.appearance",
            Self::Notifications => "settings.pages.notifications",
            Self::Privacy => "settings.pages.privacy",
            Self::Keybinds => "settings.pages.keybinds",
            Self::Language => "settings.pages.language",
            Self::Desktop => "settings.pages.desktop",
        }
    }
}

type PageSurface = (AnyElement, Option<AnyElement>);
mod account_pages;
mod preference_pages;
pub(crate) mod updater_section;

#[derive(Clone, Copy)]
enum Destination {
    Page(Page),
    Home,
    Close,
    Logout,
}

#[derive(Debug, PartialEq, Eq)]
enum ExitGuard {
    Wait,
    ConfirmDiscard,
    Proceed,
}

fn exit_guard((busy, dirty): (bool, bool)) -> ExitGuard {
    match (busy, dirty) {
        (true, _) => ExitGuard::Wait,
        (false, true) => ExitGuard::ConfirmDiscard,
        (false, false) => ExitGuard::Proceed,
    }
}

pub(super) enum SettingsEvent {
    Close,
    Logout,
}
impl EventEmitter<SettingsEvent> for Settings {}

pub(super) struct Settings {
    session: Entity<Session>,
    images: Entity<Images>,
    voice: Entity<Voice>,
    preferences: Entity<preference_pages::State>,
    _subscriptions: [Subscription; 4],
    account_service: Option<AccountSettingsService>,
    account_connect: Option<Task<()>>,
    account_page: Option<(Entity<AccountPage>, [Subscription; 2])>,
    page: Page,
    compact: bool,
    home: bool,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    pending: Option<Destination>,
    wait: bool,
    content_scroll: ScrollHandle,
    home_bottom_inset: Pixels,
}

impl Settings {
    pub(super) fn new(
        session: Entity<Session>,
        images: Entity<Images>,
        voice: Entity<Voice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let preferences = cx.new(|cx| preference_pages::State::new(window, cx));
        let mut settings = Self {
            _subscriptions: [
                cx.observe(&session, |_, _, cx| cx.notify()),
                cx.observe(&images, |_, _, cx| cx.notify()),
                cx.observe(&voice, |_, _, cx| cx.notify()),
                cx.observe(&preferences, |_, _, cx| cx.notify()),
            ],
            session,
            preferences,
            images,
            voice,
            account_service: None,
            account_connect: None,
            account_page: None,
            page: Page::default(),
            compact: window.viewport_size().width < px(800.),
            home: window.viewport_size().width < px(800.),
            focus,
            dialog_focus: cx.focus_handle(),
            pending: None,
            wait: false,
            content_scroll: ScrollHandle::new(),
            home_bottom_inset: px(0.),
        };
        settings.connect_account(cx);
        settings
    }

    pub(super) fn is_home(&self) -> bool {
        self.home
    }

    pub(super) fn set_home_bottom_inset(&mut self, inset: Pixels, cx: &mut Context<Self>) {
        if self.home_bottom_inset != inset {
            self.home_bottom_inset = inset;
            cx.notify();
        }
    }

    fn connect_account(&mut self, cx: &mut Context<Self>) {
        if self.account_connect.is_some() || self.account_service.is_some() {
            return;
        }
        let Some((policy, context)) = self.session.read(cx).account_context() else {
            return;
        };
        self.account_connect = Some(cx.spawn(async move |this, cx| {
            let service =
                spawn_core(async move { AccountSettingsService::for_context(policy, context) })
                    .await;
            let _ = this.update(cx, |this, cx| {
                this.account_connect = None;
                if service.is_current() {
                    this.account_service = Some(service);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn account_body(
        &mut self,
        kind: AccountPageKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(service) = self
            .account_service
            .clone()
            .filter(|service| service.is_current())
        else {
            self.account_page = None;
            return if self.account_connect.is_some() {
                div().child(t("common.loading")).into_any_element()
            } else {
                div()
                    .child(t("settings.loadFailed"))
                    .child(
                        Button::new("account-connect-retry", Theme::current(cx))
                            .label(t("common.retry"))
                            .on_click(cx.listener(|this, _, _, cx| this.connect_account(cx))),
                    )
                    .into_any_element()
            };
        };
        let (page, _) = self.account_page.get_or_insert_with(|| {
            let page = cx.new(|cx| AccountPage::new(kind, service.clone(), window, cx));
            if matches!(kind, AccountPageKind::Account) {
                let host = cx.entity().downgrade();
                let opening = service.clone();
                let ending = service.clone();
                let open_host = host.clone();
                page.update(cx, |page, _| {
                    page.set_account_actions(
                        Rc::new(move |window, cx| {
                            if opening.is_current() {
                                let _ = open_host.update(cx, |host, cx| {
                                    host.request(Destination::Page(Page::Sessions), window, cx);
                                });
                            }
                        }),
                        Rc::new(move |window, cx| {
                            if ending.is_current() {
                                let _ = host.update(cx, |host, cx| {
                                    // Disable/delete already succeeded; do not ask to discard.
                                    host.transition(Destination::Logout, window, cx);
                                });
                            }
                        }),
                    );
                });
            }
            let observe = cx.observe(&page, |_, _, cx| cx.notify());
            let events = cx.subscribe(&page, move |this, _, event, cx| {
                if !service.is_current() {
                    return;
                }
                this.session.update(cx, |session, cx| match event {
                    AccountPageEvent::ProfileSaved | AccountPageEvent::UsernameChanged => {
                        session.refresh_account_user(service.clone(), cx);
                    }
                    AccountPageEvent::UserUnblocked(user) => session.update_user(user.clone(), cx),
                });
            });
            (page, [observe, events])
        });
        if matches!(kind, AccountPageKind::Privacy) {
            let prefs = &cx.global::<NativePreferences>().saved;
            let prefs = PrivacyPreferences {
                show_notification_content: prefs.show_notification_content,
                send_typing_indicators: prefs.send_typing_indicators,
                display_embeds: prefs.display_embeds,
                warn_external_links: prefs.warn_external_links,
            };
            page.update(cx, |page, _| {
                page.set_privacy_preferences(
                    prefs,
                    Rc::new(|change, window, cx| {
                        let change = match change {
                            PrivacyChange::ShowNotificationContent(value) => {
                                Change::ShowNotificationContent(value)
                            }
                            PrivacyChange::TypingIndicators(value) => {
                                Change::TypingIndicators(value)
                            }
                            PrivacyChange::DisplayEmbeds(value) => Change::DisplayEmbeds(value),
                            PrivacyChange::LinkWarnings(value) => Change::LinkWarnings(value),
                            PrivacyChange::ClearTrustedDomains => Change::ClearTrustedLinkOrigins,
                        };
                        NativePreferences::commit(change, window, cx)
                            .map_err(|error| error.to_string())
                    }),
                )
            });
        }
        page.clone().into_any_element()
    }

    fn page_status(&self, cx: &App) -> (bool, bool) {
        if self.home {
            (false, false)
        } else if self.page == Page::Soundboard {
            let (busy, dirty, _) = self.voice.read(cx).soundboard_settings_status(cx);
            (busy, dirty)
        } else if let Some((page, _)) = &self.account_page {
            let page = page.read(cx);
            (page.is_busy(), page.is_dirty(cx))
        } else {
            (false, false)
        }
    }

    fn focus_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.stay(window, cx);
        } else if !self.home
            && self.page == Page::Soundboard
            && self.voice.read(cx).soundboard_settings_status(cx).2
        {
            self.voice
                .update(cx, |voice, cx| voice.dismiss_soundboard(window, cx));
        } else {
            self.request(self.back_destination(), window, cx);
        }
    }

    fn back_destination(&self) -> Destination {
        if self.compact && !self.home {
            Destination::Home
        } else {
            Destination::Close
        }
    }

    fn request(&mut self, destination: Destination, window: &mut Window, cx: &mut Context<Self>) {
        if !self.home && matches!(destination, Destination::Page(page) if page == self.page) {
            self.focus_frame(window, cx);
            return;
        }
        if self.pending.is_some() {
            return;
        }
        match exit_guard(self.page_status(cx)) {
            ExitGuard::Wait => {
                self.wait = true;
                cx.notify();
            }
            ExitGuard::ConfirmDiscard => {
                self.pending = Some(destination);
                self.dialog_focus.focus(window, cx);
                cx.notify();
            }
            ExitGuard::Proceed => self.transition(destination, window, cx),
        }
    }

    fn stay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn transition(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page_status(cx).0 {
            self.stay(window, cx);
            self.wait = true;
            cx.notify();
            return;
        }
        self.pending = None;
        self.wait = false;
        self.account_page = None;
        if !self.home && self.page == Page::Soundboard {
            self.voice
                .update(cx, |voice, cx| voice.close_soundboard_settings(window, cx));
        }
        match destination {
            Destination::Close => cx.emit(SettingsEvent::Close),
            Destination::Logout => cx.emit(SettingsEvent::Logout),
            Destination::Home => {
                self.home = true;
                self.focus_frame(window, cx);
            }
            Destination::Page(page) => {
                self.home = false;
                self.page = page;
                self.content_scroll = ScrollHandle::new();
                self.focus_frame(window, cx);
                if page == Page::Soundboard {
                    self.voice
                        .update(cx, |voice, cx| voice.open_soundboard_settings(window, cx));
                }
            }
        }
    }

    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> PageSurface {
        if self.page == Page::Soundboard {
            return self
                .voice
                .update(cx, |voice, cx| voice.soundboard_settings_page(cx))
                .unwrap_or_else(|| (div().child(t("common.loading")).into_any_element(), None));
        }
        use preference_pages::Page as Preference;
        let preference = match self.page {
            Page::Voice => Some(Preference::Voice),
            Page::Appearance => Some(Preference::Appearance),
            Page::Notifications => Some(Preference::Notifications),
            Page::Keybinds => Some(Preference::Keybinds),
            Page::Language => Some(Preference::Language),
            Page::Desktop => Some(Preference::Desktop),
            Page::Account | Page::Profile | Page::Sessions | Page::Privacy | Page::Soundboard => {
                None
            }
        };
        if let Some(page) = preference {
            return (
                preference_pages::render(page, &self.preferences, window, cx).into_any_element(),
                None,
            );
        }
        if let Some(kind) = self.page.account_kind() {
            return (self.account_body(kind, window, cx), None);
        }
        (
            div().child(t("settings.loadFailed")).into_any_element(),
            None,
        )
    }

    fn render_navigation(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let user = session
            .snapshot
            .as_ref()
            .map(|snapshot| &snapshot.current_user);
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let source = user
            .and_then(|user| user.avatar.as_ref())
            .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id));
        let name = user
            .map(|user| {
                user.display_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&user.username)
            })
            .unwrap_or_default()
            .to_owned();
        let image = self
            .images
            .update(cx, |images, cx| images.get(source.as_deref(), 80, cx));
        let scroll_index = self.compact && self.home;
        let mut nav = div()
            .id("settings-navigation-scroll")
            .when(scroll_index, |nav| nav.flex_grow(1.).flex_shrink_0())
            .when(!scroll_index, |nav| {
                nav.flex_1().min_h_0().overflow_y_scroll()
            })
            .px(px(4.))
            .pt(px(4.))
            .pb(px(12.));
        for (heading, pages) in [
            ("settings.groups.user", USER_PAGES),
            ("settings.groups.client", CLIENT_PAGES),
        ] {
            let mut group = div()
                .flex()
                .flex_col()
                .gap(px(theme.space1))
                .mb(px(theme.space3))
                .child(
                    div()
                        .px(px(theme.space2))
                        .mb(px(theme.space1))
                        .text_size(px(theme.font_size_xs))
                        .line_height(relative(SHELL_LINE_HEIGHT))
                        .font_weight(FontWeight(850.))
                        .text_color(theme.text_muted)
                        .child(t(heading).to_uppercase()),
                );
            for &page in pages {
                group = group.child(
                    Button::new(page.key(), theme)
                        .variant(ButtonVariant::Nav)
                        .label(t(page.key()))
                        .selected(!self.home && self.page == page)
                        .leading(settings_icon(page.key(), theme.text_muted))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request(Destination::Page(page), window, cx)
                        })),
                );
            }
            nav = nav.child(group);
        }
        // Tauri keeps the nav variant and muted icon, overriding only the logout label.
        let logout_theme = Theme {
            text_muted: theme.danger,
            ..theme
        };
        nav = nav.child(
            div().mt(px(theme.space1)).child(
                Button::new("settings-logout", logout_theme)
                    .variant(ButtonVariant::Nav)
                    .label(t("settings.logout"))
                    .leading(settings_icon("settings.logout", theme.text_muted))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request(Destination::Logout, window, cx)
                    })),
            ),
        );
        div()
            .flex()
            .flex_col()
            .when(scroll_index, |index| index.min_h_full().flex_shrink_0())
            .when(!scroll_index, |index| index.h_full().min_h_0())
            .line_height(relative(SHELL_LINE_HEIGHT))
            .text_color(theme.text)
            .bg(theme.canvas)
            // GPUI's overflow mask is rectangular, so round the sidebar's own paint.
            .when(!self.compact, |nav| {
                nav.rounded_tl(px(DESKTOP_FRAME_RADIUS))
                    .rounded_bl(px(DESKTOP_FRAME_RADIUS))
            })
            .px(px(12.))
            .pt(px(if self.compact { 16. } else { 22. }))
            .pb(if scroll_index {
                px(16.) + self.home_bottom_inset
            } else {
                px(14.)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(11.))
                    .mx(px(7.))
                    .mb(px(13.))
                    .p(px(theme.space2))
                    .flex_shrink_0()
                    .child(avatar(&theme, &name, 40., image))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme.font_size_md))
                                    .font_weight(FontWeight::BOLD)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme.font_size_xs))
                                    .text_color(theme.text_muted)
                                    .child(t("settings.accountSummary")),
                            ),
                    ),
            )
            .child(nav)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap(px(2.))
                    .px(px(theme.space2))
                    .pt(px(theme.space2))
                    .text_size(px(9.))
                    .child(div().font_weight(FontWeight::BOLD).child(client_version()))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(t("settings.version.client")),
                    ),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let viewport = window.viewport_size();
        let compact = viewport.width < px(800.);
        if self.compact != compact {
            self.compact = compact;
        }
        if compact && self.home {
            return div()
                .id("settings-index-scroll")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .bg(theme.canvas)
                .overflow_y_scroll()
                .child(self.render_navigation(theme, cx))
                .into_any_element();
        }
        let (page, overlay) = self.render_page(window, cx);
        let header = div()
            .flex()
            .min_w_0()
            .text_color(theme.text)
            .justify_between()
            .items_start()
            .gap(px(theme.space3))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space1))
                    .when(compact && !self.home, |copy| {
                        copy.items_start().child(
                            Button::new("settings-back", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.settings"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request(Destination::Home, window, cx);
                                })),
                        )
                    })
                    .when(!compact, |copy| {
                        copy.child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .line_height(relative(SHELL_LINE_HEIGHT))
                                .font_weight(FontWeight(750.))
                                .text_color(theme.text_muted)
                                .child(t("common.settings").to_uppercase()),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(27.))
                            .line_height(px(27. * theme.line_height_compact))
                            .font_weight(FontWeight::BOLD)
                            .child(t(self.page.key())),
                    ),
            )
            .child(
                UiCloseButton::new("settings-close", theme).on_click(cx.listener(
                    |this, _, window, cx| this.request(this.back_destination(), window, cx),
                )),
            );
        let wait = self.wait && self.page_status(cx).0;
        let content = div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .px(px(if compact { 16. } else { 36. }))
            .pt(px(if compact { 16. } else { 28. }))
            .pb(px(if compact { 24. } else { 40. }))
            .child(header)
            .when(wait, |content| {
                content.child(
                    div()
                        .text_color(theme.text_muted)
                        .child(t("settings.unsaved.wait")),
                )
            })
            .child(div().mt(px(theme.space6)).child(page));
        let shadow = || {
            vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }]
        };
        let mut frame = div()
            .id("client-settings-frame")
            .relative()
            .flex()
            .overflow_hidden()
            .occlude()
            .w(if compact {
                viewport.width
            } else {
                px(1040.).min(viewport.width - px(36.))
            })
            .h(if compact {
                relative(1.)
            } else {
                px(720.).min((viewport.height - px(36.)).max(px(1.))).into()
            })
            .when(!compact, |frame| {
                frame.rounded(px(DESKTOP_FRAME_RADIUS)).shadow(shadow())
            })
            .bg(theme.surface);
        if !compact {
            frame = frame.child(
                div()
                    .w(px(236.))
                    .h_full()
                    .flex_shrink_0()
                    .child(self.render_navigation(theme, cx)),
            );
        }
        frame = frame.child(content);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("client-settings-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme.backdrop)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.request(Destination::Close, window, cx)
                        }),
                    ),
            )
            .child(if self.pending.is_some() {
                frame.into_any_element()
            } else {
                frame
                    .focus_trap("client-settings-focus", &self.focus)
                    .into_any_element()
            })
            .children(overlay)
            .when_some(self.pending, |stage, destination| {
                stage.child(
                    UiDialog::new("settings-unsaved", theme, t("settings.unsaved.title"))
                        .description(t("settings.unsaved.description"))
                        .focus(&self.dialog_focus)
                        .show_close(false)
                        .on_close(cx.listener(|this, _, window, cx| this.stay(window, cx)))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .mt(px(theme.space4))
                                .gap(px(theme.space2))
                                .child(
                                    Button::new("settings-stay", theme)
                                        .variant(ButtonVariant::Secondary)
                                        .label(t("settings.unsaved.stay"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.stay(window, cx)
                                            }),
                                        ),
                                )
                                .child(
                                    Button::new("settings-discard", theme)
                                        .variant(ButtonVariant::Danger)
                                        .label(t("settings.unsaved.discard"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.transition(destination, window, cx)
                                        })),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}

fn client_version() -> &'static str {
    concat!("Stonks Chat ", env!("CARGO_PKG_VERSION"))
}

// Filled 24-unit paths from the Tauri SettingsIcon registry, rendered at 18px.
fn settings_icon(key: &'static str, tint: Rgba) -> impl IntoElement {
    use std::{cell::RefCell, collections::HashMap, sync::Arc};
    type Cache = HashMap<(&'static str, [u8; 3]), Arc<Image>>;
    thread_local! { static CACHE: RefCell<Cache> = RefCell::default(); }
    let rgb = [tint.r, tint.g, tint.b].map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
    let image = CACHE.with(|cache| cache.borrow_mut().entry((key, rgb)).or_insert_with(|| {
        let path = match key {
            "settings.pages.account" => "M12 2a5 5 0 1 0 0 10 5 5 0 0 0 0-10Zm0 12c-5 0-9 2.5-9 5.5V22h18v-2.5c0-3-4-5.5-9-5.5Zm0-10a3 3 0 1 1 0 6 3 3 0 0 1 0-6Zm0 12c3.8 0 7 1.7 7 3.5v.5H5v-.5C5 17.7 8.2 16 12 16Z",
            "settings.pages.profile" => "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8Zm0 2c-4.4 0-8 2.2-8 5v1h16v-1c0-2.8-3.6-5-8-5Z",
            "settings.pages.sessions" => "M12 2 4.5 5v5.5c0 4.8 3.1 9.1 7.5 10.5 4.4-1.4 7.5-5.7 7.5-10.5V5L12 2Zm0 3 4.8 1.9v3.6c0 3.3-1.9 6.4-4.8 7.7-2.9-1.3-4.8-4.4-4.8-7.7V6.9L12 5Z",
            "settings.pages.voice" => "M12 15a4 4 0 0 0 4-4V6a4 4 0 1 0-8 0v5a4 4 0 0 0 4 4Zm-2-9a2 2 0 1 1 4 0v5a2 2 0 1 1-4 0V6Zm-5 5h2a5 5 0 0 0 10 0h2a7 7 0 0 1-6 6.9V21h3v2H8v-2h3v-3.1A7 7 0 0 1 5 11Z",
            "settings.pages.soundboard" => "M15 3v12.3A3.5 3.5 0 1 1 13 12V6.5l8-2V3l-6 1.5V3Zm-5.5 11A1.5 1.5 0 1 0 11 15.5V14H9.5Zm8 2A1.5 1.5 0 1 0 19 17.5V16h-1.5Z",
            "settings.pages.appearance" => "M12 3a9 9 0 0 0 0 18h1.4a2.1 2.1 0 0 0 1.4-3.7 1.3 1.3 0 0 1 .9-2.3H18a3 3 0 0 0 3-3c0-5-4-9-9-9Zm-5 9a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3Zm3-4a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3Zm4 0a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3Zm3 4a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3Z",
            "settings.pages.notifications" => "M12 2a3 3 0 0 0-3 3v.3A6 6 0 0 0 6 10.5V16l-2 2v1h16v-1l-2-2v-5.5a6 6 0 0 0-3-5.2V5a3 3 0 0 0-3-3Zm0 2a1 1 0 0 1 1 1v1.8l.7.2a4 4 0 0 1 2.3 3.5V17H8v-6.5A4 4 0 0 1 10.3 7l.7-.2V5a1 1 0 0 1 1-1Zm-2 16h4a2 2 0 0 1-4 0Z",
            "settings.pages.privacy" => "M12 2 4.5 5v5.5c0 4.8 3.1 9.1 7.5 10.5 4.4-1.4 7.5-5.7 7.5-10.5V5L12 2Zm0 3 4.8 1.9v3.6c0 3.3-1.9 6.4-4.8 7.7-2.9-1.3-4.8-4.4-4.8-7.7V6.9L12 5Z",
            "settings.pages.keybinds" => "M4 5h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2Zm0 2v10h16V7H4Zm2 2h2v2H6V9Zm3 0h2v2H9V9Zm3 0h2v2h-2V9Zm3 0h3v2h-3V9ZM6 13h3v2H6v-2Zm4 0h8v2h-8v-2Z",
            "settings.pages.language" => "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Zm6.9 9h-3a15 15 0 0 0-1.4-6.2A8 8 0 0 1 18.9 11ZM12 4c.8 1 1.7 3.4 1.9 7h-3.8C10.3 7.4 11.2 5 12 4Zm-2.5.8A15 15 0 0 0 8.1 11h-3a8 8 0 0 1 4.4-6.2ZM5.1 13h3a15 15 0 0 0 1.4 6.2A8 8 0 0 1 5.1 13Zm6.9 7c-.8-1-1.7-3.4-1.9-7h3.8c-.2 3.6-1.1 6-1.9 7Zm2.5-.8a15 15 0 0 0 1.4-6.2h3a8 8 0 0 1-4.4 6.2Z",
            "settings.pages.desktop" => "M3 4h18a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-7v2h3v2H7v-2h3v-2H3a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Zm0 2v11h18V6H3Z",
            "settings.logout" => "M10 3H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h6v-2H4V5h6V3Zm5.6 4.6L14.2 9l2 2H8v2h8.2l-2 2 1.4 1.4L20 12l-4.4-4.4Z",
            _ => "",
        };
        let [r, g, b] = rgb;
        let source = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#{r:02x}{g:02x}{b:02x}" d="{path}"/></svg>"##);
        Arc::new(Image::from_bytes(ImageFormat::Svg, source.into_bytes()))
    }).clone());
    img(image).w(px(18.)).h(px(18.)).flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use super::{exit_guard, ExitGuard};

    #[test]
    fn pending_work_takes_precedence_over_discard_for_all_destinations() {
        assert_eq!(exit_guard((true, true)), ExitGuard::Wait);
        assert_eq!(exit_guard((true, false)), ExitGuard::Wait);
        assert_eq!(exit_guard((false, true)), ExitGuard::ConfirmDiscard);
        assert_eq!(exit_guard((false, false)), ExitGuard::Proceed);
    }
}
