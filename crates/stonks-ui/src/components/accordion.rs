//! Shared settings disclosure, matching UiAccordion's card/header/content.
use crate::{
    components::{Button, ButtonVariant},
    icons::Icon,
    Theme,
};
use gpui_kit::*;
use std::rc::Rc;
#[derive(IntoElement)]
pub struct UiAccordion {
    id: ElementId,
    theme: Theme,
    title: SharedString,
    subtitle: SharedString,
    open: bool,
    focus: Option<FocusHandle>,
    leading: Option<AnyElement>,
    content: Option<AnyElement>,
    on_toggle: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}
impl UiAccordion {
    pub fn new(id: impl Into<ElementId>, theme: Theme, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            theme,
            title: title.into(),
            subtitle: SharedString::default(),
            open: false,
            focus: None,
            leading: None,
            content: None,
            on_toggle: None,
        }
    }
    pub fn subtitle(mut self, value: impl Into<SharedString>) -> Self {
        self.subtitle = value.into();
        self
    }
    pub fn open(mut self, value: bool) -> Self {
        self.open = value;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn leading(mut self, value: impl IntoElement) -> Self {
        self.leading = Some(value.into_any_element());
        self
    }
    pub fn child(mut self, value: impl IntoElement) -> Self {
        self.content = Some(value.into_any_element());
        self
    }
    pub fn on_toggle(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for UiAccordion {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let mut header = div().flex().w_full().items_center().gap(px(18.));
        if let Some(leading) = self.leading {
            header = header.child(leading);
        }
        header = header
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(theme.space1))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(theme.font_size_md))
                            .font_weight(FontWeight(750.))
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(theme.font_size_xs))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme.text_muted)
                            .child(self.subtitle),
                    ),
            )
            .child(
                (if self.open {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                })
                .render_stroke(theme.text_muted, 16., 1.7),
            );
        let mut trigger = Button::new(self.id, theme)
            .variant(ButtonVariant::Accordion)
            .expanded(self.open)
            .tooltip(self.title)
            .leading(header);
        if let Some(focus) = self.focus {
            trigger = trigger.track_focus(&focus);
        }
        if let Some(callback) = self.on_toggle {
            trigger = trigger.on_click(move |event, window, cx| callback(event, window, cx));
        }
        let mut card = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w_0()
            .overflow_hidden()
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_md))
            .bg(theme.surface)
            .text_color(theme.text)
            .child(trigger);
        if self.open {
            if let Some(content) = self.content {
                card = card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(7.))
                        .mx(px(theme.space2))
                        .mb(px(theme.space2))
                        .p(px(8.))
                        .rounded(px(theme.radius_sm))
                        .bg(theme.surface_raised)
                        .child(content),
                );
            }
        }
        card
    }
}
