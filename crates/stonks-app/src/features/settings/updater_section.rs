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
        let busy = matches!(self.status, Status::Checking);
        let details = match &self.status {
            Status::PublicRelease(version) => Some(t_vars(
                "settings.desktop.gpuiPublicRelease",
                &[("version", version)],
            )),
            Status::Failed(error) => Some(t(match error {
                DiscoveryError::Unavailable => "settings.desktop.gpuiReleaseUnavailable",
                DiscoveryError::InvalidMetadata => "settings.desktop.gpuiReleaseInvalid",
            })),
            Status::Idle | Status::Checking => None,
        };
        let channel = t("settings.desktop.gpuiUpdateUnsupported");
        let description = match details {
            Some(details) => format!("{details} {channel}"),
            None => channel,
        };
        settings_section(&theme, &t("settings.desktop.updates")).child(settings_row(
            &theme,
            &format!("Stonks Chat {CURRENT_VERSION}"),
            &description,
            Button::new("desktop-check-update", theme)
                .variant(ButtonVariant::Secondary)
                .label(t(if busy {
                    "settings.desktop.checking"
                } else {
                    "settings.desktop.checkNow"
                }))
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.check(cx))),
        ))
    }
}
