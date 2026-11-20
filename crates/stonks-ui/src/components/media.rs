//! Shared native avatar and image-preview geometry, using semantic tokens.
use crate::Theme;
use gpui_kit::{
    div, img, px, Div, FontWeight, InteractiveElement, ObjectFit, ParentElement, RenderImage,
    Styled, StyledImage,
};
use std::sync::Arc;

/// Source UiMediaSpoiler cover. The caller must omit concealed children so
/// their image/network/player lifecycle does not start before reveal.
pub fn concealed_media(
    id: impl Into<gpui_kit::ElementId>,
    theme: &Theme,
    on_reveal: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Div {
    div()
        .w_full()
        .min_w(px(160.))
        .max_w_full()
        .min_h(px(100.))
        .p(px(theme.space4))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius_md))
        .bg(theme.surface_interactive)
        .child(
            super::Button::new(id, *theme)
                .variant(super::ButtonVariant::Secondary)
                .label(crate::i18n::t("messages.revealAttachment"))
                .on_click(on_reveal),
        )
}

/// Image activation with the native button's Enter/Space and focus behavior.
/// Geometry remains owned by the supplied shared image preview.
pub fn image_opener(
    id: impl Into<gpui_kit::ElementId>,
    theme: &Theme,
    label: impl Into<gpui_kit::SharedString>,
) -> gpui_kit::base::Button {
    let ring = theme.focus_ring;
    gpui_kit::base::Button::new(id)
        .accessibility_label(label)
        .p_0()
        .min_w_0()
        .cursor_pointer()
        .focus_visible(move |style| {
            style.shadow(vec![gpui_kit::BoxShadow {
                color: ring.into(),
                offset: gpui_kit::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(3.),
                inset: false,
            }])
        })
}

pub fn avatar(theme: &Theme, name: &str, size: f32, image: Option<Arc<RenderImage>>) -> Div {
    let mut root = div()
        .flex()
        .flex_shrink_0()
        .size(px(size))
        .items_center()
        .justify_center()
        .rounded(px(theme.radius_pill))
        .overflow_hidden()
        .bg(theme.surface_interactive)
        .text_color(theme.text)
        .text_size(px(theme.font_size_sm))
        .font_weight(FontWeight::BOLD);
    root = if let Some(image) = image {
        root.child(
            img(image)
                .id("avatar-image")
                .size_full()
                .rounded(px(theme.radius_pill))
                .object_fit(ObjectFit::Cover),
        )
    } else {
        root.child(
            name.chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .collect::<String>(),
        )
    };
    root
}

pub fn presence_avatar(
    theme: &Theme,
    name: &str,
    online: bool,
    presence: Option<&str>,
    image: Option<Arc<RenderImage>>,
) -> Div {
    let presence = presence.unwrap_or_default().to_ascii_lowercase();
    let color = if !online || matches!(presence.as_str(), "offline" | "invisible") {
        theme.text_muted
    } else {
        match presence.as_str() {
            "idle" | "away" => theme.warning,
            "busy" => theme.danger,
            "focus" => theme.focus,
            _ => theme.success,
        }
    };
    div()
        .relative()
        .flex_shrink_0()
        .size(px(40.))
        .child(avatar(theme, name, 40., image))
        .child(
            div()
                .absolute()
                .right(px(-2.))
                .bottom(px(-2.))
                .size(px(theme.space4))
                .rounded(px(theme.radius_pill))
                .border_3()
                .border_color(theme.surface)
                .bg(color),
        )
}

/// Reserve the same bounds before and after loading. The caller determines
/// available width; metadata only controls the aspect ratio within those bounds.
pub fn image_attachment(
    theme: &Theme,
    filename: &str,
    image: Option<Arc<RenderImage>>,
    dimensions: (Option<u64>, Option<u64>),
    available_width: f32,
) -> Div {
    let (width, height) = preview_size(dimensions, available_width);
    let root = div()
        .flex_shrink_0()
        .w(px(width))
        .h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius_sm))
        .overflow_hidden()
        .bg(theme.canvas)
        .text_color(theme.text_muted)
        .text_size(px(theme.font_size_sm));
    match image {
        Some(image) => root.child(
            img(image.clone())
                .id(("attachment-image", image.id.0))
                .size_full()
                .rounded(px(theme.radius_sm))
                .object_fit(ObjectFit::Contain),
        ),
        None => root.child(
            div()
                .min_w_0()
                .p(px(theme.space2))
                .child(filename.to_owned()),
        ),
    }
}

fn preview_size(dimensions: (Option<u64>, Option<u64>), available: f32) -> (f32, f32) {
    let available = available.clamp(1., 520.);
    match dimensions {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let scale = (available / width as f32).min(390. / height as f32).min(1.);
            (
                (width as f32 * scale).max(1.),
                (height as f32 * scale).max(1.),
            )
        }
        _ => (available, 180.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_geometry_preserves_aspect_and_fits_narrow_panes() {
        assert_eq!(preview_size((Some(1920), Some(1080)), 480.), (480., 270.));
        assert_eq!(preview_size((Some(100), Some(1000)), 480.), (39., 390.));
        assert_eq!(preview_size((Some(1920), Some(1080)), 160.), (160., 90.));
        assert_eq!(preview_size((Some(0), Some(0)), 160.), (160., 180.));
    }
}
