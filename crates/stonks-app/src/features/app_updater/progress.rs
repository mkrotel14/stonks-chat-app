//! Branded status window for a user-confirmed update. The updater owns the install.

use std::sync::Arc;

use gpui_kit::{prelude::FluentBuilder, *};
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    i18n::t,
    theme::CurrentTheme,
    Theme,
};

use super::{Status, Updater};

const TITLE: &str = "Updating Stonks Chat";
const BRAND: &[u8] = include_bytes!("../../../launcher/com.rdxlabs.stonks.gpui.png");

struct ProgressWindow(AnyWindowHandle);
impl Global for ProgressWindow {}

/// Call from the manual install path before starting the attempt. The updater
/// must notify observers when its phase or byte counts change.
pub(super) fn open(window: &mut Window, cx: &mut App) {
    let Some(updater) = Updater::shared(cx) else {
        return;
    };
    let opener = window.window_handle();
    let display_id = window.display(cx).map(|display| display.id());
    // GPUI draws a new window synchronously. Defer until the parent has
    // returned the updater entity to the app and published the initial phase.
    cx.defer(move |cx| {
        if let Some(handle) = cx.try_global::<ProgressWindow>().map(|slot| slot.0) {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
        }
        let bounds = Bounds::centered(display_id, size(px(400.), px(320.)), cx);
        match gpui_kit::open_window(
            WindowOptions {
                app_id: Some("stonks-chat-gpui".into()),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id,
                titlebar: None,
                window_decorations: Some(WindowDecorations::Client),
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Progress::new(updater, opener, window, cx)),
        ) {
            Ok((handle, _)) => cx.set_global(ProgressWindow(handle)),
            Err(error) => eprintln!("Could not open the update progress window: {error:#}"),
        }
    });
}

#[derive(Clone, Copy)]
enum Phase<'a> {
    Preparing,
    Downloading,
    Verifying,
    Installing,
    #[cfg(target_os = "linux")]
    Restarting,
    #[cfg(target_os = "windows")]
    InstallerLaunched,
    Failed(&'a str),
}

impl<'a> Phase<'a> {
    fn from_status(status: &'a Status) -> Self {
        match status {
            Status::Downloading => Self::Downloading,
            Status::Verifying => Self::Verifying,
            Status::Installing => Self::Installing,
            #[cfg(target_os = "linux")]
            Status::Restarting => Self::Restarting,
            #[cfg(target_os = "windows")]
            Status::InstallerLaunched => Self::InstallerLaunched,
            Status::InstallFailed(error) => Self::Failed(error),
            _ => Self::Preparing,
        }
    }

    fn busy(self) -> bool {
        #[cfg(target_os = "windows")]
        if matches!(self, Self::InstallerLaunched) {
            return false;
        }
        !matches!(self, Self::Failed(_))
    }

    fn download(self, bytes: Option<(u64, u64)>) -> Option<Download> {
        match (self, bytes) {
            (Self::Downloading, Some((received, total))) if total > 0 => Some(Download {
                received: received.min(total),
                total,
            }),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Self::Restarting => "Restarting Stonks Chat",
            #[cfg(target_os = "windows")]
            Self::InstallerLaunched => "Stonks Chat installer opened",
            Self::Failed(_) => "Could not update Stonks Chat",
            _ => TITLE,
        }
    }

    fn message(self) -> String {
        match self {
            Self::Preparing => t("settings.desktop.preparing"),
            Self::Downloading => "Downloading the update…".into(),
            Self::Verifying => "Verifying the download…".into(),
            Self::Installing => t("settings.desktop.archInstalling"),
            #[cfg(target_os = "linux")]
            Self::Restarting => "The update is installed. Stonks Chat will reopen shortly.".into(),
            #[cfg(target_os = "windows")]
            Self::InstallerLaunched => {
                "Follow the installer’s instructions to finish the update.".into()
            }
            Self::Failed(error) if !error.trim().is_empty() => error.to_owned(),
            Self::Failed(_) => "The update could not be installed.".into(),
        }
    }
}

#[derive(Clone, Copy)]
struct Download {
    received: u64,
    total: u64,
}

impl Download {
    fn ratio(self) -> f64 {
        self.received as f64 / self.total as f64
    }

    fn label(self) -> String {
        const MIB: f64 = 1024. * 1024.;
        format!(
            "{:.0}% downloaded · {:.1} of {:.1} MiB",
            self.ratio() * 100.,
            self.received as f64 / MIB,
            self.total as f64 / MIB,
        )
    }
}

struct Progress {
    updater: Entity<Updater>,
    brand: Arc<Image>,
    focus: FocusHandle,
    _status: Subscription,
    _theme: Subscription,
}

