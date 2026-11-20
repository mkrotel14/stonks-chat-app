//! History merge rules, independent of rendering and request timing.
use std::collections::HashSet;
use stonks_core::messages::models::TimelineMessage;

#[derive(Default)]
pub struct PendingEdit {
    values: serde_json::Map<String, serde_json::Value>,
    appended: Vec<stonks_core::messages::models::TimelineEmbed>,
}
impl PendingEdit {
    pub fn update(&mut self, edit: &serde_json::Value) {
        for key in ["content", "edited", "embeds", "pinned"] {
            if let Some(value) = edit.get(key) {
                self.values.insert(key.into(), value.clone());
                if key == "embeds" {
                    self.appended.clear();
                }
            }
        }
    }
    pub fn append(&mut self, embeds: &[serde_json::Value]) {
        self.appended
            .extend(stonks_core::messages::normalize::normalize_embeds(embeds));
    }
    pub fn apply(&self, message: &mut TimelineMessage) {
        apply_edit(message, &serde_json::Value::Object(self.values.clone()));
        message.embeds.extend(self.appended.iter().cloned());
    }
}

pub fn append_embeds(message: &mut TimelineMessage, raw: &[serde_json::Value]) -> bool {
    if raw.is_empty() {
        return false;
    }
    message
        .embeds
        .extend(stonks_core::messages::normalize::normalize_embeds(raw));
    true
}

pub struct Merge {
    pub inserted: usize,
    pub oldest: Option<String>,
}

impl Merge {
    pub(crate) fn can_page_older(&self, full_page: bool, nearby: bool) -> bool {
        // Nearby combines two independently limited sides. Its total length
        // cannot establish exhaustion on the older side; probe with `before`.
        self.oldest.is_some() && (nearby || full_page)
    }
}

/// A full page only suggests older history may exist. If live deletions
/// empty the list, verify that suggestion without discarding the raw cursor.
pub(crate) fn deletion_probe(
    removed: bool,
    remaining: usize,
    has_more: bool,
    cursor: Option<&str>,
) -> Option<&str> {
    if removed && remaining == 0 && has_more {
        cursor
    } else {
        None
    }
}

pub fn apply_edit(message: &mut TimelineMessage, edit: &serde_json::Value) {
    if let Some(pinned) = edit.get("pinned").and_then(serde_json::Value::as_bool) {
        message.pinned = pinned;
    }
    if let Some(embeds) = edit.get("embeds").and_then(serde_json::Value::as_array) {
        message.embeds = stonks_core::messages::normalize::normalize_embeds(embeds);
    }
    for (key, field) in [
        ("content", &mut message.content),
        ("edited", &mut message.edited_at),
    ] {
        match edit.get(key) {
            Some(serde_json::Value::Null) => *field = None,
            Some(serde_json::Value::String(value)) => *field = Some(value.clone()),
            _ => {}
        }
    }
}

