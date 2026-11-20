//! Shared UiLink text variant from src/ui/primitives/Link.tsx.
use crate::Theme;
use gpui_kit::*;

#[derive(IntoElement)]
pub struct UiLink {
    id: ElementId,
    theme: Theme,
    label: SharedString,
    url: SharedString,
}
impl UiLink {
    pub fn new(
        id: impl Into<ElementId>,
        theme: Theme,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            label: label.into(),
            url: url.into(),
        }
    }
}
impl RenderOnce for UiLink {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let url = self.url.clone();
        div()
            .id(self.id)
            .role(Role::Link)
            .aria_label(self.label.clone())
            .tab_index(0)
            .cursor_pointer()
            .text_color(self.theme.link)
            .rounded(px(self.theme.radius_sm))
            .hover(|style| style.underline())
            .focus_visible(move |style| {
                style.shadow(vec![BoxShadow {
                    color: self.theme.focus.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .on_click(move |_, window, cx| crate::external_links::request(&url, window, cx))
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "enter" {
                    window.prevent_default();
                    cx.stop_propagation();
                    crate::external_links::request(&self.url, window, cx);
                }
            })
            .child(self.label)
    }
}
