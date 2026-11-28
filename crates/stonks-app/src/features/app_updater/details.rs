//! The shared release notes and install consent surface for the shell and Desktop Settings.

use super::{Status, Updater};
use gpui_kit::*;
use stonks_ui::{
    components::{Button, ButtonVariant, Feedback, FeedbackTone, UiDialog},
    i18n::{t, t_vars},
    Theme,
};

gpui_kit::actions!(app_updater, [ShowUpdateDetails, ConfirmUpdate]);

pub(crate) fn show(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(ShowUpdateDetails), cx);
}

pub(crate) fn confirm(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(ConfirmUpdate), cx);
}

pub(crate) struct Release<'a> {
    pub(crate) version: &'a str,
    asset: &'a str,
    digest: &'a str,
    name: Option<&'a str>,
    pub(crate) notes: Option<&'a str>,
    confirmation: &'static str,
}

impl<'a> Release<'a> {
    pub(crate) fn available(status: &'a Status) -> Option<Self> {
        #[cfg(all(
            target_arch = "x86_64",
            any(target_os = "linux", target_os = "windows")
        ))]
        if let Status::Available(candidate) = status {
            use super::managed::Kind;
            return Some(Self {
                version: candidate.version(),
                asset: candidate.asset_name(),
                digest: candidate.asset_digest(),
                name: nonblank(candidate.release_name.as_deref()),
                notes: nonblank(candidate.release_notes.as_deref()),
                confirmation: match candidate.kind() {
                    Kind::Arch => "settings.desktop.archConfirmDetail",
                    Kind::Rpm => "settings.desktop.rpmConfirmDetail",
                    Kind::Windows => "settings.desktop.windowsConfirmDetail",
                },
            });
        }
        let _ = status;
        None
    }

    pub(crate) fn name_or_asset(&self) -> &'a str {
        self.name.unwrap_or(self.asset)
    }
}

#[cfg(any(
    test,
    all(
        target_arch = "x86_64",
        any(target_os = "linux", target_os = "windows")
    )
))]
fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Notes,
    Confirm,
    Installing,
}

pub(crate) struct Details {
    updater: Entity<Updater>,
    version: String,
    asset: String,
    digest: String,
    stage: Option<Stage>,
    focus: FocusHandle,
    opener: Option<FocusHandle>,
    notes_scroll: ScrollHandle,
    _observe: Subscription,
}

impl Details {
    pub(crate) fn open(confirm: bool, window: &mut Window, cx: &mut App) -> Option<Entity<Self>> {
        let updater = Updater::shared(cx)?;
        let release = Release::available(&updater.read(cx).status)?;
        let (version, asset, digest) = (
            release.version.to_owned(),
            release.asset.to_owned(),
            release.digest.to_owned(),
        );
        let opener = window.focused(cx);
        Some(cx.new(|cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            let observe =
                cx.observe_in(&updater, window, |this: &mut Self, updater, window, cx| {
                    if matches!(this.stage, Some(Stage::Notes | Stage::Confirm))
                        && this.release(&updater.read(cx).status).is_none()
                        || this.stage == Some(Stage::Installing)
                            && Release::available(&updater.read(cx).status).is_some()
                    {
                        this.close(window, cx);
                    } else {
                        cx.notify();
                    }
                });
            Self {
                updater,
                version,
                asset,
                digest,
                stage: Some(if confirm {
                    Stage::Confirm
                } else {
                    Stage::Notes
                }),
                focus,
                opener,
                notes_scroll: ScrollHandle::new(),
                _observe: observe,
            }
        }))
    }

    pub(crate) fn is_open(&self) -> bool {
        self.stage.is_some()
    }

    fn release<'a>(&self, status: &'a Status) -> Option<Release<'a>> {
        Release::available(status).filter(|release| {
            release.version == self.version
                && release.asset == self.asset
                && release.digest == self.digest
        })
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = None;
        if let Some(opener) = self.opener.take() {
            opener.focus(window, cx);
        }
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn set_stage(&mut self, stage: Stage, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = Some(stage);
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn install_on_startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(
            target_arch = "x86_64",
            any(target_os = "linux", target_os = "windows")
        ))]
        {
            use super::managed::Kind;
            let automatic = matches!(
                &self.updater.read(cx).status,
                Status::Available(candidate) if matches!(candidate.kind(), Kind::Rpm | Kind::Windows)
            );
            if automatic {
                self.install(window, cx);
            }
        }
        let _ = (window, cx);
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(
            target_arch = "x86_64",
            any(target_os = "linux", target_os = "windows")
        ))]
        if matches!(self.stage, Some(Stage::Notes | Stage::Confirm)) {
            let updater = self.updater.clone();
            let started = updater.update(cx, |updater, cx| {
                // A new check (or consent in another window) may have invalidated this offer.
                if self.release(&updater.status).is_none() {
                    return false;
                }
                updater.begin_install(window, cx);
                true
            });
            if started {
                self.set_stage(Stage::Installing, window, cx);
            } else {
                self.close(window, cx);
            }
        }
        let _ = (window, cx);
    }
}

