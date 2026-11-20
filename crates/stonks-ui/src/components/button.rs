//! Port of `src/ui/primitives/Button.tsx` and `buttonInteraction.stylex.ts`.
//!
//! Same variants, same geometry, same feedback:
//! hover = 10% text overlay + 1px inset ring at 24% text, active = 18%
//! overlay, danger variants ring in the danger color, focus-visible ring in
//! the focus token, disabled = not-allowed + 50% opacity.
//!
//! Ledger notes (vs. the CSS original):
//! - The active press transform (translateY/scale) has no div-level
//!   equivalent in GPUI; only the overlay is applied.
//! - The focus ring draws adjacent to the border box; the CSS 2px offset gap
//!   is approximated.
//! - Enabled buttons join GPUI's tab order; its stable element focus state
//!   supplies Enter/Space activation and focus-visible styling.

use std::rc::Rc;

use gpui_kit::*;

use crate::theme::{feedback_overlay, mix_color, Theme, TRANSPARENT};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Danger,
    Quiet,
    Choice,
    Nav,
    FloatingTab,
    Row,
    SettingsRow,
    DangerQuiet,
    DangerSubtle,
    Rail,
    Channel,
    Heading,
    Icon,
    IconSmall,
    Close,
    SearchResult,
    Accordion,
    DisclosureChoice,
    PermissionAllow,
    PermissionDeny,
    PermissionInherit,
    IconUpload,
    RoleIconUpload,
    EmojiUpload,
    EmojiPreview,
    MfaMethod,
    MemberRole,
    MemberRow,
    MemberAvatar,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    theme: Theme,
    variant: ButtonVariant,
    label: Option<SharedString>,
    tooltip: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    tooltip_content: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    selected: bool,
    icon_only: bool,
    icon_button_size: Option<f32>,
    muted: bool,
    disabled: bool,
    focus: Option<FocusHandle>,
    expanded: Option<bool>,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_right_click: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, theme: Theme) -> Self {
        Self {
            id: id.into(),
            theme,
            variant: ButtonVariant::Primary,
            label: None,
            tooltip: None,
            accessibility_label: None,
            tooltip_content: None,
            leading: None,
            trailing: None,
            selected: false,
            icon_only: false,
            icon_button_size: None,
            muted: false,
            disabled: false,
            focus: None,
            expanded: None,
            on_click: None,
            on_right_click: None,
        }
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn accessibility_label(mut self, text: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(text.into());
        self
    }

    pub fn tooltip_content(
        mut self,
        builder: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.tooltip_content = Some(Rc::new(builder));
        self
    }

    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Optional leading element (e.g. an `Icon` render) before the label.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    pub fn muted(mut self, value: bool) -> Self {
        self.muted = value;
        self
    }
    pub fn icon_only(mut self, value: bool) -> Self {
        self.icon_only = value;
        self
    }
    pub fn icon_button_size(mut self, size: f32) -> Self {
        self.icon_button_size = Some(size);
        self.icon_only = true;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_right_click(
        mut self,
        callback: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_right_click = Some(Rc::new(callback));
        self
    }

    pub fn on_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(listener));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        let danger_family = matches!(
            self.variant,
            ButtonVariant::Danger | ButtonVariant::DangerQuiet | ButtonVariant::DangerSubtle
        );

        // Resting colors per variant (from Button.tsx stylex classes).
        let (bg, fg) = match self.variant {
            ButtonVariant::Primary => (theme.accent, theme.accent_text),
            ButtonVariant::Secondary => (theme.surface_interactive, theme.text),
            ButtonVariant::Danger => (theme.danger, theme.danger_text),
            ButtonVariant::Quiet if self.selected => (theme.surface_selected, theme.text),
            ButtonVariant::Quiet => (TRANSPARENT, theme.text),
            ButtonVariant::Choice => {
                if self.selected {
                    (theme.surface_selected, theme.text)
                } else {
                    (theme.surface, theme.text)
                }
            }
            ButtonVariant::FloatingTab => (
                if self.selected {
                    theme.surface_selected
                } else {
                    TRANSPARENT
                },
                if self.selected {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ),
            ButtonVariant::Nav => {
                if self.selected {
                    (theme.surface_selected, theme.text)
                } else {
                    (TRANSPARENT, theme.text_muted)
                }
            }
            ButtonVariant::Row | ButtonVariant::SettingsRow => (theme.surface, theme.text),
            ButtonVariant::DangerQuiet => (TRANSPARENT, theme.danger),
            ButtonVariant::DangerSubtle => (
                mix_color(theme.surface_raised, theme.danger, 0.14),
                theme.danger,
            ),
            ButtonVariant::Rail => (
                if self.selected {
                    theme.accent
                } else {
                    theme.surface_interactive
                },
                if self.selected {
                    theme.accent_text
                } else {
                    theme.text
                },
            ),
            ButtonVariant::Channel => (
                if self.selected {
                    theme.surface_selected
                } else {
                    TRANSPARENT
                },
                if self.muted && !self.selected {
                    theme.text_muted
                } else {
                    theme.text
                },
            ),
            ButtonVariant::Heading => (TRANSPARENT, theme.text_muted),
            ButtonVariant::Icon | ButtonVariant::IconSmall => (
                if self.selected {
                    theme.surface_selected
                } else {
                    TRANSPARENT
                },
                theme.text,
            ),
            ButtonVariant::Close => (theme.surface_interactive, theme.text_muted),
            ButtonVariant::MfaMethod => (
                if self.selected {
                    theme.surface_selected
                } else {
                    theme.surface_interactive
                },
                if self.selected {
                    theme.text
                } else {
                    theme.text_muted
                },
            ),
            ButtonVariant::MemberRole => (theme.surface_interactive, theme.text),
            ButtonVariant::MemberRow => (TRANSPARENT, theme.text),
            ButtonVariant::MemberAvatar => (TRANSPARENT, theme.text_muted),
            ButtonVariant::SearchResult => (TRANSPARENT, theme.text),
            ButtonVariant::DisclosureChoice => (
                if self.selected {
                    theme.surface_selected
                } else {
                    theme.surface
                },
                theme.text,
            ),
            ButtonVariant::Accordion
            | ButtonVariant::IconUpload
            | ButtonVariant::RoleIconUpload
            | ButtonVariant::EmojiUpload
            | ButtonVariant::EmojiPreview => (TRANSPARENT, theme.text),
            ButtonVariant::PermissionAllow => (
                if self.selected {
                    theme.surface_selected
                } else {
                    TRANSPARENT
                },
                if self.selected {
                    theme.success
                } else {
                    theme.text
                },
            ),
            ButtonVariant::PermissionDeny => (
                if self.selected {
                    theme.surface_selected
                } else {
                    TRANSPARENT
                },
                if self.selected {
                    theme.danger
                } else {
                    theme.text
                },
            ),
            ButtonVariant::PermissionInherit => (
                if self.selected {
                    theme.accent
                } else {
                    TRANSPARENT
                },
                if self.selected {
                    theme.accent_text
                } else {
                    theme.text
                },
            ),
        };

        // buttonInteraction.control: hover/active feedback without changing
        // the resting surface or geometry.
        let hover_bg = feedback_overlay(bg, theme.text, 0.10);
        let active_bg = feedback_overlay(bg, theme.text, 0.18);
        let ring_color = if danger_family {
            Rgba {
                a: 1.0,
                ..theme.danger
            }
        } else {
            Rgba {
                a: 0.24,
                ..theme.text
            }
        };
        let inset_ring = vec![BoxShadow {
            color: ring_color.into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            // CSS uses a 1px inset spread. Zero spread leaves the shadow hole
            // flush with the button edge, so only the hover tint is visible.
            spread_radius: px(if self.variant == ButtonVariant::DangerQuiet {
                1.
            } else {
                0.
            }),
            inset: true,
        }];
        let focus_ring = vec![BoxShadow {
            color: theme.focus_ring.into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(3.),
            inset: false,
        }];

        let mut button = div()
            .id(self.id)
            .role(Role::Button)
            .flex()
            .items_center()
            .justify_center()
            .min_h(px(34.))
            .gap(px(theme.space2))
            .px(px(theme.space3))
            .rounded(px(theme.radius_sm))
            .text_size(px(theme.font_size_sm))
            .font_weight(FontWeight::BOLD)
            .bg(bg)
            .text_color(fg);

        button = match self.variant {
            ButtonVariant::Rail => button
                .size(px(42.))
                .min_h(px(42.))
                .p_0()
                .rounded(px(theme.radius_pill))
                .flex_shrink_0(),
            ButtonVariant::Channel => button
                .w_full()
                .min_h(px(36.))
                .px(px(9.))
                .py(px(6.))
                .justify_start()
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::Heading => button
                .w_full()
                .min_h(px(28.))
                .px(px(10.))
                .py(px(3.))
                .justify_start()
                .text_size(px(theme.font_size_xs)),
            ButtonVariant::Icon => button
                .size(px(34.))
                .p_0()
                .rounded(px(theme.radius_pill))
                .flex_shrink_0(),
            ButtonVariant::IconSmall => button
                .size(px(29.))
                .min_h(px(29.))
                .p_0()
                .rounded(px(theme.radius_pill))
                .flex_shrink_0(),
            ButtonVariant::Close => button
                .size(px(36.))
                .min_h(px(36.))
                .p_0()
                .rounded(px(theme.radius_pill))
                .flex_shrink_0(),
            ButtonVariant::MfaMethod => button
                .min_h(px(42.))
                .w_full()
                .border_1()
                .border_color(if self.selected {
                    theme.accent
                } else {
                    theme.outline
                })
                .text_size(px(theme.font_size_xs))
                .font_weight(FontWeight(750.)),
            ButtonVariant::MemberRole => button
                .min_h(px(48.))
                .w_full()
                .justify_start()
                .gap(px(theme.space3))
                .px(px(theme.space3))
                .text_size(px(theme.font_size_md))
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::MemberRow => button
                .w_full()
                .min_w_0()
                .min_h(px(48.))
                .justify_start()
                .gap(px(theme.space3))
                .px(px(theme.space2))
                .py(px(theme.space1))
                .text_size(px(theme.font_size_md))
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::MemberAvatar => button
                .p(px(theme.space2))
                .gap(px(theme.space3))
                .rounded(px(theme.radius_md))
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::IconUpload => button
                .size(px(96.))
                .p_0()
                .overflow_hidden()
                .rounded(px(theme.radius_pill)),
            ButtonVariant::RoleIconUpload => button
                .size(px(72.))
                .p_0()
                .overflow_hidden()
                .rounded(px(theme.radius_pill)),
            ButtonVariant::EmojiUpload => button
                .size(px(82.))
                .p_0()
                .overflow_hidden()
                .rounded(px(theme.radius_pill))
                .border_1()
                .border_color(theme.outline),
            ButtonVariant::EmojiPreview => button
                .size(px(42.))
                .p(px(theme.space1))
                .overflow_hidden()
                .rounded(px(theme.radius_sm)),
            ButtonVariant::DisclosureChoice => button
                .w_full()
                .min_h(px(54.))
                .flex_shrink_0()
                .justify_between()
                .gap(px(theme.space4))
                .px(px(13.))
                .py(px(10.))
                .rounded(px(10.)),
            ButtonVariant::Accordion => button
                .w_full()
                .min_h(px(62.))
                .gap(px(18.))
                .px(px(15.))
                .py(px(12.))
                .rounded(px(theme.radius_md - 1.))
                .rounded_bl(px(if self.expanded == Some(true) {
                    0.
                } else {
                    theme.radius_md - 1.
                }))
                .rounded_br(px(if self.expanded == Some(true) {
                    0.
                } else {
                    theme.radius_md - 1.
                }))
                .text_size(px(theme.font_size_md))
                .line_height(relative(theme.line_height_normal))
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::PermissionAllow
            | ButtonVariant::PermissionDeny
            | ButtonVariant::PermissionInherit => button
                .w(px(36.))
                .h(px(34.))
                .min_h(px(34.))
                .p_0()
                .flex_shrink_0()
                .text_size(px(theme.font_size_lg)),
            ButtonVariant::SearchResult => button
                .w_full()
                .p(px(10.))
                .justify_start()
                .font_weight(FontWeight::NORMAL),
            ButtonVariant::Choice => button
                .w_full()
                .justify_start()
                .p(px(theme.space2))
                .border_2()
                .rounded(px(theme.radius_md))
                .border_color(if self.selected {
                    theme.accent
                } else {
                    TRANSPARENT
                }),
            ButtonVariant::FloatingTab => button
                .h(px(36.))
                .min_h(px(36.))
                .flex_shrink_0()
                .gap(px(5.))
                .px(px(if self.selected { 14. } else { 11. }))
                .rounded(px(theme.radius_pill))
                .font_weight(FontWeight::SEMIBOLD),
            ButtonVariant::Nav => button
                .w_full()
                .min_h(px(39.))
                .justify_start()
                .px(px(10.))
                .py(px(9.)),
            ButtonVariant::Row | ButtonVariant::SettingsRow => button
                .w_full()
                .min_h(px(62.))
                .justify_between()
                .gap(px(18.))
                .px(px(15.))
                .py(px(if self.variant == ButtonVariant::SettingsRow {
                    12.
                } else {
                    13.
                }))
                .border_1()
                .border_color(theme.outline)
                .rounded(px(theme.radius_md)),
            _ => button,
        };

        if self.icon_only {
            if let Some(size) = self.icon_button_size {
                button = button.min_h(px(size));
            }
            button = button
                .size(px(self.icon_button_size.unwrap_or(34.)))
                .p_0()
                .rounded(px(theme.radius_pill))
                .flex_shrink_0();
        }
        if let Some(focus) = self.focus {
            button = button.track_focus(&focus);
        }
        if let Some(expanded) = self.expanded {
            button = button.aria_expanded(expanded);
        }
        if matches!(
            self.variant,
            ButtonVariant::Nav
                | ButtonVariant::FloatingTab
                | ButtonVariant::Channel
                | ButtonVariant::Rail
                | ButtonVariant::Choice
        ) {
            button = button.aria_selected(self.selected);
        }
        if matches!(
            self.variant,
            ButtonVariant::PermissionAllow
                | ButtonVariant::PermissionDeny
                | ButtonVariant::PermissionInherit
        ) {
            button = button.aria_toggled(if self.selected {
                Toggled::True
            } else {
                Toggled::False
            });
        }
        if self.variant == ButtonVariant::DisclosureChoice {
            button = button
                .role(Role::RadioButton)
                .aria_toggled(if self.selected {
                    Toggled::True
                } else {
                    Toggled::False
                });
        }
        let variant = self.variant;
        let selected = self.selected;
        let hover_theme = *theme;
        if self.disabled {
            button = button.cursor_not_allowed().opacity(0.5);
        } else {
            button = button
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| match variant {
                    ButtonVariant::Rail => style
                        .bg(hover_theme.accent)
                        .text_color(hover_theme.accent_text),
                    ButtonVariant::Channel => style.bg(if selected {
                        hover_theme.surface_selected
                    } else {
                        hover_theme.surface_interactive
                    }),
                    ButtonVariant::Heading => style.bg(TRANSPARENT).text_color(hover_theme.text),
                    ButtonVariant::MemberRow => style
                        .bg(hover_theme.surface_interactive)
                        .shadow(inset_ring.clone()),
                    _ => style.bg(hover_bg).shadow(inset_ring.clone()),
                })
                .active(move |style| style.bg(active_bg))
                .focus_visible(move |style| style.shadow(focus_ring.clone()));
            if let Some(on_click) = self.on_click {
                button = button.on_click(move |event, window, cx| on_click(event, window, cx));
            }
        }

        if !self.disabled {
            if let Some(callback) = self.on_right_click {
                button = button.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    cx.stop_propagation();
                    callback(event, window, cx);
                });
            }
        }
        if let Some(text) = self.tooltip {
            button = button.aria_label(text.clone()).tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            });
        }
        if let Some(label) = self.accessibility_label {
            button = button.aria_label(label);
        }
        if let Some(builder) = self.tooltip_content {
            button = button.tooltip(move |window, cx| builder(window, cx));
        }
        if let Some(leading) = self.leading {
            button = button.child(leading);
        }
        if let Some(label) = self.label {
            button = button.child(label);
        }
        if let Some(trailing) = self.trailing {
            button = button.child(trailing);
        }
        if self.variant == ButtonVariant::DisclosureChoice {
            button = button.child(
                div()
                    .size(px(20.))
                    .flex_shrink_0()
                    .rounded_full()
                    .border_2()
                    .border_color(if self.selected {
                        theme.accent
                    } else {
                        theme.text_muted
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(
                        self.selected
                            .then(|| div().size(px(8.)).rounded_full().bg(theme.accent)),
                    ),
            );
        }
        if self.variant == ButtonVariant::Rail && self.selected {
            button = button.relative().child(
                div()
                    .absolute()
                    .left(px(-7.))
                    .w(px(3.))
                    .h(px(24.))
                    .rounded(px(4.))
                    .bg(theme.text),
            );
        }
        button
    }
}
