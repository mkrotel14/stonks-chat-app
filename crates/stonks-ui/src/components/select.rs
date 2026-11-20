//! Token-styled wrapper around the installed kit's native Select behavior.
use crate::Theme;
pub use gpui_kit::component::select::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
pub use gpui_kit::component::IndexPath;
use gpui_kit::*;

#[derive(Clone)]
pub struct UiSelectOption {
    value: SharedString,
    label: SharedString,
    disabled: bool,
}
impl UiSelectOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}
impl SelectItem for UiSelectOption {
    type Value = SharedString;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &SharedString {
        &self.value
    }
    fn disabled(&self) -> bool {
        self.disabled
    }
}
pub type UiSelectState = SelectState<SearchableVec<UiSelectOption>>;
pub type UiSelectEvent = SelectEvent<SearchableVec<UiSelectOption>>;

#[derive(IntoElement)]
pub struct UiSelect {
    state: Entity<UiSelectState>,
    theme: Theme,
    disabled: bool,
    min_height: f32,
    label: SharedString,
}
impl UiSelect {
    pub fn new(state: &Entity<UiSelectState>, theme: Theme) -> Self {
        Self {
            state: state.clone(),
            theme,
            disabled: false,
            min_height: 36.,
            label: SharedString::default(),
        }
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    /// Shared contexts can preserve the source control's explicit height.
    pub fn min_height(mut self, height: f32) -> Self {
        self.min_height = height.max(0.);
        self
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
}
impl RenderOnce for UiSelect {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        Select::new(&self.state)
            .appearance(false)
            .disabled(self.disabled)
            .accessibility_label(self.label)
            .w_full()
            .min_h(px(self.min_height))
            .px(px(self.theme.space3))
            .rounded(px(self.theme.radius_sm))
            .bg(self.theme.surface)
            .text_color(self.theme.text)
            .text_size(px(self.theme.font_size_md))
            .border_1()
            .border_color(if focused {
                self.theme.focus
            } else {
                self.theme.outline
            })
    }
}
