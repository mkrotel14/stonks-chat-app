use super::{
    protocol::{queue_is_bounded, MusicPacket, MusicQueueItem, MusicSnapshot},
    publisher::{MusicPcmCommand, MUSIC_FRAME_SAMPLES},
    queue::{
        insert_batch, insert_item, item_owner, remove_owner_items, update_item_metadata,
        BatchInsert, BatchInsertError, QueueFull, QueueInsert,
    },
    resolver::{resolve_media, tool_path},
    runtime::MusicRuntime,
    transport::publish_packet,
    *,
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::atomic::Ordering,
};
use tokio::io::AsyncReadExt;
#[cfg(target_os = "linux")]
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const QUEUE_FULL_ERROR: &str = "The shared music queue is full (100 tracks).";

#[derive(Debug, Clone)]
pub(in crate::voice) struct MusicSeekRequest {
    pub(super) session_id: String,
    pub(super) item_id: String,
    pub(super) requester_id: String,
    pub(super) observed_revision: u64,
    pub(super) observed_playback_epoch: u64,
    pub(super) request_id: String,
    pub(super) target_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::voice) struct MusicSeekOutcome {
    pub(super) revision: u64,
    pub(super) playback_epoch: u64,
    pub(super) elapsed_seconds: u64,
}

const MAX_PROCESSED_SEEKS: usize = 128;
type SeekRequestKey = (String, String);

#[derive(Default)]
pub(super) struct ProcessedSeeks {
    outcomes: HashMap<SeekRequestKey, MusicSeekOutcome>,
    order: VecDeque<SeekRequestKey>,
}

impl ProcessedSeeks {
    pub(super) fn outcome(&self, request: &MusicSeekRequest) -> Option<MusicSeekOutcome> {
        self.outcomes
            .get(&(request.requester_id.clone(), request.request_id.clone()))
            .copied()
    }

    pub(super) fn record(&mut self, request: &MusicSeekRequest, outcome: MusicSeekOutcome) {
        let key = (request.requester_id.clone(), request.request_id.clone());
        if self.outcomes.contains_key(&key) {
            return;
        }
        self.outcomes.insert(key.clone(), outcome);
        self.order.push_back(key);
        if self.order.len() > MAX_PROCESSED_SEEKS {
            if let Some(expired) = self.order.pop_front() {
                self.outcomes.remove(&expired);
            }
        }
    }
}

#[derive(Debug)]
pub(in crate::voice) enum MusicCommand {
    Add {
        item: MusicQueueItem,
        result: Option<tokio::sync::oneshot::Sender<Result<QueueInsert, QueueFull>>>,
    },
    AddBatch {
        observed_revision: u64,
        items: Vec<MusicQueueItem>,
        result: tokio::sync::oneshot::Sender<Result<BatchInsert, BatchInsertError>>,
    },
    UpdateMetadata {
        id: String,
        requester: String,
        title: String,
        source_url: String,
        artwork_url: Option<String>,
    },
    Remove(String),
    ParticipantLeft(String),
    Skip,
    Pause,
    Resume,
    Seek {
        request: MusicSeekRequest,
        result: tokio::sync::oneshot::Sender<Result<MusicSeekOutcome, ()>>,
    },
    Stop,
    Supersede,
}

pub(super) fn seek_target(
    request: &MusicSeekRequest,
    session_id: &str,
    item_id: &str,
    revision: u64,
    playback_epoch: u64,
    duration_seconds: Option<u64>,
) -> Option<u64> {
    let duration = duration_seconds?;
    (request.session_id == session_id
        && request.item_id == item_id
        && request.observed_revision <= revision
        && request.observed_playback_epoch == playback_epoch
        && duration > 1
        && request.target_seconds < duration)
        .then_some(request.target_seconds)
}

pub(super) fn participant_is_present(room: &Room, participant_id: &str) -> bool {
    room.local_participant().identity().to_string() == participant_id
        || room
            .remote_participants()
            .keys()
            .any(|identity| identity.to_string() == participant_id)
}

pub(super) fn commit_host_state(
    runtime: &mut MusicRuntime,
    generation: u64,
    state: MusicSnapshot,
) -> Option<MusicSnapshot> {
    if runtime.generation != generation || !state.has_bounded_queue() {
        return None;
    }
    runtime.set_snapshot(state.clone());
    Some(state)
}