const AVAILABLE: &str = "Update available";
const CHANGELOG: &str = "What’s new";

fn actions(theme: Theme) -> Div {
    div().flex().flex_wrap().justify_end().gap(px(theme.space2))
}

pub(crate) fn notes(
    theme: Theme,
    id: &'static str,
    text: &str,
    max_height: Pixels,
) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .max_h(max_height)
        .overflow_y_scroll()
        .text_size(px(theme.font_size_sm))
        .text_color(theme.text_muted)
        .line_height(relative(theme.line_height_normal))
        .child(text.to_owned())
}

impl Details {
    fn dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<UiDialog> {
        let theme = Theme::current(cx);
        let updater = self.updater.clone();
        let status = &updater.read(cx).status;
        let stage = self.stage?;
        let dismiss = || cx.listener(|this, _: &ClickEvent, window, cx| this.dismiss(window, cx));
        let dialog = |id, title: String| {
            UiDialog::new(id, theme, title)
                .width(520.)
                .viewport_inset(16.)
                .focus(&self.focus)
                .on_close(dismiss())
        };
        Some(match stage {
            Stage::Notes => {
                let release = self.release(status)?;
                let scroll = self.notes_scroll.clone();
                let scroll_keys = scroll.clone();
                dialog("update-details", AVAILABLE.into())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(theme.space1))
                            .child(
                                div()
                                    .text_size(px(theme.font_size_lg))
                                    .font_weight(FontWeight(800.))
                                    .text_color(theme.success_text)
                                    .child(format!("Stonks Chat {}", release.version)),
                            )
                            .child(
                                div()
                                    .text_size(px(theme.font_size_sm))
                                    .text_color(theme.text_muted)
                                    .child(release.name_or_asset().to_owned()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(theme.space2))
                            .p(px(theme.space4))
                            .rounded(px(theme.radius_md))
                            .bg(theme.surface_interactive)
                            .child(div().font_weight(FontWeight::BOLD).child(CHANGELOG))
                            .child(
                                notes(
                                    theme,
                                    "update-release-notes",
                                    release.notes.unwrap_or("No release notes were provided."),
                                    ((window.viewport_size().height * 0.44).min(px(360.))
                                        - px(theme.space4 * 2.
                                            + theme.space2
                                            + theme.font_size_md * theme.line_height_normal))
                                    .max(px(36.)),
                                )
                                .aria_label(CHANGELOG)
                                .track_scroll(&scroll)
                                .tab_index(0)
                                .focus_visible(move |style| {
                                    style.shadow(vec![BoxShadow {
                                        color: theme.focus_ring.into(),
                                        offset: point(px(0.), px(0.)),
                                        blur_radius: px(0.),
                                        spread_radius: px(2.),
                                        inset: true,
                                    }])
                                })
                                .on_key_down(
                                    move |event, window, cx| {
                                        scroll_on_key(&scroll_keys, event, window, cx);
                                    },
                                ),
                            ),
                    )
                    .child(
                        actions(theme)
                            .child(
                                Button::new("update-later", theme)
                                    .variant(ButtonVariant::Secondary)
                                    .label("Later")
                                    .on_click(dismiss()),
                            )
                            .child(
                                Button::new("update-next", theme)
                                    .label(t("settings.desktop.installUpdate"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.install(window, cx);
                                    })),
                            ),
                    )
            }
            Stage::Confirm => {
                let release = self.release(status)?;
                dialog("update-confirm", t("settings.desktop.archConfirm"))
                    .description(t(release.confirmation))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(theme.space1))
                            .child(format!("Stonks Chat {}", release.version))
                            .child(div().text_color(theme.text_muted).child(self.asset.clone())),
                    )
                    .child(
                        actions(theme)
                            .child(
                                Button::new("update-cancel", theme)
                                    .variant(ButtonVariant::Secondary)
                                    .label(t("common.cancel"))
                                    .on_click(dismiss()),
                            )
                            .child(
                                Button::new("update-install-confirmed", theme)
                                    .label(t("settings.desktop.installUpdate"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.install(window, cx);
                                    })),
                            ),
                    )
            }
            Stage::Installing => {
                let (key, error) = match status {
                    Status::Downloading => ("settings.desktop.archDownloading", None),
                    Status::Installing => ("settings.desktop.archInstalling", None),
                    Status::Verifying => ("settings.desktop.preparing", None),
                    _ if status.handing_off() => ("settings.desktop.preparing", None),
                    Status::InstallFailed(error) => ("settings.desktop.archFailed", Some(error)),
                    _ => ("settings.desktop.preparing", None),
                };
                let busy = status.busy();
                let mut dialog =
                    dialog("update-progress", format!("Stonks Chat {}", self.version)).busy(busy);
                if let Some(error) = error {
                    dialog = dialog.child(Feedback::new(
                        theme,
                        FeedbackTone::Error,
                        t_vars(key, &[("error", error)]),
                    ));
                } else {
                    dialog = dialog.description(t(key));
                }
                if !busy {
                    dialog = dialog.child(
                        actions(theme).child(
                            Button::new("update-close", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.close"))
                                .on_click(dismiss()),
                        ),
                    );
                }
                dialog
            }
        })
    }
}

