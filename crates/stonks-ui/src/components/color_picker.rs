//! Source colour-input trigger with native picker state and token controls.
use crate::{
    components::{settings_field, ColorSwatch, SettingsInput, UiRange},
    i18n::t,
    Theme,
};
use gpui_kit::base::ColorPicker as BaseColorPicker;
pub use gpui_kit::component::color_picker::{
    ColorPickerEvent as UiColorPickerEvent, ColorPickerState as UiColorPickerState,
};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::Selectable;
use gpui_kit::*;
#[derive(IntoElement)]
struct PickerTrigger(ColorSwatch, bool);
impl Selectable for PickerTrigger {
    fn selected(mut self, value: bool) -> Self {
        self.0 = self.0.selected(value);
        self.1 = value;
        self
    }
    fn is_selected(&self) -> bool {
        self.1
    }
}
impl RenderOnce for PickerTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}
#[derive(IntoElement)]
pub struct UiColorPicker {
    state: Entity<UiColorPickerState>,
    theme: Theme,
    disabled: bool,
    label: SharedString,
}
impl UiColorPicker {
    pub fn new(state: &Entity<UiColorPickerState>, theme: Theme) -> Self {
        Self {
            state: state.clone(),
            theme,
            disabled: false,
            label: SharedString::default(),
        }
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn label(mut self, value: impl Into<SharedString>) -> Self {
        self.label = value.into();
        self
    }
}
impl RenderOnce for UiColorPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.state
            .update(cx, |state, cx| state.sync_pending_value(window, cx));
        let theme = self.theme;
        let state = self.state.read(cx);
        let color = state.value().map(Rgba::from).unwrap_or(theme.accent);
        let open = state.is_open();
        let focus = state.focus_handle(cx);
        let sliders = state.sliders().clone();
        let hex = state.hex_input().clone();
        let trigger = ColorSwatch::new(("custom-color", self.state.entity_id()), theme, color)
            .label(self.label.clone())
            .disabled(self.disabled);
        if self.disabled {
            return trigger.into_any_element();
        }
        let change = self.state.clone();
        let popover = self.state.clone();
        let mut popup = Popover::new(("color-popup", self.state.entity_id()))
            .open(open)
            .appearance(false)
            .p(px(theme.space3))
            .w(px(280.))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_md))
            .trigger(PickerTrigger(trigger, false))
            .on_open_change(move |open: &bool, _, cx| {
                popover.update(cx, |state, cx| state.set_open(*open, cx))
            });
        if open {
            popup = popup.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space3))
                    .child(
                        div()
                            .w_full()
                            .h(px(40.))
                            .rounded(px(theme.radius_sm))
                            .bg(color),
                    )
                    .child(
                        settings_field(&theme, &t("serverSettings.colourHue"), "").child(
                            UiRange::new(sliders.hue(), theme).label(t("serverSettings.colourHue")),
                        ),
                    )
                    .child(
                        settings_field(&theme, &t("serverSettings.colourSaturation"), "").child(
                            UiRange::new(sliders.saturation(), theme)
                                .label(t("serverSettings.colourSaturation")),
                        ),
                    )
                    .child(
                        settings_field(&theme, &t("serverSettings.colourLightness"), "").child(
                            UiRange::new(sliders.lightness(), theme)
                                .label(t("serverSettings.colourLightness")),
                        ),
                    )
                    .child(
                        settings_field(&theme, &t("serverSettings.customColour"), "").child(
                            SettingsInput::new(&hex, theme).label(t("serverSettings.customColour")),
                        ),
                    ),
            );
        }
        BaseColorPicker::new(("color-picker", self.state.entity_id()))
            .open(open)
            .track_focus(&focus)
            .accessibility_label(self.label)
            .on_open_change(move |open, _, cx| {
                change.update(cx, |state, cx| state.set_open(open, cx))
            })
            .child(popup)
            .into_any_element()
    }
}
