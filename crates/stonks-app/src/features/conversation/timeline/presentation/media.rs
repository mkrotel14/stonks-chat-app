//! Playback ownership follows rendered media, including edits/deletions.
use super::embeds;
use std::collections::HashSet;
use stonks_core::messages::models::TimelineMessage;

pub(super) fn playable_keys(
    messages: &[TimelineMessage],
    display_embeds: bool,
    revealed: &HashSet<String>,
) -> HashSet<String> {
    messages
        .iter()
        .flat_map(|message| {
            let attachments = message
                .attachments
                .iter()
                .filter(|file| {
                    (!file.filename.starts_with("SPOILER_")
                        || revealed.contains(&format!("media-{}-{}", message.id, file.id)))
                        && matches!(file.kind.as_str(), "Audio" | "Video")
                        && file
                            .original_url
                            .as_deref()
                            .or(file.url.as_deref())
                            .is_some()
                })
                .map(|file| format!("{}-attachment-{}", message.id, file.id));

            let duplicate = message.embeds.iter().any(|embed| {
                embed.kind != "None"
                    && embed.url.as_deref() == message.content.as_deref().map(str::trim)
            });
            let content = (display_embeds
                && !duplicate
                && embeds::content_playable(message.content.as_deref()))
            .then(|| format!("{}-content", message.id));
            attachments.chain(content)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stonks_core::messages::models::{TimelineAttachment, TimelineEmbed};
    fn keys(messages: &[TimelineMessage], display_embeds: bool) -> HashSet<String> {
        playable_keys(messages, display_embeds, &HashSet::new())
    }
    fn message() -> TimelineMessage {
        TimelineMessage {
            id: "message".into(),
            channel_id: "channel".into(),
            author_id: "author".into(),
            author_name: "Author".into(),
            author_avatar_url: None,
            content: Some("https://example.com/video.mp4".into()),
            edited_at: None,
            attachments: vec![TimelineAttachment {
                id: "audio".into(),
                filename: "sound.mp3".into(),
                content_type: "audio/mpeg".into(),
                size: 1,
                kind: "Audio".into(),
                width: None,
                height: None,
                url: Some("https://example.com/sound.mp3".into()),
                original_url: None,
            }],
            embeds: vec![],
            reply_ids: vec![],
            reactions: vec![],
            pinned: false,
            system: false,
            system_type: None,
            system_finished_at: None,
        }
    }
    #[test]
    fn deletion_edits_and_embed_preferences_release_hidden_players() {
        let mut item = message();
        assert_eq!(keys(&[item.clone()], true).len(), 2);
        assert_eq!(
            keys(&[item.clone()], false),
            HashSet::from(["message-attachment-audio".into()])
        );
        item.content = Some("edited into plain text".into());
        assert!(!keys(&[item.clone()], true).contains("message-content"));
        item.attachments[0].kind = "File".into();
        assert!(keys(&[item], true).is_empty());
        assert!(keys(&[], true).is_empty());
        let mut concealed = message();
        concealed.content = None;
        concealed.attachments[0].filename = "SPOILER_sound.mp3".into();
        assert!(keys(&[concealed.clone()], true).is_empty());
        assert_eq!(
            playable_keys(
                &[concealed],
                true,
                &HashSet::from(["media-message-audio".into()])
            ),
            HashSet::from(["message-attachment-audio".into()])
        );
    }
    #[test]
    fn matching_browser_embed_suppresses_duplicate_inline_player() {
        let mut item = message();
        item.attachments.clear();
        item.embeds.push(TimelineEmbed {
            kind: "Video".into(),
            title: None,
            description: None,
            url: item.content.clone(),
            site_name: None,
            image_url: None,
            video_url: item.content.clone(),
            icon_url: None,
            special_type: None,
            special_id: None,
        });
        assert!(keys(&[item.clone()], true).is_empty());
        item.embeds.clear();
        assert_eq!(
            keys(&[item], true),
            HashSet::from(["message-content".into()])
        );
    }
}
