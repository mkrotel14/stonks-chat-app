//! Root view: startup splash and routing between auth, server selection,
//! and the authenticated shell, with shared window chrome.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::*;
use stonks_core::http::HttpPolicy;
use stonks_core::server::CONFIG_FILE;
use stonks_core::session::SessionStore;
use stonks_ui::components::{draggable_chrome, window_controls, Button, ButtonVariant};
use stonks_ui::i18n::t;
use stonks_ui::icons::Icon;
use stonks_ui::{fonts, Theme};

use super::auth::{AuthScreen, AuthScreenEvent};
use super::conversation::timeline::Timeline;
use super::server_selection::{ServerPicker, ServerPickerEvent};
use super::shell::ShellView;
use crate::desktop::PendingDeepLink;
use crate::session::Session;
use crate::shared::images::Images;

#[path = "app_updater.rs"]
pub(crate) mod updater;

static BRAND_PNG: &[u8] = include_bytes!("../../launcher/com.rdxlabs.stonks.gpui.png");

pub fn shared_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = crate::desktop::config_home().expect("roaming AppData directory is available");
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME is set")).join(".config")
        });
    base.join("com.rdxlabs.stonks").join(CONFIG_FILE)
}

enum Screen {
    Startup(StartupPhase),
    Auth,
    ServerPicker,
    Shell,
}

enum StartupPhase {
    CheckingRelease,
    Restoring,
}

