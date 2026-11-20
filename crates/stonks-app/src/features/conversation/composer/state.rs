//! Composer transitions independent of GPUI, transport and wall-clock time.
use super::attachments::{Attachment, PickedAttachment, MAX_ATTACHMENTS};
use super::mentions::{self, DraftMention};
use std::{collections::HashMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReplyDraft {
    pub id: String,
    pub author: String,
    pub excerpt: Option<String>,
}

struct Retry {
    content: String,
    reply_to: Option<String>,
    key: String,
    files: Vec<(u64, bool)>,
    uploaded: Vec<String>,
}

pub(super) enum SubmitError {
    Music(String),
    Send(String),
}

#[derive(Default)]
struct Draft {
    text: String,
    mentions: Vec<DraftMention>,
    revision: u64,
    error: Option<SubmitError>,
    reply: Option<ReplyDraft>,
    retry: Option<Retry>,
    files: Vec<PickedAttachment>,
    picker_error: Option<String>,
    inflight: Option<Attempt>,
    music: Option<u64>,
}

#[derive(Clone)]
pub struct Attempt {
    pub id: u64,
    pub channel: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub key: String,
    pub files: Vec<Attachment>,
    pub uploaded: Vec<String>,
    revision: u64,
}

pub(super) struct MusicAttempt {
    pub channel: String,
    pub url: String,
    id: u64,
    revision: u64,
    navigation: u64,
}

pub struct Completion {
    pub channel: String,
    pub cleared: bool,
}

#[derive(Default)]
pub struct ComposerState {
    active: Option<String>,
    drafts: HashMap<String, Draft>,
    navigation: u64,
    next_attempt: u64,
}

impl ComposerState {
    pub fn active_channel(&self) -> Option<&str> {
        self.active.as_deref()
    }
    pub fn value(&self) -> &str {
        self.active
            .as_ref()
            .and_then(|id| self.drafts.get(id))
            .map_or("", |draft| draft.text.as_str())
    }
    pub(super) fn reply(&self) -> Option<&ReplyDraft> {
        self.active
            .as_ref()
            .and_then(|id| self.drafts.get(id))
            .and_then(|draft| draft.reply.as_ref())
    }

    pub(super) fn set_reply(&mut self, reply: Option<ReplyDraft>) -> bool {
        let Some(channel) = self.active.as_ref() else {
            return false;
        };
        if self.reply() == reply.as_ref() {
            return false;
        }
        let draft = self.drafts.entry(channel.clone()).or_default();
        draft.reply = reply;
        draft.revision = draft.revision.wrapping_add(1);
        true
    }

    pub(super) fn error(&self) -> Option<&SubmitError> {
        self.active_draft()?.error.as_ref()
    }
    fn active_draft(&self) -> Option<&Draft> {
        self.drafts.get(self.active.as_ref()?)
    }
    fn active_draft_mut(&mut self) -> Option<&mut Draft> {
        Some(self.drafts.entry(self.active.clone()?).or_default())
    }
    pub fn sending_here(&self) -> bool {
        self.active_draft()
            .is_some_and(|draft| draft.inflight.is_some() || draft.music.is_some())
    }
    pub fn can_send(&self) -> bool {
        self.active.is_some()
            && !self.sending_here()
            && (!self.value().trim().is_empty() || !self.files().is_empty())
    }
    pub(super) fn files(&self) -> &[PickedAttachment] {
        self.active_draft().map_or(&[], |draft| &draft.files)
    }
    pub(super) fn navigation(&self) -> u64 {
        self.navigation
    }
    pub(super) fn picker_error(&self) -> Option<&str> {
        self.active_draft()?.picker_error.as_deref()
    }
    pub(super) fn finish_pick(
        &mut self,
        navigation: u64,
        result: Result<Vec<PickedAttachment>, String>,
    ) {
        if navigation != self.navigation {
            return;
        }
        let Some(draft) = self.active_draft_mut() else {
            return;
        };
        match result {
            Ok(files) => {
                let available = MAX_ATTACHMENTS.saturating_sub(draft.files.len());
                let old_len = draft.files.len();
                draft.files.extend(files.into_iter().take(available));
                if old_len != draft.files.len() {
                    draft.revision = draft.revision.wrapping_add(1);
                    draft.picker_error = None;
                }
            }
            Err(error) => draft.picker_error = Some(error),
        }
    }
    pub(super) fn remove_file(&mut self, id: u64) {
        let Some(draft) = self.active_draft_mut() else {
            return;
        };
        let old_len = draft.files.len();
        draft.files.retain(|file| file.payload.id != id);
        if old_len != draft.files.len() {
            draft.revision = draft.revision.wrapping_add(1);
            draft.picker_error = None;
        }
    }
    pub(super) fn set_spoiler(&mut self, id: u64, spoiler: bool) {
        let Some(draft) = self.active_draft_mut() else {
            return;
        };
        if let Some(file) = draft.files.iter_mut().find(|file| file.payload.id == id) {
            if file.payload.spoiler != spoiler {
                file.payload.spoiler = spoiler;
                file.revealed = false;
                draft.revision = draft.revision.wrapping_add(1);
            }
        }
    }

    pub(super) fn reveal_file(&mut self, id: u64) {
        if let Some(file) = self
            .active_draft_mut()
            .and_then(|draft| draft.files.iter_mut().find(|file| file.payload.id == id))
        {
            file.revealed = true;
        }
    }

    pub fn edit(&mut self, text: String) {
        let Some(channel) = self.active.as_ref() else {
            return;
        };
        let draft = self.drafts.entry(channel.clone()).or_default();
        if draft.text != text {
            draft.revision = draft.revision.wrapping_add(1);
            mentions::reconcile(&draft.text, &text, &mut draft.mentions);
            draft.text = text;
            if matches!(draft.error, Some(SubmitError::Music(_))) {
                draft.error = None;
            }
        }
    }

    pub(super) fn draft_mentions(&self) -> &[DraftMention] {
        self.active_draft().map_or(&[], |draft| &draft.mentions)
    }

    pub(super) fn insert_mention(
        &mut self,
        text: String,
        replaced: Range<usize>,
        inserted_len: usize,
        mention: Option<DraftMention>,
    ) {
        let Some(draft) = self.active_draft_mut() else {
            return;
        };
        mentions::replace_ranges(&mut draft.mentions, replaced, inserted_len);
        if let Some(mention) = mention {
            let index = draft
                .mentions
                .partition_point(|other| other.range.start < mention.range.start);
            draft.mentions.insert(index, mention);
        }
        draft.text = text;
        draft.revision = draft.revision.wrapping_add(1);
    }

    /// `current_text` accounts for edits still queued by the input widget.
    pub fn activate(&mut self, channel: Option<String>, current_text: String) {
        self.edit(current_text);
        if self.active != channel {
            self.navigation = self.navigation.wrapping_add(1);
        }
        self.active = channel;
    }

    pub(super) fn begin_music(&mut self, content: &str) -> Option<MusicAttempt> {
        if !self.can_send() || !self.files().is_empty() {
            return None;
        }
        let url = music_command_url(content)?.to_owned();
        let channel = self.active.clone()?;
        let draft = self.drafts.get_mut(&channel)?;
        self.next_attempt = self.next_attempt.wrapping_add(1);
        draft.music = Some(self.next_attempt);
        draft.error = None;
        Some(MusicAttempt {
            channel,
            url,
            id: self.next_attempt,
            revision: draft.revision,
            navigation: self.navigation,
        })
    }

    pub(super) fn complete_music(
        &mut self,
        attempt: MusicAttempt,
        result: Option<Result<(), String>>,
    ) -> bool {
        let Some(draft) = self.drafts.get_mut(&attempt.channel) else {
            return false;
        };
        if draft.music != Some(attempt.id) {
            return false;
        }
        draft.music = None;
        if self.navigation != attempt.navigation || draft.revision != attempt.revision {
            return false;
        }
        match result {
            Some(Ok(())) => {
                draft.text.clear();
                draft.mentions.clear();
                draft.reply = None;
                draft.retry = None;
                draft.revision = draft.revision.wrapping_add(1);
                true
            }
            Some(Err(error)) => {
                draft.error = Some(SubmitError::Music(error));
                false
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub fn begin(&mut self, key: impl FnOnce() -> String) -> Option<Attempt> {
        self.begin_with_content(self.value().trim().to_owned(), key)
    }

    pub fn begin_with_content(
        &mut self,
        content: String,
        key: impl FnOnce() -> String,
    ) -> Option<Attempt> {
        if !self.can_send() || (content.trim().is_empty() && self.files().is_empty()) {
            return None;
        }
        let channel = self.active.clone()?;
        let draft = self.drafts.get_mut(&channel)?;
        if draft.files.is_empty() && music_command_url(&content).is_some() {
            return None;
        }
        let content = content.trim_end().to_owned();
        let reply_to = draft.reply.as_ref().map(|reply| reply.id.clone());
        let files: Vec<_> = draft
            .files
            .iter()
            .map(|file| file.payload.clone())
            .collect();
        let identity: Vec<_> = files.iter().map(|file| (file.id, file.spoiler)).collect();
        let (key, uploaded) = draft
            .retry
            .as_ref()
            .filter(|retry| {
                retry.content == content && retry.reply_to == reply_to && retry.files == identity
            })
            .map(|retry| (retry.key.clone(), retry.uploaded.clone()))
            .unwrap_or_else(|| (key(), Vec::new()));
        draft.error = None;
        draft.retry = Some(Retry {
            content: content.clone(),
            reply_to: reply_to.clone(),
            key: key.clone(),
            files: identity,
            uploaded: uploaded.clone(),
        });
        self.next_attempt = self.next_attempt.wrapping_add(1);
        let attempt = Attempt {
            id: self.next_attempt,
            channel,
            content,
            reply_to,
            key,
            files,
            uploaded,
            revision: draft.revision,
        };
        draft.inflight = Some(attempt.clone());
        Some(attempt)
    }

    pub fn complete(
        &mut self,
        channel: &str,
        id: u64,
        error: Option<String>,
        uploaded: Vec<String>,
    ) -> Option<Completion> {
        let draft = self.drafts.get_mut(channel)?;
        if draft.inflight.as_ref()?.id != id {
            return None;
        }
        let attempt = draft.inflight.take()?;
        let cleared = error.is_none() && draft.revision == attempt.revision;
        if cleared {
            draft.text.clear();
            draft.mentions.clear();
            draft.reply = None;
            draft.files.clear();
            draft.picker_error = None;
            draft.revision = draft.revision.wrapping_add(1);
        }
        if error.is_none() {
            draft.retry = None;
        } else if uploaded.len() <= attempt.files.len() && uploaded.starts_with(&attempt.uploaded) {
            if let Some(retry) = &mut draft.retry {
                retry.uploaded = uploaded;
            }
        }
        draft.error = error.map(SubmitError::Send);
        Some(Completion {
            channel: attempt.channel,
            cleared,
        })
    }

    /// Logout/deployment changes must not retain another account's drafts.
    pub fn reset(&mut self) {
        self.active = None;
        self.drafts.clear();
        self.navigation = self.navigation.wrapping_add(1);
    }
}

// Mirror Tauri's musicCommandUrl classification, not URL or file validation.
fn music_command_url(content: &str) -> Option<&str> {
    fn whitespace(c: char) -> bool {
        (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
    }
    let Some(command) = content.get(..6) else {
        return None;
    };
    let argument = &content[6..];
    if !command.eq_ignore_ascii_case("@music") || !argument.starts_with(whitespace) {
        return None;
    }
    let argument = argument.trim_matches(whitespace);
    if argument.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let argument = ['"', '\'']
        .into_iter()
        .find_map(|quote| argument.strip_prefix(quote)?.strip_suffix(quote))
        .map_or(argument, |unquoted| unquoted.trim_matches(whitespace));
    let lower = argument.to_ascii_lowercase();
    if let Some(url) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        return (!url.is_empty() && !url.contains(whitespace)).then_some(argument);
    }
    let path = lower.strip_prefix('/').or_else(|| match lower.as_bytes() {
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic() => Some(&lower[3..]),
        _ => None,
    });
    path.is_some_and(|path| {
        [".mp3", ".wav", ".flac", ".ogg", ".oga", ".m4a", ".aac"]
            .into_iter()
            .any(|ext| path.strip_suffix(ext).is_some_and(|stem| !stem.is_empty()))
    })
    .then_some(argument)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn active(channel: &str, text: &str) -> ComposerState {
        let mut state = ComposerState::default();
        state.activate(Some(channel.into()), String::new());
        state.edit(text.into());
        state
    }
    #[test]
    fn drafts_and_errors_are_owned_by_their_channel() {
        let mut state = active("a", "first");
        let send = state.begin(|| "a-key".into()).unwrap();
        state.complete(&send.channel, send.id, Some("failed".into()), vec![]);
        state.activate(Some("b".into()), "first".into());
        assert_eq!(state.value(), "");
        assert!(state.error().is_none());
        state.edit("second".into());
        state.activate(Some("a".into()), "second".into());
        assert_eq!(state.value(), "first");
        assert!(matches!(state.error(), Some(SubmitError::Send(error)) if error == "failed"));
    }
    #[test]
    fn failed_retries_keep_the_key_even_after_a_send_in_another_channel() {
        let mut state = active("a", " first ");
        let first = state.begin(|| "a-key".into()).unwrap();
        state.complete(&first.channel, first.id, Some("uncertain".into()), vec![]);
        state.activate(Some("b".into()), " first ".into());
        state.edit("second".into());
        let second = state.begin(|| "b-key".into()).unwrap();
        state.complete(&second.channel, second.id, None, vec![]);
        state.activate(Some("a".into()), String::new());
        state.edit("first".into());
        assert_eq!(state.begin(|| "wrong-key".into()).unwrap().key, "a-key");
    }
    #[test]
    fn edits_and_even_reverted_edits_are_not_cleared_by_an_old_response() {
        let mut state = active("a", "hello");
        let attempt = state.begin(|| "key".into()).unwrap();
        state.edit("hello again".into());
        state.edit("hello".into());
        assert!(
            !state
                .complete(&attempt.channel, attempt.id, None, vec![])
                .unwrap()
                .cleared
        );
        assert_eq!(state.value(), "hello");
    }
    #[test]
    fn completion_of_a_previous_channel_preserves_the_visible_draft() {
        let mut state = active("a", "hello");
        let attempt = state.begin(|| "key".into()).unwrap();
        state.activate(Some("b".into()), "hello".into());
        state.edit("draft b".into());
        assert!(
            state
                .complete(&attempt.channel, attempt.id, None, vec![])
                .unwrap()
                .cleared
        );
        assert_eq!(state.value(), "draft b");
        state.activate(Some("a".into()), "draft b".into());
        assert_eq!(state.value(), "");
    }
    #[test]
    fn reset_rejects_stale_responses_and_removes_private_drafts() {
        let mut state = active("a", "private");
        let old = state.begin(|| "key".into()).unwrap();
        state.reset();
        state.activate(Some("a".into()), String::new());
        assert_eq!(state.value(), "");
        state.edit("new account".into());
        let new = state.begin(|| "new-key".into()).unwrap();
        assert_ne!(old.id, new.id);
        assert!(state.complete(&old.channel, old.id, None, vec![]).is_none());
        assert!(state.sending_here());
        assert_eq!(state.value(), "new account");
    }
    #[test]
    fn mapped_custom_emoji_payload_retries_match_wire_content_and_clear_display_draft() {
        let mut state = active("a", " :wave: ");
        let first = state
            .begin_with_content(":id-one:".into(), || "first".into())
            .unwrap();
        assert_eq!(first.content, ":id-one:");
        state.complete(&first.channel, first.id, Some("uncertain".into()), vec![]);
        assert_eq!(state.value(), " :wave: ");
        let retry = state
            .begin_with_content(":id-one:".into(), || "wrong".into())
            .unwrap();
        assert_eq!(retry.key, "first");
        state.complete(&retry.channel, retry.id, Some("uncertain".into()), vec![]);
        // A renamed/replaced emoji mapping changes the actual request body,
        // so it must not reuse the idempotency key for the old payload.
        let changed = state
            .begin_with_content(":id-two:".into(), || "changed".into())
            .unwrap();
        assert_eq!(changed.key, "changed");
        assert!(
            state
                .complete(&changed.channel, changed.id, None, vec![])
                .unwrap()
                .cleared
        );
        assert_eq!(state.value(), "");
    }

    fn reply(id: &str) -> ReplyDraft {
        ReplyDraft {
            id: id.into(),
            author: "Author".into(),
            excerpt: None,
        }
    }

    #[test]
    fn replies_follow_channel_drafts_and_are_discarded_on_reset() {
        let mut state = active("a", "hello");
        state.set_reply(Some(reply("a-target")));
        state.activate(Some("b".into()), "hello".into());
        assert!(state.reply().is_none());
        state.set_reply(Some(reply("b-target")));
        assert!(state.begin(|| "blank".into()).is_none());
        state.activate(Some("a".into()), String::new());
        assert_eq!(state.reply(), Some(&reply("a-target")));
        let attempt = state.begin(|| "sent".into()).unwrap();
        state.activate(Some("b".into()), "hello".into());
        assert!(
            state
                .complete(&attempt.channel, attempt.id, None, vec![])
                .unwrap()
                .cleared
        );
        assert_eq!(state.reply(), Some(&reply("b-target")));
        state.activate(Some("a".into()), String::new());
        assert!(state.reply().is_none());
        state.reset();
        state.activate(Some("b".into()), String::new());
        assert!(state.reply().is_none());
    }

    #[test]
    fn retry_matches_content_and_reply_id_but_not_preview_metadata() {
        let mut state = active("a", "hello");
        state.set_reply(Some(reply("first")));
        let first = state.begin(|| "first-key".into()).unwrap();
        state.complete(&first.channel, first.id, Some("uncertain".into()), vec![]);
        let mut updated = reply("first");
        updated.excerpt = Some("edited preview".into());
        state.set_reply(Some(updated));
        let retry = state.begin(|| "unused".into()).unwrap();
        assert_eq!(retry.key, "first-key");
        assert_eq!(retry.reply_to.as_deref(), Some("first"));
        state.complete(&retry.channel, retry.id, Some("uncertain".into()), vec![]);
        state.set_reply(Some(reply("second")));
        let changed = state.begin(|| "second-key".into()).unwrap();
        assert_eq!(changed.key, "second-key");
        state.complete(
            &changed.channel,
            changed.id,
            Some("uncertain".into()),
            vec![],
        );
        state.set_reply(None);
        let plain = state.begin(|| "plain-key".into()).unwrap();
        assert_eq!(plain.key, "plain-key");
        assert!(plain.reply_to.is_none());
    }

    #[test]
    fn changing_or_canceling_reply_during_send_preserves_the_new_draft() {
        for replacement in [Some(reply("new")), None] {
            let mut state = active("a", "hello");
            state.set_reply(Some(reply("old")));
            let send = state.begin(|| "old-key".into()).unwrap();
            state.set_reply(replacement.clone());
            assert_eq!(send.reply_to.as_deref(), Some("old"));
            assert!(
                !state
                    .complete(&send.channel, send.id, None, vec![])
                    .unwrap()
                    .cleared
            );
            assert_eq!(state.value(), "hello");
            assert_eq!(state.reply(), replacement.as_ref());
        }
    }

    #[test]
    fn blank_and_duplicate_submissions_are_rejected() {
        let mut state = active("a", " \n ");
        assert!(state.begin(|| "key".into()).is_none());
        state.edit("ready".into());
        let attempt = state.begin(|| "key".into()).unwrap();
        assert!(state.begin(|| "second".into()).is_none());
        state.complete(&attempt.channel, attempt.id, None, vec![]);
        assert!(state
            .complete(&attempt.channel, attempt.id, None, vec![])
            .is_none());
    }

    fn picked(id: u64) -> PickedAttachment {
        PickedAttachment {
            payload: Attachment {
                id,
                filename: "sample.txt".into(),
                bytes: [42].into(),
                spoiler: false,
            },
            preview: None,
            revealed: false,
        }
    }

    #[test]
    fn music_classification_matches_tauri_command_scope() {
        for content in [
            "@Music https://example.test/song.mp3  ",
            "@mUsIc\t' HTTP://example.test/stream '",
            "@Music\n/home/user/song.FLAC",
            "@Music \"/home/user/song with spaces.mp3\"",
            "@Music C:\\Music\\song.mp3",
            "@Music z:/song.oga",
        ] {
            assert!(music_command_url(content).is_some(), "{content:?}");
        }
        for content in [
            " @Music https://example.test/track",
            "@Music",
            "@Music https://",
            "@Music https://example.test/a b",
            "@Music https://example.test/track\ncomment",
            "@Music /home/user/song\nwith newline.mp3",
            "@Music /.mp3",
            "@Music /home/user/video.mp4",
            "@Music relative.mp3",
            "@Music not-a-url",
        ] {
            assert!(music_command_url(content).is_none(), "{content:?}");
        }
    }

    #[test]
    fn music_failure_preserves_draft_and_never_begins_chat_send() {
        let command = "@Music https://example.test/track  ";
        let mut state = active("a", command);
        state.set_reply(Some(reply("target")));
        assert!(state
            .begin_with_content(command.into(), || panic!("must not allocate a send key"))
            .is_none());
        let attempt = state.begin_music(command).unwrap();
        assert_eq!(attempt.url, "https://example.test/track");
        assert!(state.sending_here());
        assert!(!state.complete_music(attempt, Some(Err("failed".into()))));
        assert!(!state.sending_here());
        assert!(matches!(state.error(), Some(SubmitError::Music(error)) if error == "failed"));
        assert_eq!(state.value(), command);
        assert_eq!(state.reply(), Some(&reply("target")));

        state.finish_pick(state.navigation(), Ok(vec![picked(1)]));
        let attempt = state
            .begin_with_content(command.into(), || "chat-key".into())
            .unwrap();
        assert_eq!(attempt.content, command.trim_end());
        assert_eq!(attempt.files.len(), 1);
        assert!(state.error().is_none());
    }

    #[test]
    fn music_success_clears_only_the_current_unchanged_draft() {
        let command = "@Music https://example.test/track";
        let mut state = active("a", command);
        let attempt = state.begin_music(command).unwrap();
        state.activate(Some("b".into()), command.into());
        state.activate(Some("a".into()), String::new());
        assert!(!state.complete_music(attempt, Some(Ok(()))));
        assert_eq!(state.value(), command);
        let attempt = state.begin_music(command).unwrap();
        state.edit("new draft".into());
        assert!(!state.complete_music(attempt, Some(Ok(()))));
        assert_eq!(state.value(), "new draft");
        state.edit(command.into());
        let attempt = state.begin_music(command).unwrap();
        assert!(!state.complete_music(attempt, None));
        assert_eq!(state.value(), command);
        let attempt = state.begin_music(command).unwrap();
        assert!(state.complete_music(attempt, Some(Ok(()))));
        assert_eq!(state.value(), "");
    }

    #[test]
    fn picker_is_bounded_and_late_results_cannot_cross_navigation_or_reset() {
        let mut state = active("a", "");
        let original = state.navigation();
        state.finish_pick(original, Ok((1..=7).map(picked).collect()));
        assert_eq!(state.files().len(), MAX_ATTACHMENTS);
        state.remove_file(2);
        state.activate(Some("b".into()), String::new());
        state.finish_pick(original, Err("stale".into()));
        assert!(state.picker_error().is_none());
        assert!(state.files().is_empty());
        state.activate(Some("a".into()), String::new());
        state.finish_pick(original, Ok(vec![picked(8)]));
        assert_eq!(state.files().len(), MAX_ATTACHMENTS - 1);
        assert!(state.files().iter().all(|file| file.payload.id != 2));
        let before_reset = state.navigation();
        state.reset();
        state.activate(Some("a".into()), String::new());
        state.finish_pick(before_reset, Ok(vec![picked(9)]));
        assert!(state.files().is_empty());
    }

    #[test]
    fn attachment_only_retry_resumes_prefix_and_spoiler_changes_identity() {
        let mut state = active("a", " ");
        state.finish_pick(state.navigation(), Ok(vec![picked(1), picked(2)]));
        let first = state.begin(|| "first".into()).unwrap();
        assert!(first.content.is_empty());
        assert_eq!(first.files.len(), 2);
        state.complete(
            "a",
            first.id,
            Some("failed".into()),
            vec!["uploaded".into()],
        );
        let retry = state.begin(|| "unused".into()).unwrap();
        assert_eq!(retry.key, first.key);
        assert_eq!(retry.uploaded, ["uploaded"]);
        state.complete(
            "a",
            retry.id,
            Some("uncertain".into()),
            vec!["unexpected-replacement".into()],
        );
        let resumed = state.begin(|| "unused".into()).unwrap();
        assert_eq!(resumed.uploaded, ["uploaded"]);
        state.complete("a", resumed.id, Some("failed".into()), resumed.uploaded);
        state.set_spoiler(2, true);
        let changed = state.begin(|| "changed".into()).unwrap();
        assert_eq!(changed.key, "changed");
        assert!(changed.uploaded.is_empty());
        assert!(changed.files[1].spoiler);
        assert!(
            state
                .complete("a", changed.id, None, vec![])
                .unwrap()
                .cleared
        );
        assert!(state.files().is_empty());
    }

    #[test]
    fn channels_send_independently_and_completion_preserves_new_attachments() {
        let mut state = active("a", "hello");
        state.finish_pick(state.navigation(), Ok(vec![picked(1)]));
        let a = state.begin(|| "a".into()).unwrap();
        state.remove_file(1);
        state.finish_pick(state.navigation(), Ok(vec![picked(2)]));
        state.activate(Some("b".into()), "hello".into());
        state.edit("second".into());
        let b = state.begin(|| "b".into()).unwrap();
        assert!(state.complete("b", a.id, None, vec![]).is_none());
        assert!(!state.complete("a", a.id, None, vec![]).unwrap().cleared);
        assert!(state.sending_here());
        assert!(state.complete("b", b.id, None, vec![]).unwrap().cleared);
        state.activate(Some("a".into()), String::new());
        assert_eq!(state.value(), "hello");
        assert_eq!(state.files()[0].payload.id, 2);
    }
}
