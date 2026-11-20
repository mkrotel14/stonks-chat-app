//! Source fenced-code surface: language/copy header and a bounded preformatted
//! viewport. Code remains plain text; highlighted spans never become markup.
use super::{highlight_code, CodeToken, RenderColors};
use crate::{
    components::{Button, ButtonVariant},
    i18n::t,
    Theme,
};
use gpui_kit::base::{Scrollbar, ScrollbarMode};
use gpui_kit::*;

pub(super) const FONT_SIZE: f32 = 12.;
pub(super) const MAX_HEIGHT: f32 = 420.;

pub(super) fn render(id: &str, language: &str, code: &str, theme: &Theme) -> Div {
    let colors = RenderColors::from_theme(theme);
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (part, kind) in highlight_code(code, language) {
        let start = text.len();
        text.push_str(&part);
        let color = match kind {
            CodeToken::Plain => colors.text,
            CodeToken::Comment => colors.muted,
            CodeToken::String => theme.success.into(),
            CodeToken::Keyword => colors.link,
            CodeToken::Number => theme.warning.into(),
        };
        highlights.push((
            start..text.len(),
            HighlightStyle {
                color: Some(color),
                ..Default::default()
            },
        ));
    }
    let copied = code.to_owned();
    div()
        .w_full()
        .max_w(px(720.))
        .my(px(7.))
        .overflow_hidden()
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_sm))
        .bg(theme.canvas)
        .child(
            div()
                .flex()
                .min_h(px(34.))
                .items_center()
                .justify_between()
                .pl(px(11.))
                .pr(px(8.))
                .border_b_1()
                .border_color(theme.outline)
                .bg(theme.surface_raised)
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_xs))
                .child(
                    if language.is_empty() {
                        t("messages.code")
                    } else {
                        language.to_owned()
                    }
                    .to_uppercase(),
                )
                .child(
                    Button::new(ElementId::Name(format!("{id}-copy-code").into()), *theme)
                        .variant(ButtonVariant::Quiet)
                        .label(t("messages.copy"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                        }),
                ),
        )
        .child(CodeViewport {
            id: id.to_owned(),
            text,
            highlights,
            theme: *theme,
        })
}

#[derive(IntoElement)]
struct CodeViewport {
    id: String,
    text: String,
    highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>,
    theme: Theme,
}
impl RenderOnce for CodeViewport {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ElementId::Name(format!("{}-code-scroll-state", self.id).into()),
            cx,
            |_, _| ScrollHandle::new(),
        );
        let scroll = state.read(cx).clone();
        div()
            .relative()
            .max_h(px(MAX_HEIGHT))
            .min_w_0()
            .child(
                div()
                    .id(ElementId::Name(format!("{}-code-scroll", self.id).into()))
                    .max_h(px(MAX_HEIGHT))
                    .min_w_0()
                    .overflow_scroll()
                    .track_scroll(&scroll)
                    .py(px(11.))
                    .px(px(13.))
                    .font_family("monospace")
                    .text_size(px(FONT_SIZE))
                    .line_height(relative(1.55))
                    .whitespace_nowrap()
                    .child(StyledText::new(self.text).with_highlights(self.highlights)),
            )
            .child(
                Scrollbar::new(&scroll)
                    .mode(ScrollbarMode::Hover)
                    .styles(|style| style.thumb(|thumb| thumb.bg(self.theme.outline))),
            )
    }
}
