//! UiDialog geometry and focus boundary from src/ui/primitives/Overlay.tsx.
use crate::{components::UiCloseButton, Theme};
use gpui_kit::base::FocusTrapElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
#[derive(IntoElement)]
pub struct UiDialog {
    id: ElementId,
    theme: Theme,
    title: SharedString,
    description: Option<SharedString>,
    busy: bool,
    width: f32,
    padding: f32,
    viewport_inset: Option<f32>,
    gap: f32,
    show_close: bool,
    position: Option<Point<Pixels>>,
    title_content: Option<AnyElement>,
    focus: Option<FocusHandle>,
    children: Vec<AnyElement>,
    on_close: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}
impl UiDialog {
    pub fn new(id: impl Into<ElementId>, theme: Theme, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            theme,
            title: title.into(),
            description: None,
            busy: false,
            width: 560.,
            padding: 20.,
            viewport_inset: None,
            gap: theme.space4,
            show_close: true,
            position: None,
            title_content: None,
            focus: None,
            children: vec![],
            on_close: None,
        }
    }
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn busy(mut self, value: bool) -> Self {
        self.busy = value;
        self
    }
    pub fn width(mut self, value: f32) -> Self {
        self.width = value;
        self
    }
    /// Use fixed viewport gutters instead of the default proportional limits.
    pub fn viewport_inset(mut self, inset: f32) -> Self {
        self.viewport_inset = Some(inset);
        self
    }
    pub fn padding(mut self, value: f32) -> Self {
        self.padding = value;
        self
    }
    pub fn gap(mut self, value: f32) -> Self {
        self.gap = value;
        self
    }
    pub fn show_close(mut self, value: bool) -> Self {
        self.show_close = value;
        self
    }
    pub fn position(mut self, value: Option<Point<Pixels>>) -> Self {
        self.position = value;
        self
    }
    pub fn title_content(mut self, value: impl IntoElement) -> Self {
        self.title_content = Some(value.into_any_element());
        self
    }
    pub fn focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn on_close(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(callback));
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}
impl RenderOnce for UiDialog {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let close = self.on_close.clone();
        let busy = self.busy;
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let (max_width, max_height) = if let Some(inset) = self.viewport_inset {
            ((width - 2. * inset).max(0.), (height - 2. * inset).max(0.))
        } else {
            (width * 0.92, (height * 0.86).min(720.))
        };
        let mut panel = div()
            .id(self.id.clone())
            .role(Role::Dialog)
            .aria_label(self.title.clone())
            .relative()
            .flex()
            .flex_col()
            .gap(px(self.gap))
            .w(px(self.width.min(max_width)))
            .min_w(px(280_f32.min(if self.viewport_inset.is_some() {
                max_width
            } else {
                width
            })))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .p(px(self.padding))
            .rounded(px(theme.radius_lg))
            .border_1()
            .border_color(theme.outline)
            .bg(theme.surface_raised)
            .text_color(theme.text)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape"
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    if !busy {
                        if let Some(close) = &close {
                            close(&ClickEvent::default(), window, cx);
                        }
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        if let Some(title) = self.title_content {
            panel = panel.child(title).when(self.show_close, |panel| {
                panel.child(
                    div()
                        .absolute()
                        .top(px(self
                            .viewport_inset
                            .map_or(theme.space2, |_| self.padding)))
                        .right(px(self
                            .viewport_inset
                            .map_or(theme.space2, |_| self.padding)))
                        .child(
                            UiCloseButton::new("dialog-close", theme)
                                .disabled(busy)
                                .on_click({
                                    let close = self.on_close.clone();
                                    move |event, window, cx| {
                                        if let Some(close) = &close {
                                            close(event, window, cx)
                                        }
                                    }
                                }),
                        ),
                )
            });
        } else {
            panel = panel.child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(theme.space3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(theme.font_size_xl))
                            .line_height(relative(theme.line_height_compact))
                            .font_weight(FontWeight::BOLD)
                            .child(self.title),
                    )
                    .when(self.show_close, |header| {
                        header.child(
                            UiCloseButton::new("dialog-close", theme)
                                .disabled(busy)
                                .on_click({
                                    let close = self.on_close.clone();
                                    move |event, window, cx| {
                                        if let Some(close) = &close {
                                            close(event, window, cx)
                                        }
                                    }
                                }),
                        )
                    }),
            );
        }
        if let Some(position) = self.position {
            panel = panel
                .absolute()
                .left(
                    position
                        .x
                        .max(px(8.))
                        .min((window.viewport_size().width - px(self.width) - px(8.)).max(px(8.))),
                )
                .top(
                    position
                        .y
                        .max(px(8.))
                        .min((window.viewport_size().height - px(24.)).max(px(8.))),
                )
                .max_h((window.viewport_size().height - position.y - px(12.)).max(px(80.)));
        }
        if let Some(description) = self.description {
            panel = panel.child(
                div()
                    .mt(px(-theme.space2))
                    .text_size(px(theme.font_size_sm))
                    .line_height(relative(theme.line_height_normal))
                    .text_color(theme.text_muted)
                    .child(description),
            );
        }
        panel = panel.children(self.children);
        let content = if let Some(focus) = self.focus {
            panel.focus_trap(self.id, &focus).into_any_element()
        } else {
            panel.into_any_element()
        };
        let close = self.on_close;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("dialog-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(if self.position.is_some() {
                        crate::theme::TRANSPARENT
                    } else {
                        rgba(0x0000008c)
                    })
                    .occlude()
                    .on_click(move |event, window, cx| {
                        if !busy {
                            if let Some(close) = &close {
                                close(event, window, cx);
                            }
                        }
                    }),
            )
            .child(content)
    }
}
