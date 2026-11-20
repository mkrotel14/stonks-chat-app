//! Serializable music state and versioned room packets. Local playback paths never cross this boundary.
use serde::{Deserialize, Serialize, Serializer};
use std::net::IpAddr;

pub(super) const MAX_MUSIC_PACKET_BYTES: usize = 64 * 1024;
pub(super) const MAX_QUEUE_ITEMS: usize = 100;
// Leave enough room for the containing State/Handoff fields and JSON escaping.
pub(super) const MAX_SERIALIZED_QUEUE_BYTES: usize = 32 * 1024;
const MAX_ID_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 512;
const MAX_SESSION_BYTES: usize = 256;
const MAX_URL_BYTES: usize = 2_048;
const MAX_PLAYBACK_SECONDS: u64 = 7 * 24 * 60 * 60;
const PUBLIC_PLAYBACK_ERROR: &str = "This track could not be played.";

fn bounded(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.chars().any(char::is_control)
}

fn public_source(value: &str) -> String {
    if value.is_empty() || value == "Local file" {
        return value.to_owned();
    }
    let Ok(mut parsed) = url::Url::parse(value) else {
        return String::new();
    };
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return String::new();
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_path("/");
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

fn host_is_public(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_unspecified())
        }
        Ok(IpAddr::V6(address)) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local())
        }
        Err(_) => true,
    }
}

pub(super) fn safe_artwork_url(value: &str) -> Option<String> {
    if value.len() > MAX_URL_BYTES {
        return None;
    }
    let mut parsed = url::Url::parse(value).ok()?;
    let host = parsed.host_str()?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !host_is_public(host)
    {
        return None;
    }
    parsed.set_query(None);
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

pub(super) fn public_source_url(value: &str) -> String {
    public_source(value)
}

// Only a provider's canonical public identifier, never a direct stream URL.
pub(super) fn transferable_url(value: &str) -> Option<String> {
    let id = value.strip_prefix("https://www.youtube.com/watch?v=")?;
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
    .then(|| value.to_owned())
}

fn serialize_transfer<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value
        .as_deref()
        .and_then(transferable_url)
        .serialize(serializer)
}

fn serialize_source<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&public_source(value))
}

fn serialize_artwork<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    safe_artwork_url(value.as_deref().unwrap_or_default()).serialize(serializer)
}

