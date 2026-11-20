//! Native metadata card geometry, ported from MessageMedia.stylex.ts.
//! Playback belongs to the media viewer; the card offers an explicit link.
use std::sync::Arc;

use gpui_kit::{
    div, img, px, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, RenderImage, Styled, StyledImage,
};

use crate::{components::media_player::media_link, i18n::t, Theme};

pub struct CardContent {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: Option<String>,
    pub target: Option<String>,
}

/// Fixed poster bounds keep loading/failure transitions from shifting rows.
/// Each card has its own parent ID so reused animated images retain local state.
pub fn embed_card(
    theme: &Theme,
    id: &str,
    content: CardContent,
    poster: Option<Arc<RenderImage>>,
    icon: Option<Arc<RenderImage>>,
    width: f32,
    has_poster: bool,
) -> Div {
    let content_width = (width.clamp(1., 530.) - 32.).max(1.);
    let target = content.target.clone();
    let mut card = embed_card_header(theme, id, content, icon, width);
    if has_poster {
        let mut media = div()
            .w_full()
            .h(px((content_width * 9. / 16.).min(330.)))
            .flex_shrink_0()
            .mt(px(theme.space3))
            .rounded(px(theme.radius_sm))
            .overflow_hidden()
            .bg(theme.canvas);
        if let Some(poster) = poster {
            media = media.child(
                img(poster)
                    .id(ElementId::Name(format!("{id}-poster").into()))
                    .size_full()
                    .rounded(px(theme.radius_sm))
                    .object_fit(ObjectFit::Cover),
            );
        }
        card = card.child(media);
    }
    if let Some(target) = target {
        card = card.child(div().mt(px(theme.space2)).child(media_link(
            theme,
            ElementId::Name(format!("{id}-open").into()),
            format!("{} ↗", t("messages.openLink")),
            target,
        )));
    }
    card
}

pub fn embed_card_header(
    theme: &Theme,
    id: &str,
    content: CardContent,
    icon: Option<Arc<RenderImage>>,
    width: f32,
) -> Div {
    let width = width.clamp(1., 530.);
    let mut card = div()
        .w(px(width))
        .max_w_full()
        .min_w_0()
        .mt(px(10.))
        .px(px(14.))
        .py(px(13.))
        .border_l_4()
        .border_color(theme.accent)
        .rounded(px(theme.radius_sm))
        .overflow_hidden()
        .bg(theme.surface_raised)
        .text_color(theme.text)
        .flex()
        .flex_col();
    if content.site.is_some() || icon.is_some() {
        let mut site = div()
            .flex()
            .items_center()
            .gap(px(7.))
            .mb(px(6.))
            .text_color(theme.text_muted)
            .text_size(px(theme.font_size_xs));
        if let Some(icon) = icon {
            site = site.child(
                img(icon)
                    .id(ElementId::Name(format!("{id}-site-icon").into()))
                    .size(px(16.))
                    .rounded(px(theme.space1))
                    .object_fit(ObjectFit::Cover),
            );
        }
        card = card.child(site.child(content.site.unwrap_or_else(|| t("messages.linkPreview"))));
    }
    if let Some(title) = content.title {
        card = card.child(if let Some(target) = content.target.clone() {
            media_link(
                theme,
                ElementId::Name(format!("{id}-title").into()),
                title,
                target,
            )
            .font_weight(FontWeight(750.))
            .line_height(gpui_kit::relative(1.35))
            .into_any_element()
        } else {
            div()
                .min_w_0()
                .font_weight(FontWeight(750.))
                .text_size(px(theme.font_size_md))
                .line_height(gpui_kit::relative(1.35))
                .child(title)
                .into_any_element()
        });
    }
    if let Some(description) = content.description {
        card = card.child(
            div()
                .min_w_0()
                .mt(px(7.))
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_sm))
                .child(description),
        );
    }
    card
}