impl Render for Details {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = self.dialog(window, cx);
        if dialog.is_some()
            && !self.focus.is_focused(window)
            && !self.focus.contains_focused(window, cx)
        {
            // Responsive shell navigation can remount the view underneath the dialog.
            self.focus.focus(window, cx);
        }
        div().absolute().inset_0().children(dialog)
    }
}

fn scroll_on_key(scroll: &ScrollHandle, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let offset = scroll.offset().y;
    let limit = -scroll.max_offset().y;
    let page = scroll.bounds().size.height;
    let next = match event.keystroke.key.as_str() {
        "up" => offset + px(24.),
        "down" => offset - px(24.),
        "pageup" => offset + page,
        "pagedown" => offset - page,
        "home" => px(0.),
        "end" => limit,
        _ => return,
    };
    scroll.set_offset(point(px(0.), next.clamp(limit, px(0.))));
    window.refresh();
    window.prevent_default();
    cx.stop_propagation();
}

#[cfg(test)]
mod tests {
    use super::nonblank;

    #[test]
    fn notes_keep_internal_line_breaks_and_skip_blank_metadata() {
        assert_eq!(nonblank(None), None);
        assert_eq!(nonblank(Some(" \n\t")), None);
        assert_eq!(
            nonblank(Some("\n- First\n\n- Second\n")),
            Some("- First\n\n- Second")
        );
    }
}
