use std::collections::HashMap;

use serde_json::Value;

use crate::error::{CommandError, ErrorCode};

use super::models::{TimelineAttachment, TimelineEmbed, TimelineMessage, TimelineReaction};

pub fn valid_id(value: &str) -> bool {
    value.len() == 26 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn file_url(autumn: Option<&str>, file: &Value, original: bool) -> Option<String> {
    let base = autumn?;
    let tag = file.get("tag")?.as_str()?;
    let id = file.get("_id")?.as_str()?;
    Some(format!(
        "{base}/{tag}/{id}{}{}",
        if original { "/original" } else { "" },
        if original { "?animated=1" } else { "" }
    ))
}

fn attachment(file: &Value, autumn: Option<&str>) -> Option<TimelineAttachment> {
    let metadata = file.get("metadata").unwrap_or(&Value::Null);
    Some(TimelineAttachment {
        id: file.get("_id")?.as_str()?.to_owned(),
        filename: file
            .get("filename")
            .and_then(Value::as_str)
            .unwrap_or("Attachment")
            .to_owned(),
        content_type: file
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream")
            .to_owned(),
        size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
        kind: super::models::attachment_kind(
            metadata
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("File"),
            file.get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream"),
            file.get("filename")
                .and_then(Value::as_str)
                .unwrap_or("Attachment"),
        )
        .to_owned(),
        width: metadata.get("width").and_then(Value::as_u64),
        height: metadata.get("height").and_then(Value::as_u64),
        url: file_url(autumn, file, false),
        original_url: file_url(autumn, file, true),
    })
}

fn embed(value: &Value) -> TimelineEmbed {
    let special = value.get("special").filter(|special| !special.is_null());
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("None");
    let media_url = |field: &str| {
        value
            .get(field)
            .and_then(|media| {
                media
                    .as_str()
                    .or_else(|| media.get("url").and_then(Value::as_str))
            })
            .map(ToOwned::to_owned)
    };
    TimelineEmbed {
        kind: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("None")
            .to_owned(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        url: value
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        site_name: value
            .get("site_name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        image_url: media_url("image").or_else(|| {
            kind.eq_ignore_ascii_case("Image")
                .then(|| media_url("url"))
                .flatten()
        }),
        video_url: media_url("video").or_else(|| {
            kind.eq_ignore_ascii_case("Video")
                .then(|| media_url("url"))
                .flatten()
        }),
        icon_url: value
            .get("icon_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        special_type: special
            .and_then(|special| special.get("type"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        special_id: special
            .and_then(|special| special.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

/// Apply the same projection to initial history and asynchronously generated
/// embed metadata in gateway MessageUpdate/MessageAppend events.
pub fn normalize_embeds(values: &[Value]) -> Vec<TimelineEmbed> {
    values.iter().map(embed).collect()
}

fn avatar_from_user(user: &Value, autumn: Option<&str>) -> Option<String> {
    user.get("avatar")
        .and_then(|file| file_url(autumn, file, true))
}

pub fn member_user_id(member: &Value) -> Option<&str> {
    member.get("_id")?.get("user")?.as_str()
}

fn author_details(
    message: &Value,
    users: &HashMap<String, Value>,
    members: &HashMap<String, Value>,
    autumn: Option<&str>,
) -> (String, Option<String>) {
    if let Some(masquerade) = message.get("masquerade") {
        if let Some(name) = masquerade.get("name").and_then(Value::as_str) {
            return (
                name.to_owned(),
                masquerade
                    .get("avatar")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            );
        }
    }
    if let Some(webhook) = message.get("webhook") {
        if let Some(name) = webhook.get("name").and_then(Value::as_str) {
            let avatar = webhook
                .get("avatar")
                .and_then(Value::as_str)
                .and_then(|id| autumn.map(|base| format!("{base}/avatars/{id}")));
            return (name.to_owned(), avatar);
        }
    }
    let author = message
        .get("system")
        .and_then(|system| system.get("by").or_else(|| system.get("id")))
        .or_else(|| message.get("author"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let member = message
        .get("member")
        .filter(|value| !value.is_null())
        .or_else(|| members.get(author));
    let user = message
        .get("user")
        .filter(|value| !value.is_null())
        .or_else(|| users.get(author));
    let name = member
        .and_then(|value| value.get("nickname"))
        .and_then(Value::as_str)
        .or_else(|| {
            user.and_then(|value| value.get("display_name"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            user.and_then(|value| value.get("username"))
                .and_then(Value::as_str)
        })
        .unwrap_or("Unknown user")
        .to_owned();
    let avatar = member
        .and_then(|value| value.get("avatar"))
        .and_then(|file| file_url(autumn, file, true))
        .or_else(|| user.and_then(|value| avatar_from_user(value, autumn)));
    (name, avatar)
}

pub fn normalize_message(
    value: &Value,
    users: &HashMap<String, Value>,
    members: &HashMap<String, Value>,
    autumn: Option<&str>,
) -> Result<TimelineMessage, CommandError> {
    let invalid_message = || {
        CommandError::new(
            ErrorCode::InvalidResponse,
            "The Stoat server returned an invalid message.",
        )
    };
    let id = value
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_message)?
        .to_owned();
    let channel_id = value
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(invalid_message)?
        .to_owned();
    let system = value.get("system").filter(|system| !system.is_null());
    let author_id = system
        .and_then(|system| system.get("by").or_else(|| system.get("id")))
        .or_else(|| value.get("author"))
        .and_then(Value::as_str)
        .ok_or_else(invalid_message)?
        .to_owned();
    let (author_name, author_avatar_url) = author_details(value, users, members, autumn);
    let attachments = value
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| attachment(file, autumn))
        .collect();
    let embeds = normalize_embeds(
        value
            .get("embeds")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    let reply_ids = value
        .get("replies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|reply| reply.as_str().or_else(|| reply.get("id")?.as_str()))
        .filter(|id| valid_id(id))
        .map(ToOwned::to_owned)
        .collect();
    let reactions = value
        .get("reactions")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .map(|(emoji, users)| TimelineReaction {
                    emoji: emoji.to_owned(),
                    user_ids: users
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(TimelineMessage {
        id,
        channel_id,
        author_id,
        author_name,
        author_avatar_url,
        content: value
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        edited_at: value
            .get("edited")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        attachments,
        embeds,
        reply_ids,
        reactions,
        pinned: value
            .get("pinned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        system: system.is_some(),
        system_type: system
            .and_then(|system| system.get("type"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        system_finished_at: system
            .and_then(|system| system.get("finished_at"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

pub fn response_parts(value: Value) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    if let Some(messages) = value.as_array() {
        return (messages.clone(), Vec::new(), Vec::new());
    }
    let take = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    (take("messages"), take("users"), take("members"))
}

#[cfg(test)]
mod embed_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn standalone_media_embeds_preserve_their_top_level_resource() {
        let image = embed(&json!({"type":"Image","url":"https://cdn.example.com/image.webp"}));
        assert_eq!(
            image.image_url.as_deref(),
            Some("https://cdn.example.com/image.webp")
        );
        assert_eq!(image.url, image.image_url);
        assert!(image.video_url.is_none());
        let video = embed(&json!({"type":"Video","url":"https://cdn.example.com/video.mp4"}));
        assert_eq!(
            video.video_url.as_deref(),
            Some("https://cdn.example.com/video.mp4")
        );
        assert!(video.image_url.is_none());
    }

    #[test]
    fn website_media_metadata_and_specials_survive_projection() {
        let website = embed(&json!({
            "type":"Website","url":"https://www.youtube.com/watch?v=abc_def-12",
            "title":"Video title","description":"Description","site_name":"YouTube",
            "image":{"url":"https://cdn.example.com/poster.webp"},
            "video":{"url":"https://cdn.example.com/video.mp4"},
            "special":{"type":"YouTube","id":"abc_def-12"},
            "icon_url":"https://cdn.example.com/site.png"
        }));
        assert_eq!(website.site_name.as_deref(), Some("YouTube"));
        assert_eq!(
            website.image_url.as_deref(),
            Some("https://cdn.example.com/poster.webp")
        );
        assert_eq!(
            website.video_url.as_deref(),
            Some("https://cdn.example.com/video.mp4")
        );
        assert_eq!(website.special_type.as_deref(), Some("YouTube"));
        assert_eq!(website.special_id.as_deref(), Some("abc_def-12"));
        let string_media =
            embed(&json!({"type":"Website","image":"https://cdn.example.com/poster.webp"}));
        assert_eq!(string_media.image_url, website.image_url);
    }
}
