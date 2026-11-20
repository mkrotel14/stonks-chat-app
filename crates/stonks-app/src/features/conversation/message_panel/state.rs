use std::collections::HashSet;
use stonks_core::{
    connection::GatewayEvent,
    messages::models::{MessagePage, TimelineMessage},
};

#[derive(Default)]
pub(super) struct Results {
    pub messages: Vec<TimelineMessage>,
    pub has_more: bool,
    pub cursor: Option<String>,
    pub pending: Vec<GatewayEvent>,
}
impl Results {
    pub fn merge(&mut self, page: MessagePage, channel: &str, before: Option<&str>, pins: bool) {
        let cursor = page.messages.last().map(|message| message.id.clone());
        let mut known: HashSet<_> = self
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let previous = self.messages.len();
        self.messages.extend(
            page.messages.into_iter().filter(|message| {
                message.channel_id == channel && known.insert(message.id.clone())
            }),
        );
        // A server repeating its boundary cannot keep the load-more loop alive.
        self.has_more =
            page.has_more && cursor.as_deref() != before && self.messages.len() > previous;
        self.cursor = cursor;
        for event in std::mem::take(&mut self.pending) {
            self.apply(&event, pins);
        }
    }
    pub fn apply(&mut self, event: &GatewayEvent, pins: bool) {
        let Some(id) = event.data.get("id").and_then(serde_json::Value::as_str) else {
            return;
        };
        if event.event_type == "MessageDelete" {
            self.messages.retain(|message| message.id != id);
            return;
        }
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else {
            return;
        };
        let data = &event.data["data"];
        if let Some(content) = data.get("content") {
            message.content = content.as_str().map(str::to_owned);
        }
        if event
            .data
            .get("clear")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some("Content")))
        {
            message.content = None;
        }
        if let Some(pinned) = data.get("pinned").and_then(serde_json::Value::as_bool) {
            message.pinned = pinned;
        }
        if pins {
            self.messages
                .retain(|message| message.id != id || message.pinned);
        }
    }
}
pub(super) fn snippet(message: &TimelineMessage) -> String {
    message
        .content
        .as_deref()
        .filter(|content| !content.is_empty())
        .map(|content| content.chars().take(180).collect())
        .or_else(|| {
            message
                .attachments
                .first()
                .map(|file| file.filename.clone())
        })
        .unwrap_or_else(|| stonks_ui::i18n::t("messages.attachment"))
}
pub(super) fn timestamp(id: &str) -> Option<i64> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut value = 0_i64;
    for byte in id.as_bytes().get(..10)? {
        value = value
            .checked_mul(32)?
            .checked_add(DIGITS.iter().position(|digit| digit == byte)? as i64)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str) -> TimelineMessage {
        stonks_core::messages::normalize::normalize_message(&serde_json::json!({"_id":id,"channel":"channel","author":"user","content":"hello","pinned":true}),&Default::default(),&Default::default(),None).unwrap()
    }
    #[test]
    fn paging_keeps_backend_order_deduplicates_and_stops_repeated_pages() {
        let mut results = Results::default();
        results.merge(
            MessagePage {
                messages: vec![message("z"), message("a")],
                has_more: true,
            },
            "channel",
            None,
            false,
        );
        results.merge(
            MessagePage {
                messages: vec![message("a"), message("b")],
                has_more: true,
            },
            "channel",
            Some("a"),
            false,
        );
        assert_eq!(
            results
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["z", "a", "b"]
        );
        assert!(results.has_more);
        results.merge(
            MessagePage {
                messages: vec![message("b")],
                has_more: true,
            },
            "channel",
            Some("b"),
            false,
        );
        assert!(!results.has_more);
    }
    #[test]
    fn deletes_and_unpins_during_requests_are_replayed() {
        let mut results = Results::default();
        results.pending.push(GatewayEvent {
            event_type: "MessageDelete".into(),
            data: serde_json::json!({"id":"a"}),
        });
        results.pending.push(GatewayEvent {
            event_type: "MessageUpdate".into(),
            data: serde_json::json!({"id":"b","data":{"pinned":false}}),
        });
        results.merge(
            MessagePage {
                messages: vec![message("a"), message("b"), message("c")],
                has_more: false,
            },
            "channel",
            None,
            true,
        );
        assert_eq!(
            results
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["c"]
        );
    }
    #[test]
    fn snippets_are_unicode_safe_and_wrong_channels_are_excluded() {
        let mut msg = message("a");
        msg.content = Some("☕".repeat(200));
        assert_eq!(snippet(&msg).chars().count(), 180);
        let mut results = Results::default();
        results.merge(
            MessagePage {
                messages: vec![msg],
                has_more: true,
            },
            "other",
            None,
            false,
        );
        assert!(results.messages.is_empty());
        assert!(!results.has_more);
    }
}
