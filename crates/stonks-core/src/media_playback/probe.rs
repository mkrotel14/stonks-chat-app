//! Bounded, cancellation-owned metadata preload for source <audio preload="metadata">.
//! It cannot activate/replace a Player, render video, or address an audio device.
use super::{resolve_source, worker, AudioMetadata, MediaKind, PlaybackError, Source};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    thread,
    time::Duration,
};
struct ProbeWorker {
    cancelled: Arc<AtomicBool>,
    worker: thread::JoinHandle<()>,
}
impl Drop for ProbeWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.worker.thread().unpark();
    }
}
pub async fn probe_audio(source: Source) -> Result<AudioMetadata, PlaybackError> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    probe_with_slots(
        source,
        SLOTS
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
            .clone(),
    )
    .await
}
pub(super) async fn probe_with_slots(
    mut source: Source,
    slots: Arc<tokio::sync::Semaphore>,
) -> Result<AudioMetadata, PlaybackError> {
    if source.kind != MediaKind::Audio {
        return Err(PlaybackError::Unsupported);
    }
    source.autoplay = false;
    tokio::time::timeout(Duration::from_secs(25), async move {
        // Bound both resolver requests and GStreamer prerolls, even when rows
        // are rapidly created/cancelled while another probe is shutting down.
        let permit = slots
            .acquire_owned()
            .await
            .map_err(|_| PlaybackError::Engine)?;
        let source = resolve_source(source).await?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let shutdown = cancelled.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let worker = thread::Builder::new()
            .name("stonks-audio-metadata".into())
            .spawn(move || {
                let result = worker::probe_audio(source, &shutdown)
                    .map(|duration| AudioMetadata { duration });
                drop(permit);
                let _ = sender.send(result);
            })
            .map_err(|_| PlaybackError::Engine)?;
        let _worker = ProbeWorker { cancelled, worker };
        receiver.await.map_err(|_| PlaybackError::Engine)?
    })
    .await
    .map_err(|_| PlaybackError::Unavailable)?
}
