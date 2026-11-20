#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageQuery {
    pub channel_id: String,
    pub before: Option<String>,
    pub nearby: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSearchRequest {
    pub channel_id: String,
    pub query: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    pub before: Option<String>,
    #[serde(default = "default_message_sort")]
    pub sort: String,
}

fn default_message_sort() -> String {
    "Relevance".into()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinMessageRequest {
    pub channel_id: String,
    pub message_id: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub channel_id: String,
    pub content: Option<String>,
    #[serde(default)]
    pub attachments: Vec<String>,
    pub reply_to: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageRequest {
    pub channel_id: String,
    pub message_id: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteMessageRequest {
    pub channel_id: String,
    pub message_id: String,
}

pub type FetchMessageRequest = DeleteMessageRequest;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionRequest {
    pub channel_id: String,
    pub message_id: String,
    pub emoji: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadAttachmentRequest {
    pub grant_id: String,
    #[serde(default)]
    pub spoiler: bool,
}

#[derive(Debug, Serialize)]
pub struct UploadedAttachment {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAttachment {
    pub grant_id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub preview_url: String,
}

pub fn attachment_content_type(filename: &str) -> &'static str {
    match std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("zip") => "application/zip",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Metadata is authoritative; MIME/extension fallback handles older uploads
/// whose metadata was stored as a generic File.
pub fn attachment_kind(kind: &str, content_type: &str, filename: &str) -> &'static str {
    for known in ["Image", "Audio", "Video"] {
        if kind.eq_ignore_ascii_case(known) {
            return known;
        }
    }
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    for (prefix, known) in [
        ("image/", "Image"),
        ("audio/", "Audio"),
        ("video/", "Video"),
    ] {
        if mime.starts_with(prefix) {
            return known;
        }
    }
    match std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp") => "Image",
        Some("mp3" | "m4a" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wav") => "Audio",
        Some("mp4" | "m4v" | "mov" | "webm" | "ogv") => "Video",
        _ => "File",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineAttachment {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub kind: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub url: Option<String>,
    pub original_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEmbed {
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
    pub icon_url: Option<String>,
    pub special_type: Option<String>,
    pub special_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineReaction {
    pub emoji: String,
    pub user_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMessage {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub content: Option<String>,
    pub edited_at: Option<String>,
    pub attachments: Vec<TimelineAttachment>,
    pub embeds: Vec<TimelineEmbed>,
    pub reply_ids: Vec<String>,
    pub reactions: Vec<TimelineReaction>,
    pub pinned: bool,
    pub system: bool,
    pub system_type: Option<String>,
    pub system_finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub messages: Vec<TimelineMessage>,
    pub has_more: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GifMedia {
    pub url: String,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct GifMediaFormats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tinywebm: Option<GifMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webm: Option<GifMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gif: Option<GifMedia>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GifResult {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_formats: Option<GifMediaFormats>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GifSearchResults {
    pub results: Vec<GifResult>,
}

impl GifSearchResults {
    pub fn project(value: &serde_json::Value) -> Result<Self, ()> {
        let (items, fallback) = if let Some(items) = value.as_array() {
            (items, false)
        } else if let Some(items) = value.get("results").and_then(serde_json::Value::as_array) {
            (items, false)
        } else if let Some(items) = value.get("data").and_then(serde_json::Value::as_array) {
            (items, true)
        } else {
            return Err(());
        };

        let results = items
            .iter()
            .map(|item| {
                let url = item
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(())?;
                if url.is_empty() {
                    return Err(());
                }
                if fallback {
                    let preview = item
                        .get("preview_url")
                        .or_else(|| item.get("previewUrl"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(url);
                    return Ok(GifResult {
                        url: url.to_owned(),
                        media_formats: Some(GifMediaFormats {
                            gif: Some(GifMedia {
                                url: preview.to_owned(),
                            }),
                            ..GifMediaFormats::default()
                        }),
                    });
                }

                let formats = item
                    .get("media_formats")
                    .or_else(|| item.get("mediaFormats"));
                let media = |name: &str| -> Result<Option<GifMedia>, ()> {
                    let Some(format) = formats.and_then(|formats| formats.get(name)) else {
                        return Ok(None);
                    };
                    if format.is_null() {
                        return Ok(None);
                    }
                    let url = format
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .filter(|url| !url.is_empty())
                        .ok_or(())?;
                    Ok(Some(GifMedia {
                        url: url.to_owned(),
                    }))
                };
                let media_formats = GifMediaFormats {
                    tinywebm: media("tinywebm")?,
                    webm: media("webm")?,
                    gif: media("gif")?,
                };
                Ok(GifResult {
                    url: url.to_owned(),
                    media_formats: (media_formats != GifMediaFormats::default())
                        .then_some(media_formats),
                })
            })
            .collect::<Result<Vec<_>, ()>>()?;
        Ok(Self { results })
    }
}

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod media_kind_tests {
    use super::attachment_kind;
    #[test]
    fn legacy_audio_video_and_images_get_players_without_overriding_metadata() {
        assert_eq!(
            attachment_kind("File", "audio/mpeg", "Renan Santos.mp3"),
            "Audio"
        );
        assert_eq!(
            attachment_kind("File", "application/octet-stream", "TRACK.MP3"),
            "Audio"
        );
        assert_eq!(
            attachment_kind("File", "video/mp4; codecs=avc1", "clip"),
            "Video"
        );
        assert_eq!(
            attachment_kind("File", "application/octet-stream", "clip.webm"),
            "Video"
        );
        assert_eq!(attachment_kind("File", "image/webp", "photo"), "Image");
        assert_eq!(attachment_kind("Video", "audio/ogg", "clip.ogg"), "Video");
        assert_eq!(
            attachment_kind("File", "application/octet-stream", "clip.mp4.exe"),
            "File"
        );
    }
}