pub struct AppView {
    screen: Screen,
    splash_brand: Arc<Image>,
    auth: Entity<AuthScreen>,
    picker: Entity<ServerPicker>,
    session: Entity<Session>,
    shell: Entity<ShellView>,
    _subscriptions: Vec<Subscription>,
    _appearance: Subscription,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config_path = shared_config_path();
        apply_theme(window, cx);
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            apply_theme(window, cx);
            cx.refresh_windows();
        });
        let server = stonks_core::server::read_selection(&config_path).unwrap_or(None);
        let policy = Arc::new(
            HttpPolicy::new(Arc::new(SessionStore::new(config_path.clone())))
                .expect("shared native HTTP client"),
        );

        crate::shared::file_picker::install(window, cx);
        let auth = cx.new(|cx| AuthScreen::new(window, cx, server, policy.clone()));
        let picker = cx.new(|cx| ServerPicker::new(window, cx, config_path, None));
        let session = cx.new(|_cx| Session::new(policy.clone()));
        let images = cx.new(|cx| Images::new(policy.clone(), &session, cx));
        let composer =
            cx.new(|cx| super::conversation::composer::Composer::new(policy.clone(), window, cx));
        composer.update(cx, |composer, cx| {
            composer.set_attachment_route(
                super::conversation::composer::core_attachment_route(policy.clone()),
                cx,
            );
        });
        let timeline =
            cx.new(|cx| Timeline::new(session.clone(), policy.clone(), images.clone(), window, cx));
        let shell = cx.new(|cx| {
            ShellView::new(
                session.clone(),
                timeline,
                images,
                composer,
                policy,
                window,
                cx,
            )
        });

        let subscriptions = vec![
            cx.observe_global_in::<PendingDeepLink>(window, Self::route_pending_link),
            cx.observe_in(&session, window, |this, _, window, cx| {
                this.route_pending_link(window, cx);
            }),
            cx.subscribe(&shell, |this, _, _: &super::shell::SignOut, cx| {
                this.shell
                    .update(cx, |shell, cx| shell.suspend_voice(true, cx));
                this.auth.update(cx, |auth, cx| auth.logout(cx))
            }),
            cx.subscribe_in(&auth, window, |this, _, event, window, cx| match event {
                AuthScreenEvent::OpenServerPicker => {
                    let server = this.auth.read(cx).server.clone();
                    this.picker.update(cx, |picker, cx| {
                        picker.reset(server.as_ref(), window, cx);
                    });
                    this.screen = Screen::ServerPicker;
                    cx.notify();
                }
                AuthScreenEvent::Authenticated => {
                    this.session.update(cx, |session, cx| session.connect(cx));
                    this.screen = Screen::Shell;
                    this.shell.update(cx, |shell, cx| {
                        shell.suspend_voice(false, cx);
                        shell.focus_composer(window, cx);
                    });
                    cx.notify();
                }
                AuthScreenEvent::RestoreNeedsAuth => {
                    if matches!(this.screen, Screen::Startup(_)) {
                        this.screen = Screen::Auth;
                        cx.notify();
                    }
                }
                AuthScreenEvent::SignedOut => {
                    // Do not carry an outgoing account's pending route into another login.
                    PendingDeepLink::take(cx);
                    this.shell.update(cx, |shell, cx| {
                        shell.clear_account_settings(cx);
                        shell.suspend_voice(true, cx);
                    });
                    this.session.update(cx, |session, cx| session.stop(cx));
                    this.screen = Screen::Auth;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&picker, window, |this, _, event, _, cx| match event {
                ServerPickerEvent::Selected(selection) => {
                    this.auth
                        .update(cx, |auth, cx| auth.set_server(selection.clone(), cx));
                    this.screen = Screen::Auth;
                    cx.notify();
                }
                ServerPickerEvent::Cancelled => {
                    this.screen = Screen::Auth;
                    cx.notify();
                }
            }),
        ];

        // Share the startup check with Desktop settings, without blocking auth
        // on the public release service.
        updater::Updater::start(cx);
        let app = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                let _ = app.update(cx, |this, cx| {
                    if matches!(this.screen, Screen::Startup(StartupPhase::CheckingRelease)) {
                        this.screen = Screen::Startup(StartupPhase::Restoring);
                        this.auth.update(cx, |auth, cx| auth.restore_session(cx));
                        cx.notify();
                    }
                });
            });
        });

        Self {
            screen: Screen::Startup(StartupPhase::CheckingRelease),
            splash_brand: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../launcher/com.rdxlabs.stonks.gpui.png").to_vec(),
            )),
            auth,
            picker,
            session,
            shell,
            _subscriptions: subscriptions,
            _appearance: appearance,
        }
    }

    fn route_pending_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Shell) || !self.session.read(cx).can_route_external() {
            return;
        }
        let Some(link) = PendingDeepLink::take(cx) else {
            return;
        };
        let result = match link.navigation_path() {
            Some(path) => self.shell.update(cx, |shell, cx| {
                shell
                    .navigate_external_route(&path, window, cx)
                    .and_then(|handled| handled.then_some(()).ok_or("This link is not supported."))
            }),
            None => Err("Joining from an invite is not supported in this client."),
        };
        if let Err(message) = result {
            let _ = window.prompt(
                PromptLevel::Warning,
                message,
                None,
                &[PromptButton::cancel(t("common.close"))],
                cx,
            );
        }
    }

    fn startup_splash(&self, theme: &Theme, window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(theme.space6))
            .bg(theme.surface)
            .text_color(theme.text)
            .font_family(fonts::FAMILY)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(theme.space3))
                    .max_w(px(480.))
                    .text_center()
                    .child(img(self.splash_brand.clone()).size(px(72.)))
                    .child(
                        div()
                            .text_size(px(theme.font_size_xl))
                            .font_weight(FontWeight::BOLD)
                            .child("Starting Stonks Chat"),
                    )
                    .child(
                        div()
                            .text_size(px(theme.font_size_md))
                            .text_color(theme.text_muted)
                            .child(t(match self.screen {
                                Screen::Startup(StartupPhase::CheckingRelease) => {
                                    "settings.desktop.checking"
                                }
                                _ => "auth.restoring",
                            })),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .flex()
                    .child(draggable_chrome(
                        div().id("startup-drag-region").flex_1(),
                        window,
                        cx,
                    ))
                    .child(window_controls(theme, window)),
            )
    }

    fn server_pill(&self, theme: &Theme, cx: &App) -> impl IntoElement {
        let server = &self.auth.read(cx).server;
        let (label, connected) = match server {
            Some(server) => (server.display_name.clone(), true),
            None => (t("auth.notConnected"), false),
        };
        div()
            .flex()
            .items_center()
            .gap(px(theme.space2))
            .py(px(theme.space2))
            .px(px(theme.space3))
            .min_w(px(0.))
            .rounded(px(theme.radius_pill))
            .bg(theme.surface)
            .text_size(px(theme.font_size_sm))
            .text_color(if connected {
                theme.text_muted
            } else {
                theme.danger
            })
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(8.))
                    .rounded(px(theme.radius_pill))
                    .bg(if connected {
                        theme.success
                    } else {
                        theme.danger
                    }),
            )
            .child(div().min_w(px(0.)).text_ellipsis().child(label))
    }
}

