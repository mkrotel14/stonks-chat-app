//! Shared quote/table geometry from FormattedMessage.stylex.ts.
use crate::Theme;
use gpui_kit::base::{Scrollbar, ScrollbarMode};
use gpui_kit::*;
use std::rc::Rc;

pub(super) fn quote(theme: &Theme, content: impl IntoElement) -> Div {
    div()
        .my(px(5.))
        .py(px(2.))
        .pl(px(11.))
        .border_l(px(3.))
        .border_color(theme.accent)
        .text_color(theme.text_muted)
        .child(content)
}

#[derive(IntoElement)]
pub(super) struct Table {
    pub id: String,
    pub theme: Theme,
    pub headers: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
}
pub(super) type Cell = Rc<dyn Fn() -> AnyElement>;
impl RenderOnce for Table {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let columns = self.headers.len().max(1);
        let row_count = self.rows.len() + 1;
        let theme = self.theme;
        let mut widths = vec![0_f32; columns];
        let mut rows = Vec::new();
        for (index, cells) in std::iter::once(self.headers).chain(self.rows).enumerate() {
            let mut prepared = Vec::new();
            let mut cells = cells.into_iter();
            for (column, width) in widths.iter_mut().enumerate() {
                let content = cells
                    .next()
                    .unwrap_or_else(|| Rc::new(|| div().into_any_element()));
                let cell = || {
                    let mut cell = div()
                        .h_full()
                        .px(px(10.))
                        .py(px(8.))
                        .border_r(px(if column + 1 < columns { 1. } else { 0. }))
                        .border_b(px(if index + 1 < row_count { 1. } else { 0. }))
                        .border_color(theme.outline)
                        .text_size(px(theme.font_size_sm))
                        .line_height(relative(1.5))
                        .child(content());
                    if index == 0 {
                        cell = cell
                            .font_weight(FontWeight::BOLD)
                            .bg(theme.surface_raised)
                            .text_color(theme.text);
                    }
                    cell.into_any_element()
                };
                let natural = cell().layout_as_root(
                    size(AvailableSpace::MaxContent, AvailableSpace::MinContent),
                    window,
                    cx,
                );
                *width = width.max(f32::from(natural.width));
                prepared.push(cell());
            }
            rows.push(prepared);
        }
        let total = widths.iter().sum::<f32>();
        let table =
            div()
                .flex()
                .flex_col()
                .min_w_full()
                .w(px(total))
                .children(rows.into_iter().map(|cells| {
                    div()
                        .flex()
                        .w_full()
                        .children(cells.into_iter().zip(&widths).map(|(cell, width)| {
                            div()
                                .flex_1()
                                .flex_basis(px(*width))
                                .min_w(px(*width))
                                .child(cell)
                        }))
                }));
        let state = window.use_keyed_state(
            ElementId::Name(format!("{}-table-state", self.id).into()),
            cx,
            |_, _| ScrollHandle::new(),
        );
        let scroll = state.read(cx).clone();
        div()
            .id(ElementId::Name(format!("{}-table-scroll", self.id).into()))
            .relative()
            .max_w_full()
            .min_w_0()
            .my(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_sm))
            .text_size(px(theme.font_size_sm))
            .child(
                div()
                    .id(ElementId::Name(
                        format!("{}-table-viewport", self.id).into(),
                    ))
                    .min_w_0()
                    .max_w_full()
                    .overflow_x_scroll()
                    .track_scroll(&scroll)
                    .child(table),
            )
            .child(
                Scrollbar::new(&scroll)
                    .mode(ScrollbarMode::Hover)
                    .styles(|style| style.thumb(|thumb| thumb.bg(theme.outline))),
            )
    }
}
