//! Shared settings dismissal control, matching UiCloseButton's icon geometry.
use crate::{
    components::{Button, ButtonVariant},
    icons::Icon,
    Theme,
};
use gpui_kit::{App, ClickEvent, ElementId, IntoElement, RenderOnce, Window};

#[derive(IntoElement)]
pub struct UiCloseButton(Button);
impl UiCloseButton {
    pub fn new(id: impl Into<ElementId>, theme: Theme) -> Self {
        Self(
            Button::new(id, theme)
                .variant(ButtonVariant::Close)
                .tooltip(crate::i18n::t("common.close"))
                .leading(Icon::X.render_stroke(theme.text_muted, 18., 1.8)),
        )
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.0 = self.0.disabled(disabled);
        self
    }
    pub fn on_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.0 = self.0.on_click(listener);
        self
    }
}
impl RenderOnce for UiCloseButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}
