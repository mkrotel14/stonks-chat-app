//! Port of `src/ui/primitives/Switch.tsx`.
//!
//! Same geometry: 40×23 track (13px radius), 15px thumb at 4px inset,
//! sliding 17px when checked; track surface-interactive → accent, thumb
//! text-muted → accent-text. Disabled = not-allowed + 50% opacity.
//!
//! Ledger note: the CSS slide/color transition is instant here until
//! `motion.rs` lands; state change is correct, animation is pending.

use std::rc::Rc;

use gpui_kit::base::Switch as NativeSwitch;
use gpui_kit::*;

use crate::theme::Theme;

const TRACK_WIDTH: f32 = 40.0;
const TRACK_HEIGHT: f32 = 23.0;
const THUMB_SIZE: f32 = 15.0;
const THUMB_INSET: f32 = 4.0;
const THUMB_TRAVEL: f32 = 17.0;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    theme: Theme,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    focus: Option<FocusHandle>,
    on_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, theme: Theme) -> Self {
        Self {
            id: id.into(),
            theme,
            checked: false,
            disabled: false,
            label: None,
            focus: None,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn on_change(mut self, listener: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(listener));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        let track_color = if self.checked {
            theme.accent
        } else {
            theme.surface_interactive
        };
        let thumb_color = if self.checked {
            theme.accent_text
        } else {
            theme.text_muted
        };
        let thumb_left = THUMB_INSET + if self.checked { THUMB_TRAVEL } else { 0.0 };

        // The installed native switch owns stable focus, tab traversal,
        // Enter/Space click activation and checked/disabled accessibility.
        let mut track = NativeSwitch::new(self.id)
            .checked(self.checked)
            .disabled(self.disabled)
            .relative()
            .w(px(TRACK_WIDTH))
            .h(px(TRACK_HEIGHT))
            .rounded(px(13.))
            .bg(track_color)
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .top(px(THUMB_INSET))
                    .left(px(thumb_left))
                    .size(px(THUMB_SIZE))
                    .rounded(px(THUMB_SIZE / 2.0))
                    .bg(thumb_color),
            );

        if let Some(label) = self.label {
            track = track.accessibility_label(label);
        }
        if let Some(focus) = self.focus {
            track = track.track_focus(&focus);
        }
        if self.disabled {
            track = track.cursor_not_allowed().opacity(0.5);
        } else {
            let focus_ring = vec![BoxShadow {
                color: theme.focus_ring.into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(3.),
                inset: false,
            }];
            track = track
                .cursor_pointer()
                .focus_visible(move |style| style.shadow(focus_ring.clone()));
            if let Some(on_change) = self.on_change {
                track = track.on_change(move |next, _, window, cx| on_change(&next, window, cx));
            }
        }
        track
    }
}
