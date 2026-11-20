//! Embed metadata, image viewer routing and browser video previews.
use super::super::super::media::{
    image_viewer::ViewerImage,
    media_player::{self, MediaPlayers},
};
use crate::shared::images::Images;
use gpui_kit::*;
use stonks_core::messages::models::TimelineEmbed;
use stonks_ui::{
    components::{
        self,
        embed::{embed_card_header, CardContent},
        media_player::{media_link, UiMediaPoster},
    },
    i18n::t,
    Theme,
};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direct {
    Image,
    Video,
}
fn http_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}
fn direct(value: &str) -> Option<Direct> {
    let url = Url::parse(&http_url(value)?).ok()?;
    let extension = url.path().rsplit('.').next()?.to_ascii_lowercase();
    match extension.as_str() {
        "avif" | "bmp" | "gif" | "jpg" | "jpeg" | "png" | "webp" => Some(Direct::Image),
        "m4v" | "mov" | "mp4" | "webm" => Some(Direct::Video),
        _ => None,
    }
}
fn valid_video_id(id: &str) -> bool {
    (6..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn youtube_id(embed: &TimelineEmbed) -> Option<String> {
    if let Some(id) = embed
        .special_type
        .as_deref()
        .filter(|kind| kind.eq_ignore_ascii_case("youtube"))
        .and(embed.special_id.as_deref())
        .filter(|id| valid_video_id(id))
    {
        return Some(id.into());
    }
    let url = Url::parse(&http_url(embed.url.as_deref()?)?).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let mut path = url.path_segments()?;
    let first = path.next()?;
    let id = match host.as_str() {
        "youtu.be" | "www.youtu.be" => first.to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => match first {
            "watch" => url
                .query_pairs()
                .find(|(key, _)| key == "v")?
                .1
                .into_owned(),
            "embed" | "shorts" | "live" => path.next()?.into(),
            _ => return None,
        },
        _ => return None,
    };
    valid_video_id(&id).then_some(id)
}
#[derive(Debug, PartialEq, Eq)]
struct Preview {
    target: Option<String>,
    poster: Option<String>,
    icon: Option<String>,
    direct_image: bool,
    browser_video: bool,
}
fn preview(embed: &TimelineEmbed) -> Option<Preview> {
    if embed.kind.eq_ignore_ascii_case("none") {
        return None;
    }
    let youtube = youtube_id(embed);
    let video_url = embed.video_url.as_deref().and_then(http_url);
    let target = embed
        .url
        .as_deref()
        .and_then(http_url)
        .or_else(|| {
            youtube
                .as_ref()
                .map(|id| format!("https://www.youtube.com/watch?v={id}"))
        })
        .or_else(|| video_url.clone());
    let direct_kind = embed.url.as_deref().and_then(direct);
    let direct_image = direct_kind == Some(Direct::Image)
        || (embed.kind.eq_ignore_ascii_case("image") && direct_kind != Some(Direct::Video));
    let poster = youtube
        .as_ref()
        .map(|id| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
        .or_else(|| embed.image_url.as_deref().and_then(http_url))
        .or_else(|| direct_image.then(|| target.clone()).flatten());
    let browser_video = !direct_image
        && target.is_some()
        && (youtube.is_some()
            || video_url.is_some()
            || direct_kind == Some(Direct::Video)
            || embed.kind.eq_ignore_ascii_case("video"));
    let icon = embed.icon_url.as_deref().and_then(http_url);
    if target.is_none()
        && poster.is_none()
        && icon.is_none()
        && embed
            .title
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        && embed
            .description
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return None;
    }
    Some(Preview {
        target,
        poster,
        icon,
        direct_image,
        browser_video,
    })
}
/// Prefetch only poster and icon images, never the video itself.
pub fn sources(embed: &TimelineEmbed) -> Vec<(String, u32)> {
    let Some(preview) = preview(embed) else {
        return vec![];
    };
    preview
        .poster
        .into_iter()
        .map(|url| (url, 1280))
        .chain(preview.icon.into_iter().map(|url| (url, 32)))
        .collect()
}

pub fn content_playable(content: Option<&str>) -> bool {
    content
        .map(str::trim)
        .is_some_and(|value| direct(value) == Some(Direct::Video))
}
pub fn content_sources(content: Option<&str>) -> Vec<(String, u32)> {
    let Some(url) = content
        .map(str::trim)
        .filter(|url| direct(url) == Some(Direct::Image))
        .and_then(http_url)
    else {
        return vec![];
    };
    vec![(url, 1280)]
}
fn image_view(
    id: &str,
    url: String,
    name: &str,
    poster: Option<std::sync::Arc<RenderImage>>,
    theme: &Theme,
    width: f32,
    open_image: &super::super::OpenImage,
) -> AnyElement {
    let dimensions = poster
        .as_ref()
        .map(|image| {
            let size = image.size(0);
            (
                Some(size.width.0.max(1) as u64),
                Some(size.height.0.max(1) as u64),
            )
        })
        .unwrap_or((None, None));
    let open = open_image.clone();
    let target = ViewerImage::new(url, name);
    components::media::image_opener(
        ElementId::Name(format!("{id}-image").into()),
        theme,
        t("messages.viewImage"),
    )
    .on_click(move |_, window, cx| {
        cx.stop_propagation();
        open(target.clone(), window, cx);
    })
    .child(components::image_attachment(
        theme, name, poster, dimensions, width,
    ))
    .into_any_element()
}
pub fn render_content(
    content: Option<&str>,
    id: &str,
    theme: &Theme,
    width: f32,
    images: &Entity<Images>,
    players: &Entity<MediaPlayers>,
    open_image: &super::super::OpenImage,
    cx: &mut App,
) -> Option<AnyElement> {
    let value = content?.trim();
    let kind = direct(value)?;
    let url = http_url(value)?;
    Some(match kind {
        Direct::Image => {
            let image = images.update(cx, |images, cx| images.get(Some(&url), 1280, cx));
            image_view(
                id,
                url,
                &t("messages.linkPreview"),
                image,
                theme,
                width,
                open_image,
            )
        }
        Direct::Video => media_player::render_video(
            players,
            id,
            &url,
            &t("media.video"),
            None,
            width,
            None,
            false,
            false,
            cx,
        ),
    })
}
pub fn render(
    embed: &TimelineEmbed,
    id: &str,
    theme: &Theme,
    width: f32,
    images: &Entity<Images>,
    open_image: &super::super::OpenImage,
    cx: &mut App,
) -> Option<AnyElement> {
    let preview = preview(embed)?;
    let poster = images.update(cx, |images, cx| {
        images.get(preview.poster.as_deref(), 1280, cx)
    });
    let icon = images.update(cx, |images, cx| images.get(preview.icon.as_deref(), 32, cx));
    let title = embed
        .title
        .as_deref()
        .unwrap_or(&t("messages.linkPreview"))
        .to_owned();
    if preview.direct_image {
        let url = preview.poster.clone().or(preview.target)?;
        return Some(image_view(
            id, url, &title, poster, theme, width, open_image,
        ));
    }

    let target = preview.target.clone();
    let content_width = (width.clamp(1., 530.) - 32.).max(1.);
    let mut card = embed_card_header(
        theme,
        id,
        CardContent {
            title: embed.title.clone(),
            description: embed.description.clone(),
            site: embed.site_name.clone(),
            target: target.clone(),
        },
        icon,
        width,
    );
    if let Some(url) = target.clone().filter(|_| preview.browser_video) {
        card = card.child(
            div().mt(px(theme.space3)).child(
                UiMediaPoster::new(
                    ElementId::Name(format!("{id}-browser-play").into()),
                    *theme,
                    content_width,
                    format!("{}: {title}", t("messages.openLink")),
                )
                .image(poster)
                .icon(stonks_ui::icons::Icon::ExternalLink)
                .on_click(move |_, window, cx| stonks_ui::external_links::request(&url, window, cx)),
            ),
        );
    } else if let Some(url) = preview.poster {
        let open = open_image.clone();
        let target = ViewerImage::new(url, &title);
        let mut image = components::media::image_opener(
            ElementId::Name(format!("{id}-poster").into()),
            theme,
            t("messages.viewImage"),
        )
        .w_full()
        .h(px((content_width * 9. / 16.).min(330.)))
        .mt(px(theme.space3))
        .rounded(px(theme.radius_sm))
        .bg(theme.canvas)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            open(target.clone(), window, cx);
        });
        if let Some(poster) = poster {
            image = image.child(
                img(poster)
                    .id("website-poster-image")
                    .size_full()
                    .rounded(px(theme.radius_sm))
                    .object_fit(ObjectFit::Cover),
            );
        }
        card = card.child(image);
    }
    if let Some(target) = target {
        card = card.child(div().mt(px(theme.space2)).child(media_link(
            theme,
            ElementId::Name(format!("{id}-open").into()),
            format!("{} ↗", t("messages.openLink")),
            target,
        )));
    }
    Some(card.into_any_element())
}
#[cfg(test)]
mod tests {
    use super::{content_playable, content_sources, direct, preview, sources, youtube_id, Direct};
    use stonks_core::messages::models::TimelineEmbed;
    fn embed() -> TimelineEmbed {
        TimelineEmbed {
            kind: "Website".into(),
            title: Some("Title".into()),
            description: None,
            url: None,
            site_name: None,
            image_url: None,
            video_url: None,
            icon_url: None,
            special_type: None,
            special_id: None,
        }
    }
    #[test]
    fn youtube_metadata_and_exact_hosts_get_explicit_playable_posters() {
        for url in [
            "https://youtu.be/abc_def-12",
            "https://www.youtube.com/watch?v=abc_def-12",
            "https://m.youtube.com/shorts/abc_def-12",
        ] {
            let mut value = embed();
            value.url = Some(url.into());
            assert_eq!(youtube_id(&value).as_deref(), Some("abc_def-12"));
            assert_eq!(
                sources(&value),
                [(
                    "https://i.ytimg.com/vi/abc_def-12/hqdefault.jpg".into(),
                    1280
                )]
            );
        }
    }
    #[test]
    fn metadata_only_youtube_has_watch_target() {
        let mut value = embed();
        value.special_type = Some("YouTube".into());
        value.special_id = Some("abc_def-12".into());
        let preview = preview(&value).unwrap();
        assert!(preview.browser_video);
        assert_eq!(
            preview.target.as_deref(),
            Some("https://www.youtube.com/watch?v=abc_def-12")
        );
    }
    #[test]
    fn external_video_prefers_page_and_only_prefetches_poster() {
        let mut value = embed();
        value.url = Some("https://vimeo.com/12345".into());
        value.video_url = Some("https://player.vimeo.com/video/12345".into());
        value.image_url = Some("https://cdn.example/poster.jpg".into());
        let card = preview(&value).unwrap();
        assert!(card.browser_video);
        assert_eq!(card.target, value.url);
        assert_eq!(sources(&value), [(value.image_url.take().unwrap(), 1280)]);
        value.url = None;
        assert_eq!(preview(&value).unwrap().target, value.video_url);
    }
    #[test]
    fn unsafe_urls_and_lookalike_hosts_never_load() {
        let mut value = embed();
        value.url = Some("https://youtube.com.evil.example/watch?v=abc_def-12".into());
        assert!(youtube_id(&value).is_none());
        value.url = Some("javascript:alert(1)".into());
        value.image_url = Some("https://user:secret@example.com/image".into());
        value.kind = "Video".into();
        value.video_url = Some("file:///video.mp4".into());
        assert!(sources(&value).is_empty());
        assert!(!preview(&value).unwrap().browser_video);
    }
    #[test]
    fn whole_message_media_detection_does_not_load_mixed_text() {
        assert_eq!(
            direct("https://cdn.example/clip.MP4?token=private"),
            Some(Direct::Video)
        );
        assert_eq!(
            direct("https://cdn.example/image.png?download=1"),
            Some(Direct::Image)
        );
        assert!(content_sources(Some("caption https://cdn.example/image.png")).is_empty());
        assert_eq!(
            content_sources(Some(" https://cdn.example/image.png ")).len(),
            1
        );
        assert!(content_sources(Some("https://cdn.example/clip.mp4")).is_empty());
    }
    #[test]
    fn browser_video_projection_excludes_images_and_hidden_embeds() {
        let mut value = embed();
        value.url = Some("https://cdn.example/clip.mp4".into());
        assert!(preview(&value).unwrap().browser_video);
        value.url = Some("https://cdn.example/image.png".into());
        assert!(!preview(&value).unwrap().browser_video);
        value.url = Some("https://youtu.be/abc_def-12".into());
        assert!(preview(&value).unwrap().browser_video);
        value.kind = "None".into();
        assert!(preview(&value).is_none());
        assert!(content_playable(Some(" https://cdn.example/clip.mp4 ")));
        assert!(!content_playable(Some(
            "caption https://cdn.example/clip.mp4"
        )));
        assert!(!content_playable(Some("https://cdn.example/image.gif")));
        assert!(!content_playable(None));
    }
    #[test]
    fn direct_website_images_are_not_generic_cards_and_none_is_hidden() {
        let mut value = embed();
        value.url = Some("https://cdn.example/image.gif".into());
        assert!(preview(&value).unwrap().direct_image);
        value.kind = "None".into();
        assert!(preview(&value).is_none());
    }
}