impl Progress {
    fn new(
        updater: Entity<Updater>,
        opener: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title(TITLE);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let close_updater = updater.clone();
        window.on_window_should_close(cx, move |_, cx| Self::allow_close(&close_updater, cx));

        let handle = window.window_handle();
        cx.on_release(move |_, cx| {
            if cx
                .try_global::<ProgressWindow>()
                .is_some_and(|slot| slot.0 == handle)
            {
                cx.remove_global::<ProgressWindow>();
                cx.defer(move |cx| {
                    let _ = opener.update(cx, |_, window, _| window.activate_window());
                });
            }
        })
        .detach();

        Self {
            _status: cx.observe(&updater, |_, _, cx| cx.notify()),
            _theme: cx.observe_global::<CurrentTheme>(|_, cx| cx.notify()),
            updater,
            brand: Arc::new(Image::from_bytes(ImageFormat::Png, BRAND.to_vec())),
            focus,
        }
    }

    fn allow_close(updater: &Entity<Updater>, cx: &mut App) -> bool {
        let phase = Phase::from_status(&updater.read(cx).status);
        if phase.busy() {
            return false;
        }
        if matches!(phase, Phase::Failed(_)) {
            updater.update(cx, |updater, cx| updater.dismiss_install_failure(cx));
        }
        true
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::allow_close(&self.updater, cx) {
            window.remove_window();
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        // A stale click must not retry after a new install has already begun.
        self.updater.update(cx, |updater, cx| {
            if matches!(updater.status, Status::InstallFailed(_)) {
                updater.retry_install(cx);
            }
        });
    }
}

impl Render for Progress {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let updater = self.updater.read(cx);
        let phase = Phase::from_status(&updater.status);
        let busy = phase.busy();
        let failed = matches!(phase, Phase::Failed(_));
        let download = phase.download(updater.progress);
        let message = phase.message();
        let title = phase.title();

        let detail = div()
            .id("update-status-message")
            .role(if failed { Role::Alert } else { Role::Status })
            .aria_label(message.clone())
            .w_full()
            .text_size(px(theme.font_size_md))
            .line_height(relative(theme.line_height_normal))
            .text_color(if failed {
                theme.danger
            } else {
                theme.text_muted
            })
            .when(failed, |detail| {
                detail
                    .max_h(px(84.))
                    .overflow_y_scroll()
                    .tab_index(0)
                    .rounded(px(theme.radius_sm))
                    .focus_visible(move |style| style.bg(theme.message_hover))
            })
            .child(message);

        div()
            .id("manual-update-progress")
            .role(Role::Dialog)
            .aria_label(title)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
            }))
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
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(theme.space3))
                    .text_center()
                    .child(img(self.brand.clone()).size(px(64.)).flex_shrink_0())
                    .child(
                        div()
                            .text_size(px(theme.font_size_xl))
                            .line_height(relative(theme.line_height_compact))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .child(detail)
                    .when_some(download, |card, download| {
                        card.child(
                            div()
                                .text_size(px(theme.font_size_sm))
                                .text_color(theme.text_muted)
                                .child(download.label()),
                        )
                    })
                    .when(!busy, |card| {
                        card.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_center()
                                .gap(px(theme.space3))
                                .when(failed, |actions| {
                                    actions.child(
                                        Button::new("update-retry", theme)
                                            .label("Retry")
                                            .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                                    )
                                })
                                .child(
                                    Button::new("update-continue", theme)
                                        .variant(ButtonVariant::Secondary)
                                        .label(if failed {
                                            "Continue to app".into()
                                        } else {
                                            t("common.close")
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close(window, cx)
                                        })),
                                ),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Phase, Status};

    #[test]
    fn active_phases_block_dismissal_and_terminal_phases_allow_it() {
        for status in [
            Status::Downloading,
            Status::Verifying,
            Status::Installing,
            #[cfg(target_os = "linux")]
            Status::Restarting,
        ] {
            assert!(Phase::from_status(&status).busy());
        }
        for status in [
            #[cfg(target_os = "windows")]
            Status::InstallerLaunched,
            Status::InstallFailed("test error".into()),
        ] {
            assert!(!Phase::from_status(&status).busy());
        }
    }

    #[test]
    fn byte_counts_are_only_used_for_downloads_with_a_known_total() {
        assert!(Phase::Downloading.download(None).is_none());
        assert!(Phase::Downloading.download(Some((42, 0))).is_none());
        assert!(Phase::Verifying.download(Some((1, 2))).is_none());
        let mib = 1024 * 1024;
        let measured = Phase::Downloading.download(Some((mib, 2 * mib))).unwrap();
        assert_eq!(measured.label(), "50% downloaded · 1.0 of 2.0 MiB");
        let overrun = Phase::Downloading.download(Some((u64::MAX, 2))).unwrap();
        assert_eq!(overrun.ratio(), 1.);
    }
}
