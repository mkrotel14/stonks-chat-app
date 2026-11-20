use super::{
    host::MusicCommand,
    protocol::{MusicQueueItem, MusicSnapshot},
    queue::{BatchInsert, BatchInsertError, QueueFull, QueueInsert},
    resolver::ResolvedMedia,
};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicU8, Arc},
};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

pub(in crate::voice) struct MusicRuntime {
    pub(super) updates: tokio::sync::watch::Sender<MusicSnapshot>,
    pub(super) publisher_gate: Arc<Mutex<()>>,
    pub(super) provisional_host: bool,
    pub(super) reconnect_host: Option<MusicSnapshot>,
    pub(super) publisher: Option<(livekit::prelude::TrackSid, JoinHandle<()>)>,
    pub(super) output_volume: u8,
    pub(super) voice: Option<tokio::sync::watch::Receiver<super::VoiceSnapshot>>,
    pub(in crate::voice) snapshot: MusicSnapshot,
    pub(in crate::voice) commands: Option<mpsc::UnboundedSender<MusicCommand>>,
    pub(in crate::voice) generation: u64,
    pub(in crate::voice) local_gain: Arc<AtomicU8>,
    pub(super) owned_items: HashMap<String, MusicQueueItem>,
    pub(super) owned_sources: HashMap<String, (String, bool)>,
    pub(super) prepared_media: HashMap<String, ResolvedMedia>,
    pub(super) host_task: Option<JoinHandle<()>>,
    pub(super) resolver_tasks: HashMap<String, JoinHandle<()>>,
    pub(super) resolver_generations: HashMap<String, u64>,
    pub(super) next_resolver_generation: u64,
    pub(super) import_generation: u64,
    pub(super) pending_additions:
        HashMap<String, tokio::sync::oneshot::Sender<Result<QueueInsert, QueueFull>>>,
    pub(super) pending_imports:
        HashMap<String, tokio::sync::oneshot::Sender<Result<BatchInsert, BatchInsertError>>>,
    pub(super) pending_seeks: HashMap<String, tokio::sync::oneshot::Sender<bool>>,
    pub(super) pcm_sender: Option<mpsc::Sender<super::publisher::MusicPcmCommand>>,
    pub(super) output_device: Option<String>,
}

impl Default for MusicRuntime {
    fn default() -> Self {
        Self {
            updates: tokio::sync::watch::channel(MusicSnapshot::default()).0,
            publisher_gate: Arc::default(),
            provisional_host: false,
            reconnect_host: None,
            publisher: None,
            voice: None,
            output_volume: 100,
            snapshot: MusicSnapshot::default(),
            commands: None,
            generation: 0,
            local_gain: Arc::new(AtomicU8::new(100)),
            owned_items: HashMap::new(),
            owned_sources: HashMap::new(),
            prepared_media: HashMap::new(),
            host_task: None,
            resolver_tasks: HashMap::new(),
            resolver_generations: HashMap::new(),
            next_resolver_generation: 0,
            import_generation: 0,
            pending_additions: HashMap::new(),
            pending_imports: HashMap::new(),
            pending_seeks: HashMap::new(),
            pcm_sender: None,
            output_device: None,
        }
    }
}

impl MusicRuntime {
    pub(super) fn set_snapshot(&mut self, state: MusicSnapshot) {
        self.provisional_host = false;
        if !state.active
            || self.reconnect_host.as_ref().is_some_and(|before| {
                before.session_id != state.session_id || before.host_id != state.host_id
            })
        {
            self.reconnect_host = None;
        }
        self.snapshot = state;
        self.notify();
    }

    pub(super) fn notify(&self) {
        self.updates.send_replace(self.snapshot.clone());
    }

    pub(in crate::voice) fn stop_tasks(&mut self) {
        self.generation = self.generation.saturating_add(1);
        if let Some(task) = self.host_task.take() {
            task.abort();
        }
        for (_, task) in self.resolver_tasks.drain() {
            task.abort();
        }
        self.resolver_generations.clear();
        self.pending_additions.clear();
        self.pending_imports.clear();
        self.pending_seeks.clear();
        self.commands = None;
    }
}

pub(super) async fn discard_requested_item(runtime: &Arc<Mutex<MusicRuntime>>, item_id: &str) {
    let mut runtime = runtime.lock().await;
    runtime.owned_items.remove(item_id);
    runtime.owned_sources.remove(item_id);
    runtime.prepared_media.remove(item_id);
    runtime.resolver_generations.remove(item_id);
    runtime.pending_additions.remove(item_id);
    if let Some(task) = runtime.resolver_tasks.remove(item_id) {
        task.abort();
    }
}

pub(super) fn import_observation_is_current(
    runtime: &MusicRuntime,
    import_generation: u64,
    host_generation: u64,
    session_id: &str,
    revision: u64,
) -> bool {
    runtime.import_generation == import_generation
        && runtime.generation == host_generation
        && runtime.snapshot.session_id == session_id
        && runtime.snapshot.revision == revision
}

pub(super) fn resolver_generation_is_current(
    runtime: &MusicRuntime,
    item_id: &str,
    resolver_generation: u64,
    host_generation: u64,
) -> bool {
    runtime.generation == host_generation
        && runtime.resolver_generations.get(item_id) == Some(&resolver_generation)
        && runtime.owned_sources.contains_key(item_id)
}
