//! Visual geometry fixture for the shared native media controls.
//! `cargo run --manifest-path crates/stonks-app/Cargo.toml --example media`
use gpui_kit::*;
use std::sync::Arc;
use stonks_ui::{
    components::{avatar, image_attachment},
    fonts, Theme,
};

#[path = "../src/shared/images/decode.rs"]
mod decode;

struct Preview {
    image: Arc<RenderImage>,
    avatar: Arc<RenderImage>,
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let available = (f32::from(window.viewport_size().width) / 2. - 32.).max(1.);
        div()
            .size_full()
            .flex()
            .font_family(fonts::FAMILY)
            .children([Theme::dark(), Theme::light()].into_iter().enumerate().map(
                |(index, theme)| {
                    div()
                        .id(ElementId::Name(format!("theme-{index}").into()))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .p(px(theme.space4))
                        .gap(px(theme.space3))
                        .bg(theme.surface)
                        .text_color(theme.text)
                        .child("Avatar: loaded / loading or failed")
                        .child(
                            div()
                                .flex()
                                .gap(px(theme.space3))
                                .child(avatar(&theme, "João", 36., Some(self.avatar.clone())))
                                .child(avatar(&theme, "João", 36., None)),
                        )
                        .child("Wide image")
                        .child(image_attachment(
                            &theme,
                            "wide.png",
                            Some(self.image.clone()),
                            (Some(640), Some(320)),
                            available,
                        ))
                        .child("Narrow image / fallback")
                        .child(
                            div()
                                .flex()
                                .gap(px(theme.space2))
                                .child(image_attachment(
                                    &theme,
                                    "narrow.png",
                                    Some(self.image.clone()),
                                    (Some(100), Some(200)),
                                    available.min(100.),
                                ))
                                .child(image_attachment(
                                    &theme,
                                    "unavailable.png",
                                    None,
                                    (Some(100), Some(200)),
                                    available.min(100.),
                                )),
                        )
                },
            ))
    }
}
fn main() {
    let pixels = image::RgbaImage::from_fn(640, 320, |x, y| {
        // BGRA: red/blue halves plus a white grid make channel swaps,
        // aspect distortion and cover/contain behavior easy to spot.
        image::Rgba(if x % 80 < 2 || y % 80 < 2 {
            [255, 255, 255, 255]
        } else if x < 320 {
            [0, 0, 255, 255]
        } else {
            [255, 0, 0, 255]
        })
    });
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
    // Optional local files make real failed previews/animated avatars
    // reviewable without embedding account data in the repository.
    let mut args = std::env::args().skip(1);
    let load = |path: String, edge| {
        Arc::new(RenderImage::new(
            decode::decode(&std::fs::read(path).expect("fixture file"), edge)
                .expect("fixture decode")
                .frames,
        ))
    };
    let image = args.next().map(|path| load(path, 1280)).unwrap_or(image);
    let avatar_image = args
        .next()
        .map(|path| load(path, 80))
        .unwrap_or_else(|| image.clone());
    gpui_kit::application().run(move |cx| {
        fonts::register(cx);
        cx.open_window(
            WindowOptions {
                app_id: Some("stonks-media-preview".into()),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Preview {
                    image,
                    avatar: avatar_image,
                })
            },
        )
        .expect("media preview window");
        cx.activate(true);
    });
}
