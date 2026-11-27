//! Desktop Settings → Updates. Mounted after the Integration section.

use crate::features::app::updater::{Status, Updater, CURRENT_VERSION};
use gpui_kit::*;
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
            Status::Available(candidate) => t_vars(
                "settings.desktop.archAvailable",
                &[("package", &candidate.label())],
            ),
            Status::Idle => String::new(),
            Status::Checking => t("settings.desktop.checking"),
            Status::Current => t("settings.desktop.archCurrent"),
            Status::Confirming => t("settings.desktop.archConfirm"),
            Status::Downloading => t("settings.desktop.archDownloading"),
            Status::Installing => t("settings.desktop.archInstalling"),
            Status::Installed => t("settings.desktop.archInstalled"),
            Status::InstallFailed(error) => {
                t_vars("settings.desktop.archFailed", &[("error", error)])
            }
        };
        let button = Button::new("desktop-check-update", theme.clone())
            .variant(ButtonVariant::Secondary)
            .label(t(if matches!(status, Status::Checking) {
                "settings.desktop.checking"
            } else {
                "settings.desktop.checkNow"
            }))
            .disabled(status.busy() || matches!(status, Status::Installed))
            .on_click(cx.listener(|this, _, _, cx| this.check(cx)));
        #[cfg(all(
            target_arch = "x86_64",
            any(target_os = "linux", target_os = "windows")
        ))]
        let button = if matches!(status, Status::Available(_)) {
            Button::new("desktop-install-update", theme.clone())
                .variant(ButtonVariant::Secondary)
                .label(t("settings.desktop.archInstall"))
                .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)))
        } else {
            button
        };
        settings_section(&theme, &t("settings.desktop.updates")).child(settings_row(
            &theme,
            &format!("Stonks Chat {CURRENT_VERSION}"),
            &description,
            button,
        ))
    }
}
