//! Shared token-coloured native range control with pointer and keyboard stepping.
use crate::Theme;
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::*;
#[derive(IntoElement)]
pub struct UiRange {
    state: Entity<SliderState>,
    theme: Theme,
    label: SharedString,
    disabled: bool,
    slim: bool,
    focus: Option<FocusHandle>,
}
impl UiRange {
    pub fn new(state: &Entity<SliderState>, theme: Theme) -> Self {
        Self {
            state: state.clone(),
            theme,
            label: SharedString::default(),
            disabled: false,
            slim: false,
            focus: None,
        }
    }
    /// Use an 11px thumb and a compact track for a single-value range.
    pub fn slim(mut self, value: bool) -> Self {
        self.slim = value;
        self
    }
    pub fn label(mut self, value: impl Into<SharedString>) -> Self {
        self.label = value.into();
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}
impl RenderOnce for UiRange {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.read(cx);
        let slider = if self.slim && state.value().is_single() {
            use gpui_kit::base::{Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack};
            let position = relative(state.percentage().end);
            let accent = self.theme.accent;
            BaseSlider::new(&self.state)
                .horizontal()
                .disabled(self.disabled)
                .accessibility_label(self.label)
                .w_full()
                .child(
                    SliderTrack::new(&self.state)
                        .axis(Axis::Horizontal)
                        .disabled(self.disabled)
                        .flex()
                        .items_center()
                        .w_full()
                        .h(px(11.))
                        .px(px(5.5))
                        .child(
                            SliderIndicator::new(&self.state)
                                .relative()
                                .w_full()
                                .h(px(4.))
                                .rounded_full()
                                .bg(self.theme.surface_selected)
                                .child(div().h_full().w(position).rounded_full().bg(accent))
                                .child(
                                    SliderThumb::new(&self.state)
                                        .axis(Axis::Horizontal)
                                        .disabled(self.disabled)
                                        .absolute()
                                        .left(position)
                                        .ml(px(-5.5))
                                        .top(px(-3.5))
                                        .size(px(11.))
                                        .rounded_full()
                                        .bg(if self.disabled {
                                            self.theme.text_muted
                                        } else {
                                            accent
                                        }),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            Slider::new(&self.state)
                .horizontal()
                .disabled(self.disabled)
                .accessibility_label(self.label)
                .w_full()
                .bg(self.theme.accent)
                .text_color(self.theme.accent)
                .into_any_element()
        };
        if let Some(focus) = self.focus {
            div()
                .id(("range-focus", self.state.entity_id()))
                .w_full()
                .min_w_0()
                .track_focus(&focus)
                .tab_index(-1)
                .child(slider)
                .into_any_element()
        } else {
            slider
        }
    }
}
