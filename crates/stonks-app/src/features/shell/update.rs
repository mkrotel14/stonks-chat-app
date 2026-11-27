//! The first action in both shell header layouts when a managed update is ready.

use super::*;
use crate::features::app::updater::{details, Updater};

pub(super) use details::{ConfirmUpdate, ShowUpdateDetails};

pub(super) fn header_update(theme: Theme, cx: &App) -> Option<Button> {
    let updater = Updater::shared(cx)?;
    details::Release::available(&updater.read(cx).status)?;
    Some(
        Button::new("header-update", theme)
            .variant(ButtonVariant::Icon)
            .tooltip("View update")
            .accessibility_label("Update available")
            .leading(Icon::Download.render_stroke(theme.success_text, 18., 2.))
            .on_click(details::show),
    )
}

impl ShellView {
    pub(super) fn update_details_open(&self, cx: &App) -> bool {
        self.update_details
            .as_ref()
            .is_some_and(|details| details.read(cx).is_open())
    }

    pub(super) fn open_update_details(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.update_details_open(cx) {
            self.update_details = details::Details::open(confirm, window, cx);
            cx.notify();
        }
    }
}
