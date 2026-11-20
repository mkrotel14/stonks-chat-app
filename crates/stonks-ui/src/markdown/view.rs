//! Shared native formatted-message surface. Feature code supplies only
//! mention resolution and spoiler state; geometry and styling live here.

use std::{collections::HashSet, rc::Rc};

use gpui_kit::*;

use super::*;

pub type SpoilerHover = Rc<dyn Fn(&str, bool, &mut App)>;
pub type InlineAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
#[derive(Clone, Default)]
pub struct InlineActions {
    pub user: Option<InlineAction>,
    pub channel: Option<InlineAction>,
}
#[derive(Clone, Copy, Default)]
pub struct RenderOptions {
    pub compact: bool,
}

pub fn render(
    content: &str,
    id: &str,
    theme: &Theme,
    mentions: &MentionContext<'_>,
    revealed: &HashSet<String>,
    on_spoiler_hover: SpoilerHover,
) -> Div {
    render_with_actions(
        content,
        id,
        theme,
        mentions,
        revealed,
        on_spoiler_hover,
        InlineActions::default(),
    )
}
pub fn render_with_actions(
    content: &str,
    id: &str,
    theme: &Theme,
    mentions: &MentionContext<'_>,
    revealed: &HashSet<String>,
    on_spoiler_hover: SpoilerHover,
    actions: InlineActions,
) -> Div {
    render_with_media(
        content,
        id,
        theme,
        mentions,
        revealed,
        on_spoiler_hover,
        actions,
        InlineMedia::default(),
    )
}
pub fn render_with_media(
    content: &str,
    id: &str,
    theme: &Theme,
    mentions: &MentionContext<'_>,
    revealed: &HashSet<String>,
    on_spoiler_hover: SpoilerHover,
    actions: InlineActions,
    media: InlineMedia,
) -> Div {
    render_with_options(
        content,
        id,
        theme,
        mentions,
        revealed,
        on_spoiler_hover,
        actions,
        media,
        RenderOptions::default(),
    )
}
pub fn render_with_options(
    content: &str,
    id: &str,
    theme: &Theme,
    mentions: &MentionContext<'_>,
    revealed: &HashSet<String>,
    on_spoiler_hover: SpoilerHover,
    actions: InlineActions,
    media: InlineMedia,
    options: RenderOptions,
) -> Div {
    let colors = RenderColors::from_theme(theme);
    let inline = |text: &str, key: String| {
        let line = styled_line(&tokenize_inline(text), &colors, mentions, &key, |key| {
            revealed.contains(key)
        });
        inline_images::InlineText {
            theme: *theme,
            line,
            key,
            hover: on_spoiler_hover.clone(),
            actions: actions.clone(),
            media: media.clone(),
        }
    };
    let mut root = div()
        .flex()
        .flex_col()
        .min_w_0()
        .text_size(px(15.5))
        .line_height(relative(if options.compact { 1.35 } else { 1.5 }))
        .mt(px(if options.compact { 0. } else { 2. }));
    let blocks = parse_blocks(content);
    let count = blocks.len();
    for (index, block) in blocks.into_iter().enumerate() {
        let key = format!("{id}.{index}");
        let element = match block {
            Block::Paragraph(lines) => div()
                .mb(px(if index + 1 == count { 0. } else { 15.5 * 0.45 }))
                .child(inline(&lines.join("\n"), key)),
            Block::Quote(lines) => blocks::quote(theme, inline(&lines.join("\n"), key)),
            Block::Heading { level, text } => div()
                .mt(px(15.5 * 0.35))
                .mb(px(15.5 * 0.15))
                .line_height(relative(1.25))
                .font_weight(FontWeight::BOLD)
                .text_size(px(match level {
                    1 => 15.5 * 1.45,
                    2 => 15.5 * 1.25,
                    _ => 15.5 * 1.08,
                }))
                .child(inline(&text, key)),
            Block::List { ordered, items } => div()
                .flex()
                .my(px(5.))
                .pl(px(15.5 * 1.4))
                .flex_col()
                .children(items.iter().enumerate().map(|(index, text)| {
                    div()
                        .flex()
                        .gap(px(theme.space2))
                        .child(if ordered {
                            format!("{}.", index + 1)
                        } else {
                            "•".into()
                        })
                        .child(
                            div()
                                .min_w_0()
                                .child(inline(text, format!("{key}.{index}"))),
                        )
                })),
            Block::Code { language, code } => code::render(&key, &language, &code, theme),
            Block::Table { headers, rows } => {
                let columns = headers.len().max(1);
                let headers = (0..columns)
                    .map(|column| {
                        let cell = inline(
                            headers.get(column).map(String::as_str).unwrap_or_default(),
                            format!("{key}.0.{column}"),
                        );
                        let factory: blocks::Cell =
                            Rc::new(move || cell.clone().into_any_element());
                        factory
                    })
                    .collect();
                let rows = rows
                    .iter()
                    .enumerate()
                    .map(|(row, cells)| {
                        (0..columns)
                            .map(|column| {
                                let cell = inline(
                                    cells.get(column).map(String::as_str).unwrap_or_default(),
                                    format!("{key}.{}.{column}", row + 1),
                                );
                                let factory: blocks::Cell =
                                    Rc::new(move || cell.clone().into_any_element());
                                factory
                            })
                            .collect()
                    })
                    .collect();
                div().child(blocks::Table {
                    id: key,
                    theme: *theme,
                    headers,
                    rows,
                })
            }
        };
        root = root.child(element);
    }
    root
}
