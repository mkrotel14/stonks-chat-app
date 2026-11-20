//! Account-scoped memory cache. Active surfaces own pending work, while
//! completed textures survive navigation until bounded LRU eviction.
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLoadState {
    Missing,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    key: String,
    epoch: u64,
    serial: u64,
}

struct Entry<V, W> {
    value: Option<V>,
    bytes: usize,
    pending: bool,
    used: u64,
    ticket: Ticket,
    work: Option<W>,
}

pub struct Cache<V, W> {
    entries: HashMap<String, Entry<V, W>>,
    sources: HashMap<&'static str, HashSet<String>>,
    identity: Option<(String, String)>,
    epoch: u64,
    tick: u64,
    serial: u64,
    max_entries: usize,
    max_bytes: usize,
    max_pending: usize,
}

impl<V: Clone, W> Cache<V, W> {
    pub fn new(max_entries: usize, max_bytes: usize, max_pending: usize) -> Self {
        Self {
            entries: HashMap::new(),
            sources: HashMap::new(),
            identity: None,
            epoch: 0,
            tick: 0,
            serial: 0,
            max_entries,
            max_bytes,
            max_pending,
        }
    }
    pub fn activate(&mut self, identity: Option<(String, String)>) -> bool {
        if self.identity == identity {
            return false;
        }
        self.entries.clear();
        self.sources.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.identity = identity;
        true
    }
    pub fn state(&self, key: &str) -> ImageLoadState {
        match self.entries.get(key) {
            None => ImageLoadState::Missing,
            Some(entry) if entry.pending => ImageLoadState::Loading,
            Some(entry) if entry.value.is_some() => ImageLoadState::Ready,
            _ => ImageLoadState::Failed,
        }
    }
    pub fn get(&mut self, key: &str) -> Option<V> {
        self.tick = self.tick.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.used = self.tick;
        entry.value.clone()
    }
    /// Reuse a completed representation while another size is being decoded.
    pub fn get_matching(&mut self, matches: impl Fn(&str) -> bool) -> Option<V> {
        let key = self
            .entries
            .iter()
            .filter(|(key, entry)| entry.value.is_some() && matches(key))
            .max_by_key(|(_, entry)| entry.used)
            .map(|(key, _)| key.clone())?;
        self.get(&key)
    }
    pub fn pending(&self) -> usize {
        self.entries.values().filter(|entry| entry.pending).count()
    }
    pub fn set_sources(&mut self, scope: &'static str, sources: impl IntoIterator<Item = String>) {
        if self.identity.is_none() {
            return;
        }
        self.sources.insert(scope, sources.into_iter().collect());
        // Navigation cancels stale downloads, not completed browser-equivalent
        // cache entries. Dropping W aborts pending transport/decode ownership.
        self.entries.retain(|key, entry| {
            !entry.pending || self.sources.values().any(|sources| sources.contains(key))
        });
    }
    fn active(&self, key: &str) -> bool {
        self.sources.values().any(|sources| sources.contains(key))
    }
    fn eviction_candidate(&self, keep: &str) -> Option<String> {
        self.entries
            .iter()
            .filter(|(key, entry)| key.as_str() != keep && !entry.pending)
            .min_by_key(|(key, entry)| (self.active(key), entry.used))
            .map(|(key, _)| key.clone())
    }
    pub fn reserve(&mut self, key: String) -> Option<Ticket> {
        if self.identity.is_none()
            || self.entries.contains_key(&key)
            || self.pending() >= self.max_pending
        {
            return None;
        }
        while self.entries.len() >= self.max_entries {
            let oldest = self.eviction_candidate(&key)?;
            self.entries.remove(&oldest);
        }
        self.tick = self.tick.wrapping_add(1);
        self.serial = self.serial.wrapping_add(1);
        let ticket = Ticket {
            key: key.clone(),
            epoch: self.epoch,
            serial: self.serial,
        };
        self.entries.insert(
            key,
            Entry {
                value: None,
                bytes: 0,
                pending: true,
                used: self.tick,
                ticket: ticket.clone(),
                work: None,
            },
        );
        Some(ticket)
    }
    pub fn attach(&mut self, ticket: &Ticket, work: W) {
        if let Some(entry) = self
            .entries
            .get_mut(&ticket.key)
            .filter(|entry| entry.ticket == *ticket && entry.pending)
        {
            entry.work = Some(work);
        }
    }
    pub fn complete(&mut self, ticket: &Ticket, result: Option<(V, usize)>) -> bool {
        if ticket.epoch != self.epoch
            || !self
                .entries
                .get(&ticket.key)
                .is_some_and(|entry| entry.ticket == *ticket && entry.pending)
        {
            return false;
        }
        let result = result.filter(|(_, bytes)| *bytes <= self.max_bytes);
        if let Some((_, bytes)) = &result {
            while self
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                .saturating_add(*bytes)
                > self.max_bytes
            {
                let Some(oldest) = self.eviction_candidate(&ticket.key) else {
                    break;
                };
                self.entries.remove(&oldest);
            }
        }
        self.tick = self.tick.wrapping_add(1);
        let entry = self
            .entries
            .get_mut(&ticket.key)
            .expect("validated pending cache ticket");
        entry.used = self.tick;
        entry.pending = false;
        if let Some((value, bytes)) = result {
            entry.value = Some(value);
            entry.bytes = bytes;
        }
        true
    }
    pub fn retry(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    struct Work(Rc<Cell<usize>>);
    impl Drop for Work {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    fn cache() -> Cache<&'static str, Work> {
        let mut cache = Cache::new(4, 64, 2);
        cache.activate(Some(("deployment".into(), "alice".into())));
        cache
    }
    fn done(cache: &mut Cache<&'static str, Work>, key: &str, value: &'static str, bytes: usize) {
        let ticket = cache.reserve(key.into()).unwrap();
        assert!(cache.complete(&ticket, Some((value, bytes))));
    }
    #[test]
    fn cold_load_deduplicates_and_warm_channel_revisit_needs_no_new_request() {
        let mut cache = cache();
        cache.set_sources("timeline", ["image-a".into()]);
        assert_eq!(cache.state("image-a"), ImageLoadState::Missing);
        let ticket = cache.reserve("image-a".into()).unwrap();
        assert!(cache.reserve("image-a".into()).is_none());
        assert_eq!(cache.state("image-a"), ImageLoadState::Loading);
        cache.complete(&ticket, Some(("decoded pixels", 20)));
        cache.set_sources("timeline", ["image-b".into()]);
        done(&mut cache, "image-b", "other pixels", 20);
        cache.set_sources("timeline", ["image-a".into()]);
        assert_eq!(cache.get("image-a"), Some("decoded pixels"));
        assert!(cache.reserve("image-a".into()).is_none());
    }
    #[test]
    fn warm_hit_preserves_the_same_decoded_texture_allocation() {
        let mut cache = Cache::<std::sync::Arc<str>, Work>::new(4, 64, 2);
        cache.activate(Some(("deployment".into(), "alice".into())));
        cache.set_sources("timeline", ["a".into()]);
        let ticket = cache.reserve("a".into()).unwrap();
        let pixels: std::sync::Arc<str> = "decoded texture".into();
        cache.complete(&ticket, Some((pixels.clone(), 16)));
        cache.set_sources("timeline", ["b".into()]);
        cache.set_sources("timeline", ["a".into()]);
        assert!(std::sync::Arc::ptr_eq(&cache.get("a").unwrap(), &pixels));
    }
    #[test]
    fn another_size_reuses_only_the_same_url_and_account() {
        let mut cache = cache();
        done(&mut cache, "80:https://cdn/avatar", "animated pixels", 12);
        let same_url = |key: &str| {
            key.split_once(':')
                .is_some_and(|(_, url)| url == "https://cdn/avatar")
        };
        let larger = cache.reserve("152:https://cdn/avatar".into()).unwrap();
        assert_eq!(cache.get_matching(same_url), Some("animated pixels"));
        assert_eq!(
            cache.get_matching(|key| key.ends_with("different-avatar")),
            None
        );
        cache.complete(&larger, Some(("larger pixels", 24)));
        assert_eq!(cache.get_matching(same_url), Some("larger pixels"));
        cache.activate(Some(("deployment".into(), "bob".into())));
        assert_eq!(cache.get_matching(same_url), None);
    }
    #[test]
    fn entry_budget_evicts_least_recently_used_completed_texture() {
        let mut cache = cache();
        for (key, value) in [("a", "a"), ("b", "b"), ("c", "c"), ("d", "d")] {
            done(&mut cache, key, value, 1);
        }
        cache.get("a");
        done(&mut cache, "e", "e", 1);
        assert_eq!(cache.state("b"), ImageLoadState::Missing);
        assert_eq!(cache.get("a"), Some("a"));
        assert_eq!(cache.entries.len(), 4);
    }
    #[test]
    fn switching_channels_cancels_only_obsolete_pending_work() {
        let mut cache = cache();
        let drops = Rc::new(Cell::new(0));
        cache.set_sources("timeline", ["ready".into(), "pending".into()]);
        let ready = cache.reserve("ready".into()).unwrap();
        cache.attach(&ready, Work(drops.clone()));
        cache.complete(&ready, Some(("pixels", 20)));
        let old = cache.reserve("pending".into()).unwrap();
        cache.attach(&old, Work(drops.clone()));
        cache.set_sources("timeline", ["next".into()]);
        assert_eq!(drops.get(), 1);
        assert_eq!(cache.get("ready"), Some("pixels"));
        let new = cache.reserve("pending".into()).unwrap();
        assert!(!cache.complete(&old, Some(("stale", 10))));
        assert!(cache.complete(&new, Some(("fresh", 10))));
        assert_eq!(cache.get("pending"), Some("fresh"));
    }
    #[test]
    fn multiple_surfaces_share_work_and_lru_budget_evicts_inactive_first() {
        let mut cache = cache();
        cache.set_sources("members", ["avatar".into()]);
        done(&mut cache, "avatar", "avatar pixels", 24);
        cache.set_sources("timeline", ["old-photo".into()]);
        done(&mut cache, "old-photo", "old pixels", 32);
        cache.get("old-photo"); // newer than avatar, but no longer visible
        cache.set_sources("timeline", ["new-photo".into()]);
        done(&mut cache, "new-photo", "new pixels", 24);
        assert_eq!(cache.get("avatar"), Some("avatar pixels"));
        assert_eq!(cache.state("old-photo"), ImageLoadState::Missing);
        assert_eq!(cache.get("new-photo"), Some("new pixels"));
    }
    #[test]
    fn account_deployment_and_logout_boundaries_clear_pixels_and_invalidate_work() {
        let mut cache = cache();
        let drops = Rc::new(Cell::new(0));
        done(&mut cache, "public-url", "alice pixels", 20);
        let old = cache.reserve("pending".into()).unwrap();
        cache.attach(&old, Work(drops.clone()));
        assert!(cache.activate(Some(("deployment".into(), "bob".into()))));
        assert_eq!(drops.get(), 1);
        assert_eq!(cache.state("public-url"), ImageLoadState::Missing);
        let replacement = cache.reserve("pending".into()).unwrap();
        assert!(!cache.complete(&old, Some(("alice data", 1))));
        cache.complete(&replacement, Some(("bob pixels", 1)));
        cache.activate(Some(("another deployment".into(), "bob".into())));
        assert_eq!(cache.state("pending"), ImageLoadState::Missing);
        cache.activate(None);
        assert!(cache.reserve("public-url".into()).is_none());
    }
    #[test]
    fn failure_requires_retry_and_entry_pending_and_byte_limits_are_bounded() {
        let mut cache = cache();
        let failed = cache.reserve("failed".into()).unwrap();
        cache.complete(&failed, None);
        assert_eq!(cache.state("failed"), ImageLoadState::Failed);
        assert!(cache.reserve("failed".into()).is_none());
        cache.retry("failed");
        assert!(cache.reserve("failed".into()).is_some());
        let oversized = cache.reserve("huge".into()).unwrap();
        assert!(cache.reserve("excess".into()).is_none());
        cache.complete(&oversized, Some(("too large", 65)));
        assert_eq!(cache.state("huge"), ImageLoadState::Failed);
    }
}