async fn broadcast_host_state(
    room: &Room,
    runtime: &Arc<Mutex<MusicRuntime>>,
    generation: u64,
    identity: (&str, &str),
    revision: &mut u64,
    mut state: MusicSnapshot,
) -> bool {
    let (session_id, coordinator_id) = identity;
    *revision = revision.saturating_add(1);
    state.session_id = session_id.to_owned();
    state.revision = *revision;
    state.coordinator_id = Some(coordinator_id.to_owned());
    state.host_id = Some(room.local_participant().identity().to_string());
    let state = {
        let mut locked = runtime.lock().await;
        let Some(state) = commit_host_state(&mut locked, generation, state) else {
            return false;
        };
        if locked.reconnect_host.is_some() {
            // Keep the worker alive, but don't advertise its timeline until
            // peers have had a chance to report a successor.
            return false;
        }
        state
    };
    if runtime.lock().await.generation != generation {
        return false;
    }
    publish_packet(room, &MusicPacket::State { state })
        .await
        .is_ok()
}

#[cfg(target_os = "linux")]
async fn open_local_output(device: &str) -> Option<tokio::process::Child> {
    let mut command = Command::new("pw-cat");
    command
        .args([
            "--playback",
            "--properties",
            "application.name=Stonks application.id=com.rdxlabs.stonks",
            "--raw",
            "--rate",
            "48000",
            "--channels",
            "1",
            "--format",
            "s16",
            "--latency",
            "20ms",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if device != "default" {
        command.args([
            "--target",
            device.strip_prefix("pipewire:").unwrap_or(device),
        ]);
    }
    command.arg("-").spawn().ok()
}

pub(super) fn apply_batch_command(
    queue: &mut VecDeque<MusicQueueItem>,
    current_item_id: Option<&str>,
    revision: u64,
    observed_revision: u64,
    items: Vec<MusicQueueItem>,
    result: tokio::sync::oneshot::Sender<Result<BatchInsert, BatchInsertError>>,
) {
    let insertion = if revision == observed_revision {
        insert_batch(queue, current_item_id, items)
    } else {
        Err(BatchInsertError::Stale)
    };
    let _ = result.send(insertion);
}

pub(super) fn apply_add_command(
    queue: &mut VecDeque<MusicQueueItem>,
    current_item_id: Option<&str>,
    item: MusicQueueItem,
    result: Option<tokio::sync::oneshot::Sender<Result<QueueInsert, QueueFull>>>,
) {
    let insertion = insert_item(queue, current_item_id, item);
    if let Some(result) = result {
        let _ = result.send(insertion);
    }
}

fn decoder_command(
    ffmpeg: &std::path::Path,
    input: &str,
    local_file: bool,
    elapsed_samples: u64,
) -> Command {
    let mut child = Command::new(ffmpeg);
    #[cfg(target_os = "windows")]
    child.creation_flags(0x08000000); // CREATE_NO_WINDOW
    child.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-re"]);
    if elapsed_samples > 0 {
        child.args(["-ss", &format!("{:.3}", elapsed_samples as f64 / 48_000.0)]);
    }
    // FFmpeg rejects HTTP protocol options when the input is a local file.
    if !local_file {
        child.args([
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "2",
        ]);
    }
    child
        .args([
            "-i", input, "-vn", "-ac", "1", "-ar", "48000", "-f", "s16le", "pipe:1",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    child
}

struct HostWorker {
    room: Arc<Room>,
    runtime: Arc<Mutex<MusicRuntime>>,
    pcm_sender: mpsc::Sender<MusicPcmCommand>,
    commands: mpsc::UnboundedReceiver<MusicCommand>,
    generation: u64,
    session_id: String,
    coordinator_id: String,
    revision: u64,
    first_item: MusicQueueItem,
    remaining_queue: Vec<MusicQueueItem>,
    ffmpeg: PathBuf,
    output_device: String,
}

async fn run_host(worker: HostWorker) {
    let HostWorker {
        room,
        runtime,
        pcm_sender,
        mut commands,
        generation,
        session_id,
        coordinator_id,
        mut revision,
        first_item,
        remaining_queue,
        ffmpeg,
        output_device,
    } = worker;
    let host_id = room.local_participant().identity().to_string();
    let (local_gain, voice, output_volume) = {
        let locked = runtime.lock().await;
        (
            locked.local_gain.clone(),
            locked.voice.clone(),
            locked.output_volume,
        )
    };
    let mut queue = VecDeque::from([first_item]);
    queue.extend(remaining_queue);
    let mut stopped = false;
    let mut superseded = false;
    let mut final_error = String::new();
    // This output belongs to the playback session, not an individual song.
    // Recreating it at each queue boundary makes PipeWire renegotiate the
    // route, which is the audible pause between otherwise continuous tracks.
    #[cfg(target_os = "linux")]
    let mut local_output = open_local_output(&output_device).await;
    #[cfg(target_os = "windows")]
    let mut local_output = tokio::task::spawn_blocking(move || {
        let device = crate::voice::devices::select(&output_device, false)?;
        let mut sink = rodio::DeviceSinkBuilder::from_device(device)
            .ok()?
            .open_stream()
            .ok()?;
        sink.log_on_drop(false);
        let player = rodio::Player::connect_new(sink.mixer());
        Some((sink, player))
    })
    .await
    .ok()
    .flatten();
    loop {
        // A track can finish (or fail during URL resolution) at the same moment
        // another participant adds to the queue. Drain those reliable commands
        // before deciding that the playlist is over.
        while let Ok(command) = commands.try_recv() {
            match command {
                MusicCommand::Add { item, result } => {
                    apply_add_command(&mut queue, None, item, result);
                }
                MusicCommand::AddBatch {
                    observed_revision,
                    items,
                    result,
                } => apply_batch_command(
                    &mut queue,
                    None,
                    revision,
                    observed_revision,
                    items,
                    result,
                ),
                MusicCommand::UpdateMetadata {
                    id,
                    requester,
                    title,
                    source_url,
                    artwork_url,
                } => update_item_metadata(
                    &mut queue,
                    None,
                    &id,
                    &requester,
                    title,
                    source_url,
                    artwork_url,
                ),
                MusicCommand::Remove(id) => queue.retain(|item| item.id != id),
                MusicCommand::ParticipantLeft(participant_id) => {
                    remove_owner_items(&mut queue, &participant_id)
                }
                MusicCommand::Stop => stopped = true,
                MusicCommand::Supersede => superseded = true,
                MusicCommand::Seek { result, .. } => {
                    let _ = result.send(Err(()));
                }
                MusicCommand::Pause | MusicCommand::Resume | MusicCommand::Skip => {}
            }
        }
        if stopped || superseded {
            break;
        }
        while queue.front().is_some_and(|item| {
            item_owner(item).is_none_or(|owner| !participant_is_present(&room, owner))
        }) {
            queue.pop_front();
        }
        let Some(mut current_item) = queue.pop_front() else {
            break;
        };
        let Some(owner_id) = item_owner(&current_item).map(str::to_owned) else {
            continue;
        };
        if owner_id != host_id {
            revision = revision.saturating_add(1);
            let state = MusicSnapshot {
                session_id: session_id.clone(),
                revision,
                coordinator_id: Some(coordinator_id.clone()),
                active: true,
                host_id: Some(owner_id.clone()),
                current_item_id: Some(current_item.id.clone()),
                title: current_item.title.clone(),
                source_url: current_item.source_url.clone(),
                artwork_url: current_item.artwork_url.clone(),
                requested_by: current_item.requested_by.clone(),
                loading: true,
                queue: queue.iter().cloned().collect(),
                ..Default::default()
            };
            {
                let mut locked = runtime.lock().await;
                if locked.generation != generation {
                    return;
                }
                locked.commands = None;
                locked.set_snapshot(state.clone());
            }

            if runtime.lock().await.generation != generation {
                return;
            }
            if publish_packet(
                &room,
                &MusicPacket::Handoff {
                    state,
                    item: current_item,
                },
            )
            .await
            .is_ok()
            {
                return;
            }
            final_error = "Could not hand off shared music.".into();
            break;
        }
        let source = runtime
            .lock()
            .await
            .owned_sources
            .get(&current_item.id)
            .cloned();
        let inherited_url = source
            .is_none()
            .then(|| super::protocol::transferable_url(&current_item.playback_source))
            .flatten();
        let Some((requested, local_file)) =
            source.or_else(|| inherited_url.clone().map(|url| (url, false)))
        else {
            final_error = "The song owner no longer has its playback source; skipped it.".into();
            continue;
        };
        current_item.playback_source = requested.clone();
        current_item.local_file = local_file;
        let loading = MusicSnapshot {
            active: true,
            current_item_id: Some(current_item.id.clone()),
            title: current_item.title.clone(),
            source_url: current_item.source_url.clone(),
            transfer_url: inherited_url,
            artwork_url: current_item.artwork_url.clone(),
            requested_by: current_item.requested_by.clone(),
            loading: true,
            queue: queue.iter().cloned().collect(),
            ..Default::default()
        };
        broadcast_host_state(
            &room,
            &runtime,
            generation,
            (&session_id, &coordinator_id),
            &mut revision,
            loading,
        )
        .await;
        // Resolution starts as soon as the song is queued. Consume that
        // prepared source here so a queued next track doesn't wait for yt-dlp
        // only after the previous FFmpeg process has ended.
        let prepared = {
            let mut locked = runtime.lock().await;
            if locked.generation != generation {
                return;
            }
            locked.prepared_media.remove(&current_item.id)
        };
        let mut media = match match prepared {
            Some(media) => Ok(media),
            None => resolve_media(&requested, local_file).await,
        } {
            Ok(media) => media,
            Err(error) => {
                final_error = error.clone();
                broadcast_host_state(
                    &room,
                    &runtime,
                    generation,
                    (&session_id, &coordinator_id),
                    &mut revision,
                    MusicSnapshot {
                        active: !queue.is_empty(),
                        error,
                        queue: queue.iter().cloned().collect(),
                        ..Default::default()
                    },
                )
                .await;
                continue;
            }
        };
        // URL resolution can take several seconds. Apply anything that arrived
        // meanwhile before this client is allowed to publish audio.
        while let Ok(command) = commands.try_recv() {
            match command {
                MusicCommand::Add { item, result } => {
                    apply_add_command(&mut queue, Some(&current_item.id), item, result);
                }
                MusicCommand::AddBatch {
                    observed_revision,
                    items,
                    result,
                } => apply_batch_command(
                    &mut queue,
                    Some(&current_item.id),
                    revision,
                    observed_revision,
                    items,
                    result,
                ),
                MusicCommand::UpdateMetadata {
                    id,
                    requester,
                    title,
                    source_url,
                    artwork_url,
                } => update_item_metadata(
                    &mut queue,
                    Some(&current_item.id),
                    &id,
                    &requester,
                    title,
                    source_url,
                    artwork_url,
                ),
                MusicCommand::Remove(id) => queue.retain(|item| item.id != id),
                MusicCommand::ParticipantLeft(participant_id) => {
                    remove_owner_items(&mut queue, &participant_id)
                }
                MusicCommand::Stop => stopped = true,
                MusicCommand::Supersede => superseded = true,
                MusicCommand::Seek { result, .. } => {
                    let _ = result.send(Err(()));
                }
                MusicCommand::Pause | MusicCommand::Resume | MusicCommand::Skip => {}
            }
        }
        if stopped || superseded {
            break;
        }
        let mut elapsed_samples = 0_u64;
        let mut retries = 0_u8;
        let mut paused = false;
        let mut base = MusicSnapshot {
            active: true,
            current_item_id: Some(current_item.id.clone()),
            title: media.title.clone(),
            artist: media.artist.clone(),
            source_url: media.source_url.clone(),
            transfer_url: media.transfer_url.clone(),
            artwork_url: media.artwork_url.clone(),
            requested_by: current_item.requested_by.clone(),
            duration_seconds: media.duration_seconds,
            queue: queue.iter().cloned().collect(),
            ..Default::default()
        };
        broadcast_host_state(
            &room,
            &runtime,
            generation,
            (&session_id, &coordinator_id),
            &mut revision,
            base.clone(),
        )
        .await;
        let mut produced_audio = false;
        let mut processed_seeks = ProcessedSeeks::default();
        let mut pending_seek_result: Option<(
            MusicSeekRequest,
            tokio::sync::oneshot::Sender<Result<MusicSeekOutcome, ()>>,
        )> = None;
        'track: loop {
            if paused {
                match commands.recv().await {
                    Some(MusicCommand::Resume) => {
                        paused = false;
                        let mut state = base.clone();
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::Add { item, result }) => {
                        apply_add_command(&mut queue, Some(&current_item.id), item, result);
                        base.queue = queue.iter().cloned().collect();
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::AddBatch {
                        observed_revision,
                        items,
                        result,
                    }) => {
                        apply_batch_command(
                            &mut queue,
                            Some(&current_item.id),
                            revision,
                            observed_revision,
                            items,
                            result,
                        );
                        base.queue = queue.iter().cloned().collect();
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::UpdateMetadata {
                        id,
                        requester,
                        title,
                        source_url,
                        artwork_url,
                    }) => {
                        update_item_metadata(
                            &mut queue,
                            Some(&current_item.id),
                            &id,
                            &requester,
                            title,
                            source_url,
                            artwork_url,
                        );
                        base.queue = queue.iter().cloned().collect();
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::Remove(id)) => {
                        queue.retain(|item| item.id != id);
                        base.queue = queue.iter().cloned().collect();
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::ParticipantLeft(participant_id)) => {
                        remove_owner_items(&mut queue, &participant_id);
                        base.queue = queue.iter().cloned().collect();
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = elapsed_samples / 48_000;
                        broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                    }
                    Some(MusicCommand::Seek { request, result }) => {
                        if let Some(outcome) = processed_seeks.outcome(&request) {
                            let _ = result.send(Ok(outcome));
                            continue;
                        }
                        let Some(target_seconds) = seek_target(
                            &request,
                            &session_id,
                            &current_item.id,
                            revision,
                            base.playback_epoch,
                            base.duration_seconds,
                        ) else {
                            let _ = result.send(Err(()));
                            continue;
                        };
                        elapsed_samples = target_seconds.saturating_mul(48_000);
                        retries = 0;
                        base.playback_epoch = base.playback_epoch.saturating_add(1);
                        let _ = pcm_sender.send(MusicPcmCommand::Flush).await;
                        #[cfg(target_os = "linux")]
                        {
                            if let Some(mut output) = local_output.take() {
                                let _ = output.kill().await;
                            }
                            local_output = open_local_output(&output_device).await;
                        }
                        let mut state = base.clone();
                        state.paused = true;
                        state.elapsed_seconds = target_seconds;
                        let published = broadcast_host_state(
                            &room,
                            &runtime,
                            generation,
                            (&session_id, &coordinator_id),
                            &mut revision,
                            state,
                        )
                        .await;
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                        if published {
                            let outcome = MusicSeekOutcome {
                                revision,
                                playback_epoch: base.playback_epoch,
                                elapsed_seconds: target_seconds,
                            };
                            processed_seeks.record(&request, outcome);
                            let _ = result.send(Ok(outcome));
                        } else {
                            let _ = result.send(Err(()));
                        }
                    }
                    Some(MusicCommand::Skip) => break 'track,
                    Some(MusicCommand::Stop) | None => {
                        stopped = true;
                        break 'track;
                    }
                    Some(MusicCommand::Supersede) => {
                        superseded = true;
                        break 'track;
                    }
                    Some(MusicCommand::Pause) => {}
                }
                continue;
            }

            let mut child =
                decoder_command(&ffmpeg, &media.stream_url, local_file, elapsed_samples);
            let Ok(mut child) = child.spawn() else {
                if let Some((_, result)) = pending_seek_result.take() {
                    let _ = result.send(Err(()));
                }
                final_error = "Could not start FFmpeg. Install it or use a Stonks build that bundles media tools.".into();
                broadcast_host_state(
                    &room,
                    &runtime,
                    generation,
                    (&session_id, &coordinator_id),
                    &mut revision,
                    MusicSnapshot {
                        active: !queue.is_empty(),
                        host_id: Some(host_id.clone()),
                        error: final_error.clone(),
                        queue: queue.iter().cloned().collect(),
                        ..Default::default()
                    },
                )
                .await;
                break 'track;
            };
            let Some(mut stdout) = child.stdout.take() else {
                let _ = child.kill().await;
                if let Some((_, result)) = pending_seek_result.take() {
                    let _ = result.send(Err(()));
                }
                break 'track;
            };
            let mut last_state_second = elapsed_samples / 48_000;
            let mut retrying = false;
            let mut seek_restart = false;
            let mut bytes = [0_u8; MUSIC_FRAME_SAMPLES * 2];
            let mut filled = 0;
            loop {
                tokio::select! {
                    command = commands.recv() => {
                        // Queue changes only affect state; they must not restart
                        // playback or tear down the local output. In particular,
                        // pw-cat is intentionally kept alive across a pause or a
                        // refreshed FFmpeg process so PipeWire does not drop the
                        // music route.
                        let mut stop_decoder = false;
                        match command {
                            Some(MusicCommand::Add { item, result }) => {
                                apply_add_command(&mut queue, Some(&current_item.id), item, result);
                                base.queue = queue.iter().cloned().collect();
                                let mut state = base.clone(); state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                continue;
                            }
                            Some(MusicCommand::AddBatch { observed_revision, items, result }) => {
                                apply_batch_command(
                                    &mut queue,
                                    Some(&current_item.id),
                                    revision,
                                    observed_revision,
                                    items,
                                    result,
                                );
                                base.queue = queue.iter().cloned().collect();
                                let mut state = base.clone(); state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                continue;
                            }
                            Some(MusicCommand::UpdateMetadata { id, requester, title, source_url, artwork_url }) => {
                                update_item_metadata(
                                    &mut queue,
                                    Some(&current_item.id),
                                    &id,
                                    &requester,
                                    title,
                                    source_url,
                                    artwork_url,
                                );
                                base.queue = queue.iter().cloned().collect();
                                let mut state = base.clone(); state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                continue;
                            }
                            Some(MusicCommand::Remove(id)) => {
                                queue.retain(|item| item.id != id);
                                base.queue = queue.iter().cloned().collect();
                                let mut state = base.clone(); state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                continue;
                            }
                            Some(MusicCommand::ParticipantLeft(participant_id)) => {
                                remove_owner_items(&mut queue, &participant_id);
                                base.queue = queue.iter().cloned().collect();
                                let mut state = base.clone(); state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                continue;
                            }
                            Some(MusicCommand::Seek { request, result }) => {
                                if let Some(outcome) = processed_seeks.outcome(&request) {
                                    let _ = result.send(Ok(outcome));
                                    continue;
                                }
                                let Some(target_seconds) = seek_target(
                                    &request,
                                    &session_id,
                                    &current_item.id,
                                    revision,
                                    base.playback_epoch,
                                    base.duration_seconds,
                                ) else {
                                    let _ = result.send(Err(()));
                                    continue;
                                };
                                elapsed_samples = target_seconds.saturating_mul(48_000);
                                retries = 0;
                                base.playback_epoch = base.playback_epoch.saturating_add(1);
                                let _ = pcm_sender.send(MusicPcmCommand::Flush).await;
                                #[cfg(target_os = "linux")]
                                {
                                    if let Some(mut output) = local_output.take() {
                                        let _ = output.kill().await;
                                    }
                                    local_output = open_local_output(&output_device).await;
                                }
                                let mut state = base.clone();
                                state.seeking = true;
                                state.elapsed_seconds = target_seconds;
                                broadcast_host_state(
                                    &room,
                                    &runtime,
                                    generation,
                                    (&session_id, &coordinator_id),
                                    &mut revision,
                                    state,
                                )
                                .await;
                                if let Some((_, previous_result)) = pending_seek_result.replace((request, result)) {
                                    let _ = previous_result.send(Err(()));
                                }
                                seek_restart = true;
                                stop_decoder = true;
                            }
                            Some(MusicCommand::Pause) => {
                                paused = true;
                                let mut state = base.clone(); state.paused = true; state.elapsed_seconds = elapsed_samples / 48_000;
                                broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                                stop_decoder = true;
                            }
                            Some(MusicCommand::Skip) => { stop_decoder = true; }
                            Some(MusicCommand::Resume) => {}
                            Some(MusicCommand::Stop) | None => { stopped = true; stop_decoder = true; }
                            Some(MusicCommand::Supersede) => { superseded = true; stop_decoder = true; }
                        }
                        if stop_decoder {
                            let _ = child.kill().await;
                            break;
                        }
                    }
                    read = stdout.read(&mut bytes[filled..]) => {
                        if let Ok(n @ 1..) = read {
                            filled += n;
                            if filled < bytes.len() { continue; }
                            filled = 0;
                        } else {
                            if let Some((_, result)) = pending_seek_result.take() {
                                let _ = result.send(Err(()));
                            }
                            let failed = child.wait().await.map_or(true, |status| !status.success());
                            let elapsed_seconds = elapsed_samples / 48_000;
                            let ended_early = media.duration_seconds
                                .is_some_and(|duration| elapsed_seconds.saturating_add(3) < duration);
                            if (failed || ended_early) && retries < 3 {
                                retries += 1;
                                match resolve_media(&requested, local_file).await {
                                    Ok(refreshed) => {
                                        media.stream_url = refreshed.stream_url;
                                        retrying = true;
                                    }
                                    Err(error) => final_error = error,
                                }
                            }
                            break;
                        }
                        produced_audio = true;
                        final_error.clear();
                        if let Some((request, result)) = pending_seek_result.take() {
                            let mut state = base.clone();
                            state.elapsed_seconds = last_state_second;
                            let published = broadcast_host_state(
                                &room,
                                &runtime,
                                generation,
                                (&session_id, &coordinator_id),
                                &mut revision,
                                state,
                            )
                            .await;
                            if published {
                                let outcome = MusicSeekOutcome {
                                    revision,
                                    playback_epoch: base.playback_epoch,
                                    elapsed_seconds: last_state_second,
                                };
                                processed_seeks.record(&request, outcome);
                                let _ = result.send(Ok(outcome));
                            } else {
                                let _ = result.send(Err(()));
                            }
                        }
                        let pcm = bytes.as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes(*pair)).collect::<Vec<_>>();
                        if pcm_sender.send(MusicPcmCommand::Samples(pcm.clone())).await.is_err() { let _ = child.kill().await; break; }
                        elapsed_samples = elapsed_samples.saturating_add(pcm.len() as u64);
                        let gain = if voice.as_ref().is_some_and(|voice| { let state = voice.borrow(); state.deafened || state.phase != VoicePhase::Connected }) {
                            0.0
                        } else {
                            f32::from(local_gain.load(Ordering::Relaxed)) * f32::from(output_volume) / 10_000.0
                        };
                        #[cfg(target_os = "linux")]
                        if let Some(stdin) = local_output.as_mut().and_then(|output| output.stdin.as_mut()) {
                            let local = pcm.iter().flat_map(|sample| ((*sample as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes()).collect::<Vec<_>>();
                            let _ = tokio::time::timeout(std::time::Duration::from_millis(100), stdin.write_all(&local)).await;
                        }

                        #[cfg(target_os = "windows")]
                        if let Some((_, player)) = &local_output {
                            player.set_volume(gain);
                            if player.len() < 8 {
                                player.append(rodio::buffer::SamplesBuffer::new(
                                    std::num::NonZeroU16::MIN,
                                    std::num::NonZeroU32::new(48_000).unwrap(),
                                    pcm.into_iter().map(|sample| f32::from(sample) / 32768.0).collect::<Vec<_>>(),
                                ));
                            }
                        }
                        let current = elapsed_samples / 48_000;
                        if current != last_state_second {
                            last_state_second = current;
                            let mut state = base.clone(); state.elapsed_seconds = current; broadcast_host_state(&room, &runtime, generation, (&session_id, &coordinator_id), &mut revision, state).await;
                        }
                    }
                }
                if paused || stopped || superseded {
                    break;
                }
            }
            #[cfg(target_os = "windows")]
            if paused || seek_restart || retrying || stopped || superseded {
                if let Some((sink, player)) = &mut local_output {
                    *player = rodio::Player::connect_new(sink.mixer());
                }
            }
            if paused {
                if let Some((_, result)) = pending_seek_result.take() {
                    let _ = result.send(Err(()));
                }
                continue;
            }
            if seek_restart {
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                continue;
            }
            if retrying {
                continue;
            }
            break 'track;
        }
        if let Some((_, result)) = pending_seek_result.take() {
            let _ = result.send(Err(()));
        }
        {
            let mut locked = runtime.lock().await;
            if locked.generation != generation {
                return;
            }
            locked.owned_items.remove(&current_item.id);
            locked.owned_sources.remove(&current_item.id);
        }
        if stopped || superseded {
            break;
        }
        if !produced_audio && final_error.is_empty() {
            final_error = "That track could not be played; skipped to the next song.".into();
        }
    }
    if !superseded {
        let mut finished = MusicSnapshot {
            active: false,
            error: String::new(),
            ..Default::default()
        };
        if !stopped {
            finished.error = final_error;
        }
        broadcast_host_state(
            &room,
            &runtime,
            generation,
            (&session_id, &coordinator_id),
            &mut revision,
            finished,
        )
        .await;
    }
    let mut locked = runtime.lock().await;
    if locked.generation == generation {
        locked.commands = None;
        locked.host_task = None;
        if !superseded {
            locked.owned_items.clear();
            locked.owned_sources.clear();
            for (_, task) in locked.resolver_tasks.drain() {
                task.abort();
            }
            locked.resolver_generations.clear();
            locked.prepared_media.clear();
        }
    }
}

pub(super) struct HostAssignment {
    pub(super) session_id: String,
    pub(super) coordinator_id: String,
    pub(super) revision: u64,
    pub(super) item: MusicQueueItem,
    pub(super) queue: Vec<MusicQueueItem>,
    pub(super) validate_snapshot: bool,
}

pub(super) async fn start_local_host(
    room: Arc<Room>,
    runtime: Arc<Mutex<MusicRuntime>>,
    assignment: HostAssignment,
) -> Result<(), String> {
    let HostAssignment {
        session_id,
        coordinator_id,
        revision,
        mut item,
        queue,
        validate_snapshot,
    } = assignment;
    if !queue_is_bounded(&queue, Some(&item.id)) {
        return Err(QUEUE_FULL_ERROR.into());
    }
    if validate_snapshot {
        super::ensure_publisher(&room, &runtime).await?;
    }
    let mut locked = runtime.lock().await;
    let local_id = room.local_participant().identity().to_string();
    if locked.commands.is_some()
        || (validate_snapshot
            && (!locked.snapshot.active
                || locked.snapshot.session_id != session_id
                || locked.snapshot.revision != revision
                || locked.snapshot.host_id.as_deref() != Some(&local_id)
                || locked.snapshot.current_item_id.as_deref() != Some(&item.id)))
    {
        return Ok(());
    }
    if let Some((source, local_file)) = locked.owned_sources.get(&item.id).cloned() {
        item.playback_source = source;
        item.local_file = local_file;
    }
    let pcm_sender = locked
        .pcm_sender
        .clone()
        .ok_or("The music publisher is not ready on the song owner's device.")?;
    let output_device = locked
        .output_device
        .clone()
        .unwrap_or_else(|| "default".into());

    let (sender, receiver) = mpsc::unbounded_channel();
    locked.commands = Some(sender);
    locked.generation = locked.generation.saturating_add(1);
    let generation = locked.generation;
    let ffmpeg = tool_path("ffmpeg");
    let task_runtime = runtime.clone();
    let task = tokio::spawn(run_host(HostWorker {
        room,
        runtime: task_runtime,
        pcm_sender,
        commands: receiver,
        generation,
        session_id,
        coordinator_id,
        revision,
        first_item: item,
        remaining_queue: queue,
        ffmpeg,
        output_device,
    }));
    if let Some(previous) = locked.host_task.replace(task) {
        previous.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    #[ignore = "requires installed FFmpeg; decodes a generated local fixture only"]
    async fn local_wav_decodes_to_native_music_pcm() {
        let mut wav = tempfile::NamedTempFile::new().unwrap();
        let sample_count = 4_800_u32;
        let byte_count = sample_count * 2;
        wav.write_all(b"RIFF").unwrap();
        wav.write_all(&(36 + byte_count).to_le_bytes()).unwrap();
        wav.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0").unwrap();
        wav.write_all(&48_000_u32.to_le_bytes()).unwrap();
        wav.write_all(&96_000_u32.to_le_bytes()).unwrap();
        wav.write_all(b"\x02\0\x10\0data").unwrap();
        wav.write_all(&byte_count.to_le_bytes()).unwrap();
        for _ in 0..sample_count {
            wav.write_all(&1234_i16.to_le_bytes()).unwrap();
        }
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            decoder_command(&tool_path("ffmpeg"), wav.path().to_str().unwrap(), true, 0).output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), byte_count as usize);
        assert!(output
            .stdout
            .chunks_exact(2)
            .all(|sample| sample == 1234_i16.to_le_bytes()));
    }
}
