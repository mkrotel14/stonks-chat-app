//! Matches Solid timelineCache.ts: 12 channels, 15-second freshness, saved viewport.
use gpui_kit::ListOffset;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use stonks_core::messages::models::TimelineMessage;
pub(in super::super) struct CachedChannel {
    pub messages: Vec<TimelineMessage>,
    pub has_more: bool,
    pub oldest_cursor: Option<String>,
    pub offset: ListOffset,
    pub tail: bool,
}
#[derive(Default)]
pub(in super::super) struct ChannelCache(VecDeque<(String, Instant, CachedChannel)>);
impl ChannelCache {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn invalidate(&mut self, id: &str) {
        self.0.retain(|(key, _, _)| key != id);
    }
    pub fn store(&mut self, id: String, value: CachedChannel) {
        self.invalidate(&id);
        // Bound retained history as well as channel count after long paging sessions.
        if value.messages.len() > 1000 {
            return;
        }
        self.0.push_back((id, Instant::now(), value));
        while self.0.len() > 12 {
            self.0.pop_front();
        }
    }
    pub fn take(&mut self, id: &str) -> Option<CachedChannel> {
        let now = Instant::now();
        self.0
            .retain(|(_, at, _)| now.duration_since(*at) < Duration::from_secs(15));
        let index = self.0.iter().position(|(key, _, _)| key == id)?;
        self.0.remove(index).map(|(_, _, value)| value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> CachedChannel {
        CachedChannel {
            messages: vec![],
            has_more: true,
            oldest_cursor: Some("oldest".into()),
            offset: ListOffset::default(),
            tail: false,
        }
    }
    #[test]
    fn restore_and_live_invalidation() {
        let mut cache = ChannelCache::default();
        cache.store("a".into(), page());
        let restored = cache.take("a").unwrap();
        assert!(!restored.tail);
        assert_eq!(restored.oldest_cursor.as_deref(), Some("oldest"));
        cache.store("a".into(), restored);
        cache.invalidate("a");
        assert!(cache.take("a").is_none());
    }
    #[test]
    fn expires_and_evicts_oldest() {
        let mut cache = ChannelCache::default();
        for i in 0..13 {
            cache.store(i.to_string(), page());
        }
        assert!(cache.take("0").is_none());
        cache.0.front_mut().unwrap().1 = Instant::now() - Duration::from_secs(16);
        assert!(cache.take("1").is_none());
        assert!(cache.take("12").is_some());
        cache.clear();
        assert!(cache.0.is_empty());
    }
}