fn serialize_public_error<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(if value.is_empty() {
        ""
    } else {
        PUBLIC_PLAYBACK_ERROR
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MusicSnapshot {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub coordinator_id: Option<String>,
    pub active: bool,
    pub host_id: Option<String>,
    #[serde(default)]
    pub current_item_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_transfer"
    )]
    pub transfer_url: Option<String>,
    #[serde(serialize_with = "serialize_source")]
    pub source_url: String,
    #[serde(serialize_with = "serialize_artwork")]
    pub artwork_url: Option<String>,
    pub requested_by: Option<String>,
    pub paused: bool,
    pub loading: bool,
    #[serde(default)]
    pub seeking: bool,
    #[serde(default)]
    pub playback_epoch: u64,
    pub elapsed_seconds: u64,
    pub duration_seconds: Option<u64>,
    #[serde(serialize_with = "serialize_public_error")]
    pub error: String,
    #[serde(default)]
    pub queue: Vec<MusicQueueItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicQueueItem {
    pub id: String,
    pub title: String,
    #[serde(serialize_with = "serialize_source")]
    pub source_url: String,
    #[serde(serialize_with = "serialize_artwork")]
    pub artwork_url: Option<String>,
    pub requested_by: Option<String>,
    #[serde(skip)]
    pub(super) playback_source: String,
    #[serde(skip)]
    pub(super) local_file: bool,
}

impl MusicQueueItem {
    fn is_valid_public(&self) -> bool {
        bounded(&self.id, MAX_ID_BYTES)
            && !self.id.is_empty()
            && bounded(&self.title, MAX_TITLE_BYTES)
            && bounded(&self.source_url, MAX_URL_BYTES)
            && public_source(&self.source_url) == self.source_url
            && self
                .artwork_url
                .as_deref()
                .is_none_or(|value| safe_artwork_url(value).as_deref() == Some(value))
            && self
                .requested_by
                .as_deref()
                .is_some_and(|value| !value.is_empty() && bounded(value, MAX_ID_BYTES))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum MusicPacket {
    State {
        state: MusicSnapshot,
    },
    Add {
        session_id: String,
        item: MusicQueueItem,
    },
    AddResult {
        session_id: String,
        item_id: String,
        accepted: bool,
    },
    AddBatch {
        session_id: String,
        request_id: String,
        observed_revision: u64,
        items: Vec<MusicQueueItem>,
    },
    AddBatchResult {
        session_id: String,
        request_id: String,
        accepted: bool,
        stale: bool,
    },
    Metadata {
        session_id: String,
        item_id: String,
        requester: String,
        title: String,
        #[serde(serialize_with = "serialize_source")]
        source_url: String,
        #[serde(serialize_with = "serialize_artwork")]
        artwork_url: Option<String>,
    },
    Control {
        session_id: String,
        action: String,
        item_id: Option<String>,
    },
    Seek {
        session_id: String,
        item_id: String,
        observed_revision: u64,
        observed_playback_epoch: u64,
        request_id: String,
        target_seconds: u64,
    },
    SeekResult {
        session_id: String,
        item_id: String,
        request_id: String,
        accepted: bool,
        revision: u64,
        playback_epoch: u64,
        elapsed_seconds: u64,
    },
    Handoff {
        state: MusicSnapshot,
        item: MusicQueueItem,
    },
    SyncRequest,
}

pub(super) fn queue_is_bounded(queue: &[MusicQueueItem], current_item_id: Option<&str>) -> bool {
    if queue.len() > MAX_QUEUE_ITEMS
        || serde_json::to_vec(queue)
            .map_or(true, |encoded| encoded.len() > MAX_SERIALIZED_QUEUE_BYTES)
    {
        return false;
    }
    let mut ids = std::collections::HashSet::with_capacity(queue.len());
    queue.iter().all(|item| {
        item.is_valid_public()
            && current_item_id != Some(item.id.as_str())
            && ids.insert(item.id.as_str())
    })
}

impl MusicSnapshot {
    pub(super) fn has_bounded_queue(&self) -> bool {
        queue_is_bounded(&self.queue, self.current_item_id.as_deref())
    }

    fn is_valid_public(&self) -> bool {
        bounded(&self.session_id, MAX_SESSION_BYTES)
            && self
                .coordinator_id
                .as_deref()
                .is_none_or(|value| bounded(value, MAX_ID_BYTES))
            && self
                .host_id
                .as_deref()
                .is_none_or(|value| bounded(value, MAX_ID_BYTES))
            && self
                .current_item_id
                .as_deref()
                .is_none_or(|value| bounded(value, MAX_ID_BYTES))
            && bounded(&self.title, MAX_TITLE_BYTES)
            && self
                .artist
                .as_deref()
                .is_none_or(|value| bounded(value, MAX_TITLE_BYTES))
            && self
                .transfer_url
                .as_deref()
                .is_none_or(|url| transferable_url(url).is_some())
            && bounded(&self.source_url, MAX_URL_BYTES)
            && public_source(&self.source_url) == self.source_url
            && self
                .artwork_url
                .as_deref()
                .is_none_or(|value| safe_artwork_url(value).as_deref() == Some(value))
            && self
                .requested_by
                .as_deref()
                .is_none_or(|value| bounded(value, MAX_ID_BYTES))
            && bounded(&self.error, MAX_TITLE_BYTES)
            && self.elapsed_seconds <= MAX_PLAYBACK_SECONDS
            && self
                .duration_seconds
                .is_none_or(|duration| duration <= MAX_PLAYBACK_SECONDS)
            && (!self.active
                || (!self.session_id.is_empty()
                    && self
                        .host_id
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())))
            && self.has_bounded_queue()
    }
}

impl MusicPacket {
    pub(super) fn is_valid_inbound(&self) -> bool {
        match self {
            Self::State { state } => state.is_valid_public(),
            Self::Add { session_id, item } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && item.is_valid_public()
            }
            Self::AddResult {
                session_id,
                item_id,
                ..
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !item_id.is_empty()
                    && bounded(item_id, MAX_ID_BYTES)
            }
            Self::AddBatch {
                session_id,
                request_id,
                items,
                ..
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !request_id.is_empty()
                    && bounded(request_id, MAX_ID_BYTES)
                    && !items.is_empty()
                    && queue_is_bounded(items, None)
            }
            Self::AddBatchResult {
                session_id,
                request_id,
                accepted,
                stale,
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !request_id.is_empty()
                    && bounded(request_id, MAX_ID_BYTES)
                    && !(*accepted && *stale)
            }
            Self::Metadata {
                session_id,
                item_id,
                requester,
                title,
                source_url,
                artwork_url,
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !item_id.is_empty()
                    && bounded(item_id, MAX_ID_BYTES)
                    && !requester.is_empty()
                    && bounded(requester, MAX_ID_BYTES)
                    && bounded(title, MAX_TITLE_BYTES)
                    && bounded(source_url, MAX_URL_BYTES)
                    && public_source(source_url) == *source_url
                    && artwork_url
                        .as_deref()
                        .is_none_or(|value| safe_artwork_url(value).as_deref() == Some(value))
            }
            Self::Control {
                session_id,
                action,
                item_id,
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && matches!(
                        action.as_str(),
                        "remove" | "skip" | "pause" | "resume" | "stop"
                    )
                    && item_id
                        .as_deref()
                        .is_none_or(|value| !value.is_empty() && bounded(value, MAX_ID_BYTES))
                    && (action == "remove") == item_id.is_some()
            }
            Self::Seek {
                session_id,
                item_id,
                request_id,
                target_seconds,
                ..
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !item_id.is_empty()
                    && bounded(item_id, MAX_ID_BYTES)
                    && !request_id.is_empty()
                    && bounded(request_id, MAX_ID_BYTES)
                    && *target_seconds <= MAX_PLAYBACK_SECONDS
            }
            Self::SeekResult {
                session_id,
                item_id,
                request_id,
                elapsed_seconds,
                ..
            } => {
                !session_id.is_empty()
                    && bounded(session_id, MAX_SESSION_BYTES)
                    && !item_id.is_empty()
                    && bounded(item_id, MAX_ID_BYTES)
                    && !request_id.is_empty()
                    && bounded(request_id, MAX_ID_BYTES)
                    && *elapsed_seconds <= MAX_PLAYBACK_SECONDS
            }
            Self::Handoff { state, item } => {
                state.is_valid_public()
                    && item.is_valid_public()
                    && state.current_item_id.as_deref() == Some(&item.id)
                    && !state.queue.iter().any(|queued| queued.id == item.id)
            }
            Self::SyncRequest => true,
        }
    }
}

pub(super) fn session_priority(state: &MusicSnapshot) -> &str {
    state
        .coordinator_id
        .as_deref()
        .or(state.host_id.as_deref())
        .unwrap_or("􏿿")
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
