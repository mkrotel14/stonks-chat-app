//! Shortcut settings, including a focused recorder that intercepts before shell actions.
use crate::{features::shell::shortcuts::chord, preferences::NativePreferences};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::preferences::{
    keybinds::{format, Action},
    Change,
};
use stonks_ui::{
    components::{settings_row, settings_section, Button, ButtonVariant, UiDialog},
    i18n::{t, t_vars},
    Theme,
};

#[derive(IntoElement)]
pub(super) struct KeybindPage;

impl RenderOnce for KeybindPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state("keybind-page", cx, |_, cx| Recorder::new(cx))
    }
}

struct Recorder {
    action: Option<Action>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    error: Option<String>,
    _capture: Subscription,
}

fn title(action: Action) -> String {
    t(&format!("settings.keybinds.actions.{}", action.name()))
}

impl Recorder {
    fn new(cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let entity = cx.entity().downgrade();
        let capture = cx.intercept_keystrokes(move |event, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                let Some(action) = this.action else {
                    return;
                };
                if !this.focus.contains_focused(window, cx) {
                    return;
                }
                if event.keystroke.key == "escape" {
                    this.close(window, cx);
                } else if matches!(event.keystroke.key.as_str(), "tab" | "enter" | "space")
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.platform
                {
                    // Keep keyboard traversal and activation of dialog controls available.
                    return;
                } else if let Some(value) = chord(&event.keystroke) {
                    this.save(action, value, window, cx);
                }
                window.prevent_default();
                cx.stop_propagation();
            });
        });
        Self {
            action: None,
            focus,
            restore: None,
            error: None,
            _capture: capture,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.action = None;
        if let Some(focus) = self.restore.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn save(&mut self, action: Action, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.error = if let Some(conflict) = cx
            .global::<NativePreferences>()
            .saved
            .keybinds
            .conflict(action, &value)
        {
            Some(t_vars(
                "settings.keybinds.conflict",
                &[("action", title(conflict))],
            ))
        } else {
            NativePreferences::commit(Change::Keybind(action, value), window, cx)
                .err()
                .map(|error| error.to_string())
        };
        self.close(window, cx);
    }
}

impl Render for Recorder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let prefs = cx.global::<NativePreferences>();
        let bindings = prefs.saved.keybinds.clone();
        let disabled = prefs.load_error.is_some();
        let mut section = settings_section(&theme, &t("settings.keybinds.heading"))
            .child(
                div()
                    .max_w(px(630.))
                    .mt(px(theme.space1))
                    .mb(px(theme.space5))
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_sm))
                    .line_height(relative(theme.line_height_normal))
                    .child(t("settings.keybinds.description")),
            )
            .when_some(self.error.clone(), |section, error| {
                section.child(div().text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .children(Action::ALL.map(|action| {
                        let value = bindings.binding(action);
                        let recording = self.action == Some(action);
                        let trailing = div().min_w(px(132.)).child(
                            Button::new(action.name(), theme)
                                .variant(ButtonVariant::Secondary)
                                .selected(recording)
                                .disabled(disabled)
                                .label(if recording {
                                    t("settings.keybinds.pressKeys")
                                } else if value.is_empty() {
                                    t("settings.keybinds.unassigned")
                                } else {
                                    format(value)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.error = None;
                                    this.restore = window.focused(cx);
                                    this.action = Some(action);
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        );
                        settings_row(
                            &theme,
                            &title(action),
                            &t(&format!("settings.keybinds.descriptions.{}", action.name())),
                            trailing,
                        )
                    })),
            );
        if let Some(action) = self.action {
            let dialog = UiDialog::new("keybind-recorder", theme, title(action))
                .description(t("settings.keybinds.pressKeys"))
                .focus(&self.focus)
                .on_close(cx.listener(|this, _, window, cx| this.close(window, cx)))
                .child(t("settings.keybinds.description"))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .mt(px(theme.space1))
                        .gap(px(theme.space2))
                        .child(
                            Button::new("clear", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("settings.keybinds.clear"))
                                .disabled(disabled || bindings.binding(action).is_empty())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save(action, String::new(), window, cx)
                                })),
                        )
                        .child(
                            Button::new("reset", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("settings.keybinds.reset"))
                                .disabled(
                                    disabled
                                        || bindings.binding(action) == action.default_binding(),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save(action, action.default_binding().into(), window, cx)
                                })),
                        )
                        .child(
                            Button::new("cancel", theme)
                                .label(t("common.cancel"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                                ),
                        ),
                );
            section = section.child(
                deferred(
                    anchored().position(point(px(0.), px(0.))).child(
                        div()
                            .relative()
                            .w(window.viewport_size().width)
                            .h(window.viewport_size().height)
                            .child(dialog),
                    ),
                )
                .with_priority(100),
            );
        }
        section
    }
}
