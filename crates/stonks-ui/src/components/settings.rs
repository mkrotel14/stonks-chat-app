//! Shared passive settings compositions, matching SettingsSection/SettingsRow.
use crate::Theme;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

pub fn settings_section(theme: &Theme, title: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme.space2))
        .mt(px(theme.space6))
        .child(
            div()
                .text_size(px(theme.font_size_sm))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_muted)
                .mb(px(theme.space1))
                .child(title.to_uppercase()),
        )
}
pub fn settings_field(theme: &Theme, label: &str, hint: impl Into<String>) -> Div {
    let hint = hint.into();
    let mut field = div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme.space2))
        .child(
            div()
                .text_color(theme.text)
                .text_size(px(theme.font_size_md))
                .font_weight(FontWeight::BOLD)
                .child(label.to_owned()),
        );
    if !hint.is_empty() {
        field = field.child(
            div()
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_sm))
                .child(hint),
        );
    }
    field
}
/// The shared UiField composition: smaller label and description below the input.
pub fn input_field(theme: &Theme, label: &str, hint: &str, input: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme.space2))
        .text_color(theme.text)
        .child(
            div()
                .text_size(px(theme.font_size_sm))
                .font_weight(FontWeight::BOLD)
                .child(label.to_owned()),
        )
        .child(input)
        .when(!hint.is_empty(), |field| {
            field.child(
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(hint.to_owned()),
            )
        })
}
pub fn settings_row(
    theme: &Theme,
    title: &str,
    description: &str,
    trailing: impl IntoElement,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .min_w_0()
        .min_h(px(62.))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap(px(theme.space4))
        .py(px(theme.space3))
        .px(px(15.))
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_md))
        .bg(theme.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_1()
                .gap(px(theme.space1))
                .child(
                    div()
                        .text_size(px(theme.font_size_md))
                        .line_height(px(theme.font_size_md * theme.line_height_compact))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(theme.font_size_xs))
                        .line_height(px(theme.font_size_xs * theme.line_height_normal))
                        .text_color(theme.text_muted)
                        .child(description.to_owned()),
                ),
        )
        .child(trailing)
}

enum SettingsInputState {
    Single(Entity<InputState>),
    Multi(Entity<TextareaState>),
}
#[derive(IntoElement)]
pub struct SettingsInput {
    state: SettingsInputState,
    theme: Theme,
    disabled: bool,
    label: SharedString,
    password: bool,
    readonly: bool,
    textarea_rows: Option<usize>,
}
impl SettingsInput {
    pub fn new(state: &Entity<InputState>, theme: Theme) -> Self {
        Self {
            state: SettingsInputState::Single(state.clone()),
            theme,
            disabled: false,
            label: SharedString::default(),
            password: false,
            readonly: false,
            textarea_rows: None,
        }
    }
    pub fn textarea(state: &Entity<TextareaState>, theme: Theme) -> Self {
        Self {
            state: SettingsInputState::Multi(state.clone()),
            theme,
            disabled: false,
            label: SharedString::default(),
            password: false,
            readonly: false,
            textarea_rows: None,
        }
    }
    pub fn textarea_rows(mut self, rows: usize) -> Self {
        self.textarea_rows = Some(rows);
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn password(mut self, value: bool) -> Self {
        self.password = value;
        self
    }
    pub fn readonly(mut self, value: bool) -> Self {
        self.readonly = value;
        self
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
}
impl RenderOnce for SettingsInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The installed kit keeps Medium editor padding (10px/8px) even
        // with appearance(false); subtract it from the outer textarea inset.
        let (focused, multiline, input) = match self.state {
            SettingsInputState::Single(state) => {
                if state.read(cx).presentation().is_masked() != self.password {
                    state.update(cx, |state, cx| state.set_masked(self.password, window, cx));
                }
                (
                    state.read(cx).focus_handle(cx).is_focused(window),
                    false,
                    Input::new(&state)
                        .aria_label(self.label.clone())
                        .when(self.password, |input| {
                            input.content_type(
                                gpui_kit::component::input::InputContentType::Password,
                            )
                        })
                        .appearance(false)
                        .disabled(self.disabled)
                        .readonly(self.readonly)
                        .p_0()
                        .h(px(34.))
                        .text_size(px(self.theme.font_size_md))
                        .line_height(px(self.theme.font_size_md * self.theme.line_height_normal))
                        .into_any_element(),
                )
            }
            SettingsInputState::Multi(state) => (
                state.read(cx).focus_handle(cx).is_focused(window),
                true,
                Textarea::new(&state)
                    .aria_label(self.label.clone())
                    .appearance(false)
                    .disabled(self.disabled)
                    .readonly(self.readonly)
                    .p_0()
                    .h(px(self.textarea_rows.map_or(86., |rows| {
                        rows as f32 * self.theme.font_size_md * self.theme.line_height_normal + 16.
                    })))
                    .text_size(px(self.theme.font_size_md))
                    .line_height(px(self.theme.font_size_md * self.theme.line_height_normal))
                    .into_any_element(),
            ),
        };
        div()
            .w_full()
            .min_h(px(if multiline { 96. } else { 36. }))
            .px(px(if multiline {
                self.theme.space3 - 10.
            } else {
                self.theme.space3
            }))
            .py(px(if multiline {
                self.theme.space3 - 8.
            } else {
                0.
            }))
            .rounded(px(self.theme.radius_sm))
            .border_1()
            .border_color(self.theme.outline)
            .bg(self.theme.canvas)
            .when(focused, |view| {
                view.shadow(vec![BoxShadow {
                    color: self.theme.focus.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .text_color(self.theme.text)
            .text_size(px(self.theme.font_size_md))
            .opacity(if self.disabled { 0.5 } else { 1. })
            .child(input)
    }
}
