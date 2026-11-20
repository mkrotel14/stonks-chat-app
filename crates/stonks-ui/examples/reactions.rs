//! Offline Tauri-sized reaction artwork, including tone/ZWJ/custom fallback.
use gpui_kit::*;
use stonks_ui::{
    components::{
        emoji::{reaction_tooltip, ReactionPerson},
        EmojiGlyph, ReactionPill,
    },
    fonts, Theme,
};
struct Preview;
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .font_family(fonts::FAMILY)
            .children([Theme::dark(), Theme::light()].into_iter().enumerate().map(
                |(theme_index, theme)| {
                    let people = vec![
                        ReactionPerson {
                            name: "Olavo de Carvalho".into(),
                            avatar: None,
                        },
                        ReactionPerson {
                            name: "You".into(),
                            avatar: None,
                        },
                    ];
                    div()
                        .flex_1()
                        .h_full()
                        .p(px(24.))
                        .bg(theme.surface)
                        .text_color(theme.text)
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child("Tauri reaction artwork: 18px")
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(4.))
                                .children(
                                    ["😺", "💀", "👍🏽", "👨‍👩‍👧‍👦", "🇧🇷", "❤️"]
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, emoji)| {
                                            ReactionPill::new(
                                                ElementId::Name(
                                                    format!("reaction-{theme_index}-{index}")
                                                        .into(),
                                                ),
                                                theme,
                                                emoji.into(),
                                                index + 1,
                                            )
                                            .selected(index == 1)
                                            .people(people.clone())
                                            .on_click(|_, _, _| {})
                                        }),
                                ),
                        )
                        .child(reaction_tooltip(&theme, ":skull:", &people))
                        .child("Same artwork at 32px for inspection")
                        .child(div().flex().gap(px(12.)).children(
                            ["😺", "💀", "👍🏽", "🇧🇷"].into_iter().enumerate().map(
                                |(index, emoji)| {
                                    EmojiGlyph::new(
                                        ElementId::Name(
                                            format!("large-{theme_index}-{index}").into(),
                                        ),
                                        emoji,
                                    )
                                    .size(32.)
                                },
                            ),
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
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-reactions-preview".into()),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1000.), px(620.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| Preview),
            )
            .expect("reaction fixture window");
            cx.activate(true);
        });
}
