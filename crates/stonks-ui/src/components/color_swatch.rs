//! Shared source34px role-colour control.
use crate::{theme::TRANSPARENT, Theme};
use gpui_kit::*;
use std::rc::Rc;
#[derive(IntoElement)]
pub struct ColorSwatch {
    id: ElementId,
    theme: Theme,
    color: Rgba,
    label: Option<SharedString>,
    selected: bool,
    disabled: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}
impl ColorSwatch {
    pub fn new(id: impl Into<ElementId>, theme: Theme, color: Rgba) -> Self {
        Self {
            id: id.into(),
            theme,
            color,
            label: None,
            selected: false,
            disabled: false,
            on_click: None,
        }
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn selected(mut self, value: bool) -> Self {
        self.selected = value;
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for ColorSwatch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let label = self.label.unwrap_or_else(|| {
            format!(
                "Use #{:02x}{:02x}{:02x}",
                (self.color.r * 255.).round() as u8,
                (self.color.g * 255.).round() as u8,
                (self.color.b * 255.).round() as u8
            )
            .into()
        });
        let mut button = div()
            .id(self.id)
            .role(Role::Button)
            .aria_label(label)
            .aria_toggled(if self.selected {
                Toggled::True
            } else {
                Toggled::False
            })
            .size(px(34.))
            .flex_shrink_0()
            .rounded(px(theme.radius_pill))
            .border_2()
            .border_color(if self.selected {
                theme.text
            } else {
                TRANSPARENT
            })
            .bg(self.color);
        if self.disabled {
            button = button.opacity(0.5).cursor_not_allowed();
        } else {
            button = button
                .tab_index(0)
                .cursor_pointer()
                .focus_visible(move |style| style.border_color(theme.focus));
            if let Some(callback) = self.on_click {
                button = button.on_click(move |event, window, cx| callback(event, window, cx));
            }
        }
        button
    }
}

impl gpui_kit::base::Selectable for ColorSwatch {
    fn selected(mut self, value: bool) -> Self {
        self.selected = value;
        self
    }
    fn is_selected(&self) -> bool {
        self.selected
    }
}
