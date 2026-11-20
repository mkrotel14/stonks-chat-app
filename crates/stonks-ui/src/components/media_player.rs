//! Shared attachment-player surfaces and explicit media poster control.
use crate::{icons::Icon, Theme};
use gpui_kit::base::Button as NativeButton;
use gpui_kit::*;
use std::{rc::Rc, sync::Arc};
pub fn audio_surface(theme: &Theme, width: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .w(px(width.clamp(1., 520.)))
        .max_w_full()
        .min_w_0()
        .p(px(theme.space3))
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_md))
        .bg(theme.surface_raised)
        .text_color(theme.text)
        .line_height(relative(1.5))
}
pub fn file_header(
    theme: &Theme,
    icon: Icon,
    name: &str,
    detail: &str,
    trailing: impl IntoElement,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_w_0()
        .child(
            div()
                .size(px(32.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radius_sm))
                .bg(theme.surface_selected)
                .child(icon.render(theme.accent, 20.)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_1()
                .gap(px(2.))
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(theme.font_size_sm))
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(theme.font_size_xs))
                        .text_color(theme.text_muted)
                        .child(detail.to_owned()),
                ),
        )
        .child(trailing)
}
pub fn video_placeholder(theme: &Theme, width: f32) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .min_h(px(86.))
        .w(px(width.clamp(1., 520.)))
        .max_w_full()
        .min_w_0()
        .gap(px(theme.space3))
        .p(px(14.))
        .rounded(px(theme.radius_md))
        .bg(theme.surface_raised)
        .text_color(theme.text)
        .line_height(relative(1.5))
}
/// Attachment video keeps natural aspect/intrinsic bounds; iframe-style embeds fill16:9.
pub fn video_size(width: f32, dimensions: Option<(u64, u64)>, embedded: bool) -> (f32, f32) {
    let available = width.clamp(1., 520.);
    if embedded {
        return (available, available * 9. / 16.);
    }
    if let Some((w, h)) = dimensions.filter(|(w, h)| *w > 0 && *h > 0) {
        let scale = (available / w as f32).min(390. / h as f32).min(1.);
        return ((w as f32 * scale).max(1.), (h as f32 * scale).max(1.));
    }
    (available, (available * 9. / 16.).min(390.))
}
#[derive(IntoElement)]
pub struct UiMediaPoster {
    id: ElementId,
    theme: Theme,
    width: f32,
    image: Option<Arc<RenderImage>>,
    label: SharedString,
    icon: Icon,
    disabled: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}
impl UiMediaPoster {
    pub fn new(
        id: impl Into<ElementId>,
        theme: Theme,
        width: f32,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            width,
            image: None,
            label: label.into(),
            icon: Icon::Play,
            disabled: false,
            on_click: None,
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }
    pub fn image(mut self, image: Option<Arc<RenderImage>>) -> Self {
        self.image = image;
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for UiMediaPoster {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let width = self.width.max(1.);
        let height = width * 9. / 16.;
        let mut button = NativeButton::new(self.id)
            .accessibility_label(self.label)
            .disabled(self.disabled)
            .relative()
            .w(px(width))
            .h(px(height))
            .max_w_full()
            .p_0()
            .overflow_hidden()
            .rounded(px(theme.radius_sm))
            .bg(theme.canvas)
            .cursor_pointer()
            .focus_visible(move |style| {
                style.shadow(vec![BoxShadow {
                    color: theme.focus_ring.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(3.),
                    inset: false,
                }])
            });
        if let Some(image) = self.image {
            button = button.child(
                img(image)
                    .id("media-poster-image")
                    .size_full()
                    .rounded(px(theme.radius_sm))
                    .object_fit(ObjectFit::Cover),
            );
        }
        let shadow = BoxShadow {
            color: theme.shadow_sm.color.into(),
            offset: point(px(0.), px(theme.shadow_sm.dy)),
            blur_radius: px(theme.shadow_sm.blur),
            spread_radius: px(0.),
            inset: false,
        };
        button = button.child(
            div()
                .absolute()
                .left(px((width - 58.) / 2.))
                .top(px((height - 42.) / 2.))
                .w(px(58.))
                .h(px(42.))
                .rounded(px(theme.radius_md))
                .bg(theme.accent)
                .text_color(theme.accent_text)
                .flex()
                .items_center()
                .justify_center()
                .shadow(vec![shadow])
                .child(self.icon.render(theme.accent_text, 24.)),
        );
        if let Some(callback) = self.on_click {
            button = button.on_click(move |event, window, cx| {
                cx.stop_propagation();
                callback(event, window, cx);
            });
        }
        if self.disabled {
            button = button.opacity(0.5);
        }
        button
    }
}
#[cfg(test)]
mod tests {
    use super::video_size;
    #[test]
    fn video_geometry_preserves_intrinsic_aspect_and_source_caps() {
        assert_eq!(video_size(520., Some((1920, 1080)), false), (520., 292.5));
        assert_eq!(video_size(500., Some((100, 1000)), false), (39., 390.));
        assert_eq!(video_size(100., Some((1920, 1080)), false), (100., 56.25));
        assert_eq!(video_size(500., Some((200, 100)), false), (200., 100.));
        assert_eq!(video_size(500., None, true), (500., 281.25));
    }
}

/// Source UiLink geometry using native keyboard activation and link semantics.
pub fn media_link(
    theme: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    url: String,
) -> NativeButton {
    let label = label.into();
    let focus = theme.focus;
    NativeButton::new(id)
        .role(Role::Link)
        .accessibility_label(label.clone())
        .p_0()
        .min_w_0()
        .justify_start()
        .text_color(theme.link)
        .text_size(px(theme.font_size_md))
        .line_height(relative(1.5))
        .cursor_pointer()
        .focus_visible(move |style| {
            style.shadow(vec![BoxShadow {
                color: focus.into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            crate::external_links::request(&url, window, cx);
        })
        .child(div().min_w_0().flex_1().child(label))
}
