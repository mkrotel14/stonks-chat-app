//! Desktop Settings → Updates. Mounted after the Integration section.

use crate::features::app::updater::{details, Status, Updater, CURRENT_VERSION};
use gpui_kit::{prelude::FluentBuilder, *};
use stonks_core::release_discovery::DiscoveryError;
use stonks_ui::{
    components::{settings_row, settings_section, Button, ButtonVariant},
    i18n::{t, t_vars},
    Theme,
};

pub(crate) fn render(cx: &App) -> Div {
    div().children(Updater::shared(cx))
}

impl Render for Updater {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let status = &self.status;
        let description = match status {
            Status::PublicRelease(version) => format!(
                "{} {}",
                t_vars(
                    "settings.desktop.gpuiPublicRelease",
                    &[("version", version)],
                ),
                t("settings.desktop.gpuiUpdateUnsupported")
            ),
            Status::Failed(error) => t(match error {
                DiscoveryError::Unavailable => "settings.desktop.gpuiReleaseUnavailable",
                DiscoveryError::InvalidMetadata => "settings.desktop.gpuiReleaseInvalid",
            }),
            #[cfg(all(
                target_arch = "x86_64",
                any(target_os = "linux", target_os = "windows")
            ))]
            Status::Available(_) => t("settings.desktop.updateSource"),
            Status::Checking => t("settings.desktop.checking"),
            Status::Current => t("settings.desktop.archCurrent"),
            Status::Downloading => t("settings.desktop.archDownloading"),
            Status::Installing => t("settings.desktop.archInstalling"),
            Status::Verifying => "Verifying the download…".into(),
            #[cfg(target_os = "linux")]
            Status::Restarting => "Restarting Stonks Chat…".into(),
            #[cfg(target_os = "windows")]
            Status::InstallerLaunched => "Installer opened".into(),
            Status::InstallFailed(error) => {
                t_vars("settings.desktop.archFailed", &[("error", error)])
            }
            _ if status.busy() => t("settings.desktop.preparing"),
            _ => String::new(),
        };
        let button = Button::new("desktop-check-update", theme)
            .variant(ButtonVariant::Secondary)
            .label(t(if matches!(status, Status::Checking) {
                "settings.desktop.checking"
            } else {
                "settings.desktop.checkNow"
            }))
            .disabled(status.busy() || status.handing_off())
            .on_click(cx.listener(|this, _, _, cx| this.check(cx)));
        settings_section(&theme, &t("settings.desktop.updates"))
            .child(settings_row(
                &theme,
                &format!("Stonks Chat {CURRENT_VERSION}"),
                &description,
                button,
            ))
            .children(
                details::Release::available(status).map(|release| available_card(theme, release)),
            )
    }
}

fn available_card(theme: Theme, release: details::Release<'_>) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme.space3))
        .mt(px(theme.space3))
        .p(px(theme.space4))
        .rounded(px(theme.radius_md))
        .border_1()
        .border_color(theme.outline)
        .bg(theme.surface_selected)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(theme.space3))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex_basis(px(220.))
                        .flex()
                        .flex_col()
                        .gap(px(theme.space1))
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.accent)
                                .font_weight(FontWeight(850.))
                                .child(t("settings.desktop.updateAvailable")),
                        )
                        .child(
                            div()
                                .text_size(px(theme.font_size_md))
                                .font_weight(FontWeight::BOLD)
                                .child(format!("Stonks Chat {}", release.version)),
                        )
                        .child(
                            div()
                                .text_size(px(theme.font_size_sm))
                                .text_color(theme.text_muted)
                                .line_height(relative(theme.line_height_normal))
                                .child(release.name_or_asset().to_owned()),
                        ),
                )
                .child(
                    Button::new("desktop-update-details", theme)
                        .label(t("settings.desktop.installUpdate"))
                        .on_click(details::confirm),
                ),
        )
        .when_some(release.notes, |card, notes| {
            card.child(details::notes(
                theme,
                "desktop-release-notes",
                notes,
                px(180.),
            ))
        })
}
