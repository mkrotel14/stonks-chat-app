//! Port of `src/ui/primitives/Checkbox.tsx`.
//!
//! 19×19 box, accent fill + accent-text check when checked, outline border
//! otherwise; row gap space3, sm text; focus ring in the focus token;
//! disabled = 50% opacity + not-allowed.
//!
//! Ledger note: the original defers to the native checkbox rendering
//! (`accentColor`); the port draws the box — corner radius (~5px) and check
//! glyph are approximations of the platform look.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::{Theme, TRANSPARENT};

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    theme: Theme,
    label: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, theme: Theme) -> Self {
        Self {
            id: id.into(),
            theme,
            label: None,
            checked: false,
            disabled: false,
            on_change: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(
        mut self,
        listener: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(listener));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        let focus_ring = vec![BoxShadow {
            color: theme.focus_ring.into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(3.),
            inset: false,
        }];

        let mut checkbox = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(px(theme.space3))
            .text_size(px(theme.font_size_sm))
            .text_color(theme.text)
            .child(
                div()
                    .size(px(19.))
                    .flex_shrink_0()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(if self.checked { theme.accent } else { theme.outline })
                    .bg(if self.checked { theme.accent } else { TRANSPARENT })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.accent_text)
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .when(self.checked, |box_| box_.child("✓")),
            );

        if self.disabled {
            checkbox = checkbox.cursor_not_allowed().opacity(0.5);
        } else {
            checkbox = checkbox
                .cursor_pointer()
                .focus_visible(move |style| style.shadow(focus_ring.clone()));
            if let Some(on_change) = self.on_change {
                let next = !self.checked;
                checkbox = checkbox.on_click(move |_, window, cx| on_change(&next, window, cx));
            }
        }
        if let Some(label) = self.label {
            checkbox = checkbox.child(label);
        }
        checkbox
    }
}