/// Preserve live edits/deletes when a REST page overlaps gateway traffic.
/// The raw cursor advances even if every message in a page was deleted.
pub fn merge_page(
    messages: &mut Vec<TimelineMessage>,
    incoming: Vec<TimelineMessage>,
    channel: &str,
    before: Option<&str>,
    deleted: &HashSet<String>,
) -> Merge {
    let mut seen: HashSet<String> = messages.iter().map(|message| message.id.clone()).collect();
    let mut oldest: Option<String> = None;
    let mut added = Vec::new();
    for message in incoming {
        if message.channel_id != channel
            || before.is_some_and(|before| message.id.as_str() >= before)
        {
            continue;
        }
        if oldest.as_ref().is_none_or(|oldest| message.id < *oldest) {
            oldest = Some(message.id.clone());
        }
        if !(message.system && message.system_type.as_deref() == Some("call_started"))
            && !deleted.contains(&message.id)
            && seen.insert(message.id.clone())
        {
            added.push(message);
        }
    }
    let inserted = added.len();
    messages.extend(added);
    messages.sort_by(|a, b| a.id.cmp(&b.id));
    Merge { inserted, oldest }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, FollowMode, ListAlignment, ListOffset, ListState};
    use std::collections::HashMap;
    use stonks_core::messages::normalize::normalize_message;
    fn message(n: u32, content: &str) -> TimelineMessage {
        normalize_message(&serde_json::json!({"_id": format!("{n:026}"), "channel": "channel", "author": "author", "content": content}), &HashMap::new(), &HashMap::new(), None).unwrap()
    }
    #[test]
    fn short_nearby_window_can_load_older_until_a_before_page_exhausts_history() {
        let mut messages = Vec::new();
        let nearby = merge_page(
            &mut messages,
            vec![
                message(6, "reply"),
                message(4, "preceding"),
                message(5, "target"),
            ],
            "channel",
            None,
            &HashSet::new(),
        );
        assert_eq!(messages[0].content.as_deref(), Some("preceding"));
        assert!(nearby.can_page_older(false, true));
        let older = merge_page(
            &mut messages,
            vec![message(3, "older")],
            "channel",
            nearby.oldest.as_deref(),
            &HashSet::new(),
        );
        assert_eq!(messages[0].content.as_deref(), Some("older"));
        assert!(!older.can_page_older(false, false));
        assert!(!Merge {
            inserted: 0,
            oldest: None
        }
        .can_page_older(false, true));
    }

    #[test]
    fn older_pages_sort_deduplicate_and_preserve_live_edits() {
        let mut messages = vec![message(5, "live edit"), message(6, "live message")];
        let before = format!("{:026}", 5);
        let result = merge_page(
            &mut messages,
            vec![
                message(5, "stale"),
                message(4, "older"),
                message(3, "oldest"),
                message(4, "duplicate"),
            ],
            "channel",
            Some(&before),
            &HashSet::new(),
        );
        assert_eq!(result.inserted, 2);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["oldest", "older", "live edit", "live message"]
        );
        assert_eq!(result.oldest, Some(format!("{:026}", 3)));
    }
    #[test]
    fn initial_page_does_not_overwrite_live_updates_or_resurrect_deletes() {
        let mut messages = vec![message(5, "live edit"), message(6, "new")];
        let deleted = HashSet::from([format!("{:026}", 4)]);
        merge_page(
            &mut messages,
            vec![
                message(5, "stale"),
                message(4, "deleted"),
                message(3, "old"),
            ],
            "channel",
            None,
            &deleted,
        );
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].content.as_deref(), Some("live edit"));
    }

    #[test]
    fn gateway_edits_before_a_page_arrives_replace_stale_content() {
        let mut incoming = message(4, "stale REST content");
        apply_edit(
            &mut incoming,
            &serde_json::json!({"content": "live edit", "edited": "now"}),
        );
        let mut messages = vec![message(5, "newer")];
        merge_page(
            &mut messages,
            vec![incoming],
            "channel",
            Some(&format!("{:026}", 5)),
            &HashSet::new(),
        );
        assert_eq!(messages[0].content.as_deref(), Some("live edit"));
        assert_eq!(messages[0].edited_at.as_deref(), Some("now"));
        apply_edit(&mut messages[0], &serde_json::json!({"content": null}));
        assert!(messages[0].content.is_none());
        assert_eq!(messages[0].edited_at.as_deref(), Some("now"));
        apply_edit(
            &mut messages[0],
            &serde_json::json!({"embeds":[{"type":"Website","title":"Live preview","url":"https://example.com"}]}),
        );
        assert_eq!(messages[0].embeds[0].title.as_deref(), Some("Live preview"));
        apply_edit(&mut messages[0], &serde_json::json!({"embeds":[]}));
        assert!(messages[0].embeds.is_empty());
    }

    #[test]
    fn appended_embeds_preserve_existing_previews_and_reconcile_rest_races_in_event_order() {
        let embed =
            |title| serde_json::json!({"type":"Website","title":title,"url":"https://example.com"});
        let mut current = message(1, "url");
        append_embeds(&mut current, &[embed("existing")]);
        assert!(append_embeds(&mut current, &[embed("async preview")]));
        assert_eq!(
            current
                .embeds
                .iter()
                .map(|embed| embed.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["existing", "async preview"]
        );
        let mut pending = PendingEdit::default();
        pending.append(&[embed("discarded by update")]);
        pending.update(&serde_json::json!({"embeds":[embed("replacement")],"content":"edited"}));
        pending.append(&[embed("later append")]);
        let mut rest = message(1, "stale response");
        append_embeds(&mut rest, &[embed("stale preview")]);
        pending.apply(&mut rest);
        assert_eq!(rest.content.as_deref(), Some("edited"));
        assert_eq!(
            rest.embeds
                .iter()
                .map(|embed| embed.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["replacement", "later append"]
        );
        assert!(!append_embeds(&mut rest, &[]));
    }

    #[test]
    fn hidden_call_rows_advance_paging_without_affecting_visible_grouping() {
        let mut call = message(4, "call");
        call.system = true;
        call.system_type = Some("call_started".into());
        let mut messages = vec![message(5, "visible")];
        let merge = merge_page(
            &mut messages,
            vec![call],
            "channel",
            Some(&format!("{:026}", 5)),
            &HashSet::new(),
        );
        assert_eq!(merge.inserted, 0);
        assert_eq!(merge.oldest, Some(format!("{:026}", 4)));
        assert_eq!(messages.len(), 1);
    }
    #[test]
    fn deleted_pages_still_advance_but_invalid_or_repeated_pages_stop() {
        let before = format!("{:026}", 5);
        let mut messages = vec![message(5, "new")];
        let result = merge_page(
            &mut messages,
            vec![message(4, "deleted")],
            "channel",
            Some(&before),
            &HashSet::from([format!("{:026}", 4)]),
        );
        assert_eq!(result.inserted, 0);
        assert_eq!(result.oldest, Some(format!("{:026}", 4)));
        let result = merge_page(
            &mut messages,
            vec![message(5, "overlap"), message(3, "wrong channel")],
            "another-channel",
            Some(&before),
            &HashSet::new(),
        );
        assert!(result.oldest.is_none());
    }

    #[test]
    fn deleting_the_last_visible_row_checks_the_existing_history_cursor() {
        let cursor = Some("oldest");
        assert_eq!(deletion_probe(true, 0, true, cursor), cursor);
        assert_eq!(deletion_probe(true, 1, true, cursor), None);
        assert_eq!(deletion_probe(true, 0, false, cursor), None);
        assert_eq!(deletion_probe(true, 0, true, None), None);
        // A filtered page alone is not a live deletion and remains pageable.
        assert_eq!(deletion_probe(false, 0, true, cursor), None);
    }

    #[test]
    fn prepending_and_remeasuring_preserve_the_same_row_and_pixel_offset() {
        let list = ListState::new(51, ListAlignment::Bottom, px(300.));
        list.scroll_to(ListOffset {
            item_ix: 7,
            offset_in_item: px(23.),
        });
        list.splice(1..1, 50);
        list.remeasure_items(51..52);
        let anchor = list.logical_scroll_top();
        assert_eq!(anchor.item_ix, 57);
        assert_eq!(anchor.offset_in_item, px(23.));
        list.splice(4..5, 0);
        assert_eq!(list.logical_scroll_top().item_ix, 56);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(23.));
    }

    #[test]
    fn reading_history_pauses_tail_following_and_new_messages_keep_the_anchor() {
        let list = ListState::new(51, ListAlignment::Bottom, px(300.));
        list.set_follow_mode(FollowMode::Tail);
        assert!(list.is_following_tail());
        list.scroll_to(ListOffset {
            item_ix: 10,
            offset_in_item: px(9.),
        });
        assert!(!list.is_following_tail());
        list.splice(51..51, 1);
        assert_eq!(list.logical_scroll_top().item_ix, 10);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(9.));
    }
}
