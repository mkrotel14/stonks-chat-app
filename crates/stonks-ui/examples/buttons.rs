//! Shared navigation/button geometry and keyboard activation fixture.
use gpui_kit::*;
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    icons::Icon,
    Theme,
};
struct Preview {
    clicks: usize,
    focus_ready: bool,
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus_ready {
            self.focus_ready = true;
            window.on_next_frame(|window, _| {
                window.on_next_frame(|window, cx| window.focus_next(cx))
            });
        }
        div()
            .size_full()
            .flex()
            .font_family(fonts::FAMILY)
            .children([Theme::dark(), Theme::light()].into_iter().enumerate().map(
                |(index, theme)| {
                    div()
                        .id(ElementId::Name(format!("theme-{index}").into()))
                        .flex_1()
                        .h_full()
                        .flex()
                        .flex_col()
                        .gap(px(theme.space3))
                        .p(px(theme.space4))
                        .bg(theme.surface)
                        .text_color(theme.text)
                        .child(format!("Activations: {}", self.clicks))
                        .children(
                            [
                                ButtonVariant::Rail,
                                ButtonVariant::Channel,
                                ButtonVariant::Heading,
                                ButtonVariant::Icon,
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(n, variant)| {
                                Button::new(ElementId::Name(format!("button-{n}").into()), theme)
                                    .variant(variant)
                                    .leading(Icon::MessageCircle.render(theme.text, 18.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clicks += 1;
                                        cx.notify();
                                    }))
                            }),
                        )
                        .child(
                            Button::new("disabled", theme)
                                .label("Disabled")
                                .disabled(true)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clicks += 100;
                                    cx.notify();
                                })),
                        )
                },
            ))
    }
}
fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        fonts::register(cx);
        gpui_kit::open_window(
            WindowOptions {
                app_id: Some("stonks-buttons-preview".into()),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|_| Preview {
                    clicks: 0,
                    focus_ready: false,
                })
            },
        )
        .expect("button preview");
        cx.activate(true);
    });
}
