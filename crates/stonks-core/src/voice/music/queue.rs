//! Queue item construction and ownership rules, independent of the LiveKit host worker.
use super::protocol::{public_source_url, queue_is_bounded, MusicQueueItem};
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn queue_item(source: String, requester: String, local_file: bool) -> MusicQueueItem {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let title = if local_file {
        PathBuf::from(&source)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Local audio")
            .to_owned()
    } else {
        url::Url::parse(&source)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .unwrap_or_else(|| "Queued track".into())
    };
    MusicQueueItem {
        id: format!("{requester}:{id}"),
        title,
        source_url: if local_file {
            "Local file".into()
        } else {
            public_source_url(&source)
        },
        artwork_url: None,
        requested_by: Some(requester),
        playback_source: source,
        local_file,
    }
}

pub(super) fn item_owner(item: &MusicQueueItem) -> Option<&str> {
    item.requested_by.as_deref()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::voice) enum QueueInsert {
    Added,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::voice) struct QueueFull;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::voice) enum BatchInsert {
    Added,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::voice) enum BatchInsertError {
    Full,
    Stale,
}

pub(super) fn insert_item(
    queue: &mut VecDeque<MusicQueueItem>,
    current_item_id: Option<&str>,
    item: MusicQueueItem,
) -> Result<QueueInsert, QueueFull> {
    if current_item_id == Some(&item.id) || queue.iter().any(|queued| queued.id == item.id) {
        return Ok(QueueInsert::Duplicate);
    }
    queue.push_back(item);
    if queue_is_bounded(queue.make_contiguous(), current_item_id) {
        Ok(QueueInsert::Added)
    } else {
        queue.pop_back();
        Err(QueueFull)
    }
}

pub(super) fn insert_batch(
    queue: &mut VecDeque<MusicQueueItem>,
    current_item_id: Option<&str>,
    items: Vec<MusicQueueItem>,
) -> Result<BatchInsert, BatchInsertError> {
    if items.is_empty() {
        return Err(BatchInsertError::Full);
    }
    let existing = queue
        .iter()
        .map(|item| item.id.as_str())
        .chain(current_item_id)
        .collect::<std::collections::HashSet<_>>();
    let incoming = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if incoming.len() != items.len() || incoming.iter().any(|id| existing.contains(id)) {
        return Ok(BatchInsert::Duplicate);
    }

    let original_len = queue.len();
    queue.extend(items);
    if queue_is_bounded(queue.make_contiguous(), current_item_id) {
        Ok(BatchInsert::Added)
    } else {
        queue.truncate(original_len);
        Err(BatchInsertError::Full)
    }
}

pub(super) fn update_item_metadata(
    queue: &mut VecDeque<MusicQueueItem>,
    current_item_id: Option<&str>,
    id: &str,
    requester: &str,
    title: String,
    source_url: String,
    artwork_url: Option<String>,
) {
    let Some(index) = queue
        .iter()
        .position(|item| item.id == id && item_owner(item) == Some(requester))
    else {
        return;
    };
    let previous = queue[index].clone();
    queue[index].title = title;
    queue[index].source_url = source_url;
    queue[index].artwork_url = artwork_url;
    if !queue_is_bounded(queue.make_contiguous(), current_item_id) {
        queue[index] = previous;
    }
}

pub(super) fn remove_owner_items(queue: &mut VecDeque<MusicQueueItem>, participant_id: &str) {
    queue.retain(|item| item_owner(item) != Some(participant_id));
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
