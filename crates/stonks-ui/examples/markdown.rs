//! Local visual fixture: `cargo run --manifest-path crates/stonks-ui/Cargo.toml --example markdown`.
//! Compare both themes at two widths; hover spoilers and click links.
use std::{collections::HashSet, rc::Rc};

use gpui_kit::*;
use stonks_ui::{
    fonts,
    markdown::{self, MentionContext, SpoilerHover},
    Theme,
};

struct Preview {
    revealed: HashSet<String>,
    code_only: bool,
    compact: bool,
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let hover: SpoilerHover = Rc::new(move |key, visible, cx| {
            view.update(cx, |this, cx| {
                let changed = if visible {
                    this.revealed.insert(key.to_owned())
                } else {
                    this.revealed.remove(key)
                };
                if changed {
                    cx.notify();
                }
            })
            .ok();
        });
        let id = "01M0P54M56DA883E6ZYDK00BKC";
        let users = vec![(id.to_owned(), "João 🦀".into())];
        let channels = vec![(id.to_owned(), "general".into())];
        let mentions = MentionContext {
            users: &users,
            channels: &channels,
            roles: &[],
        };
        let content = format!("# Message formatting\n\nOlá <@{id}> in <#{id}>\n**Bold _and italic_**, ~~removed~~ and `inline code`.\nA long sentence that wraps naturally within the available column width.\n\n[Open example](https://example.com) · ||hidden spoiler||\n\n> A quote with **emphasis**\n\n- first item\n- second item\n\n```rust\nlet answer = 42; // comment\n```\n\n| Name | Value |\n| --- | --- |\n| Count | **42** |\n\n<t:1700000000:F>\n:custom-emoji:");
        let content = if self.code_only {
            let code = (0..60)
                .map(|index| {
                    format!(
                        "let row_{index} = \"{}\"; // line {index}",
                        "long code value ".repeat(7)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("# Code and table geometry\n\n```rust\n{code}\n```\n\n> Accent quote, 3px rule and 11px inset.\n\n| Name | Long value |\n| --- | --- |\n| Count | {} |\n| Plain | 42 |", "unbroken-key-".repeat(15))
        } else {
            content
        };
        let compact = self.compact;
        div()
            .size_full()
            .flex()
            .font_family(fonts::FAMILY)
            .children([Theme::dark(), Theme::light()].into_iter().enumerate().map(
                |(index, theme)| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(theme.canvas)
                        .text_color(theme.text)
                        .p(px(theme.space3))
                        .flex()
                        .gap(px(theme.space3))
                        .children([240., 420.].into_iter().enumerate().map(
                            |(width_index, width)| {
                                div()
                                    .id(ElementId::Name(
                                        format!("column.{index}.{width_index}").into(),
                                    ))
                                    .w(px(width))
                                    .flex_shrink_0()
                                    .h_full()
                                    .overflow_y_scroll()
                                    .text_size(px(theme.font_size_sm))
                                    .child(markdown::render_with_options(
                                        &content,
                                        &format!("preview.{index}.{width_index}"),
                                        &theme,
                                        &mentions,
                                        &self.revealed,
                                        hover.clone(),
                                        markdown::InlineActions::default(),
                                        markdown::InlineMedia::default(),
                                        markdown::RenderOptions { compact },
                                    ))
                            },
                        ))
                },
            ))
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            cx.open_window(
                WindowOptions {
                    app_id: Some("stonks-markdown-preview".into()),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1400.), px(800.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Preview {
                        revealed: HashSet::new(),
                        code_only: std::env::args().any(|arg| arg == "--code"),
                        compact: std::env::args().any(|arg| arg == "--compact"),
                    })
                },
            )
            .expect("preview window");
            cx.activate(true);
        });
}