fn apply_theme(window: &mut Window, cx: &mut App) {
    use stonks_core::preferences::ThemePreference;
    let saved = &cx.global::<crate::preferences::NativePreferences>().saved;
    let light = match saved.theme {
        ThemePreference::Light => true,
        ThemePreference::Dark => false,
        ThemePreference::System => matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    };
    let theme =
        if light { Theme::light() } else { Theme::dark() }.with_accent(saved.accent.as_str());
    cx.set_global(stonks_ui::theme::CurrentTheme(theme));
    gpui_kit::component::theme::Theme::change(
        if light {
            gpui_kit::component::theme::ThemeMode::Light
        } else {
            gpui_kit::component::theme::ThemeMode::Dark
        },
        Some(window),
        cx,
    );
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        if matches!(self.screen, Screen::Startup(_)) {
            return self.startup_splash(&theme, window, cx).into_any_element();
        }
        if matches!(self.screen, Screen::Shell) {
            // The shell owns the whole window; its pane header carries the
            // window chrome (drag region + min/max/close).
            return div()
                .size_full()
                .child(self.shell.clone())
                .into_any_element();
        }
        let content: AnyElement = match self.screen {
            Screen::Auth => self.auth.clone().into_any_element(),
            Screen::ServerPicker => self.picker.clone().into_any_element(),
            Screen::Startup(_) | Screen::Shell => unreachable!("handled above"),
        };

        div()
            .size_full()
            .bg(theme.canvas)
            .text_color(theme.text)
            .font_family(fonts::FAMILY)
            .child(
                // Header (Auth.stylex.ts: fixed 72px, surface-raised) doubling
                // as the window titlebar under client-side decorations.
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(72.))
                    .bg(theme.surface_raised)
                    .flex()
                    .items_center()
                    .px(px(22.))
                    .child(draggable_chrome(
                        div()
                            .id("chrome-brand")
                            .flex()
                            .gap(px(11.))
                            .items_center()
                            .child(
                                img(Arc::new(Image::from_bytes(
                                    ImageFormat::Png,
                                    BRAND_PNG.to_vec(),
                                )))
                                .w(px(34.))
                                .h(px(34.)),
                            )
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Stonks Chat"),
                            ),
                        window,
                        cx,
                    ))
                    .child(draggable_chrome(
                        div().id("chrome-spacer").flex_1(),
                        window,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme.space2))
                            .child(self.server_pill(&theme, cx))
                            .child({
                                // Narrow windows (tiling compositors tile well
                                // below our default width): icon-only, so the
                                // window controls never overflow.
                                let button = Button::new("server-settings", theme)
                                    .variant(ButtonVariant::Quiet)
                                    .leading(Icon::Settings.render(theme.text_muted, 18.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.auth.update(cx, |auth, cx| {
                                            auth.open_server_picker(cx);
                                        });
                                    }));
                                if window.viewport_size().width >= px(640.) {
                                    button.label(t("auth.serverSettings"))
                                } else {
                                    button
                                }
                            })
                            .child(window_controls(&theme, window)),
                    ),
            )
            .child(
                div()
                    .size_full()
                    .pt(px(94.))
                    .pb(px(36.))
                    .px(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .into_any_element()
    }
}
