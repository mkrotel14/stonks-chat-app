//! Port of UiResizeGrip: transparent9px target, centered3×38 accent marker.
use crate::{theme::TRANSPARENT, Theme};
use gpui_kit::*;
use std::rc::Rc;
#[derive(IntoElement)]
pub struct UiResizeGrip {
    id: ElementId,
    theme: Theme,
    active: bool,
    pointer: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    key: Option<Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>>,
}
impl UiResizeGrip {
    pub fn new(id: impl Into<ElementId>, theme: Theme) -> Self {
        Self {
            id: id.into(),
            theme,
            active: false,
            pointer: None,
            key: None,
        }
    }
    pub fn active(mut self, value: bool) -> Self {
        self.active = value;
        self
    }
    pub fn on_mouse_down(
        mut self,
        callback: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.pointer = Some(Rc::new(callback));
        self
    }
    pub fn on_key_down(
        mut self,
        callback: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.key = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for UiResizeGrip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let mut grip = div()
            .id(self.id)
            .w(px(9.))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .tab_index(0)
            .cursor(CursorStyle::ResizeLeftRight)
            .text_color(if self.active {
                theme.accent
            } else {
                TRANSPARENT
            })
            .hover(move |style| style.text_color(theme.accent))
            .focus_visible(move |style| {
                style.text_color(theme.accent).shadow(vec![BoxShadow {
                    color: theme.focus.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }])
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        window.paint_quad(quad(
                            bounds,
                            px(1.5),
                            window.text_style().color,
                            px(0.),
                            TRANSPARENT,
                            Default::default(),
                        ));
                    },
                )
                .w(px(3.))
                .h(px(38.)),
            );
        if let Some(pointer) = self.pointer {
            grip = grip.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                pointer(event, window, cx)
            });
        }
        if let Some(key) = self.key {
            grip = grip.on_key_down(move |event, window, cx| key(event, window, cx));
        }
        grip
    }
}
