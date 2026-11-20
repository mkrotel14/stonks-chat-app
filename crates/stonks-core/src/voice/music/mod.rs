//! Native port of the Tauri `stonks:music:v2` room engine.
mod commands;
mod host;
mod orchestration;
mod preparation;
mod protocol;
mod publisher;
mod queue;
mod resolver;
mod runtime;
mod transport;

use super::{room::grant_allows_microphone, VoiceListeningGains, VoicePhase, VoiceSnapshot};
use livekit::{
    options::TrackPublishOptions,
    prelude::*,
    webrtc::{
        audio_frame::AudioFrame,
        audio_source::native::NativeAudioSource,
        prelude::{AudioSourceOptions, RtcAudioSource},
    },
};
pub use protocol::{MusicQueueItem, MusicSnapshot};
use runtime::MusicRuntime;
use std::sync::Arc;
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

pub(super) enum Operation {
    Append(String),
    Control(
        String,
        Option<String>,
        Option<u64>,
        Option<u64>,
        Option<u64>,
        Option<String>,
    ),
}
pub(super) type Command = (Operation, oneshot::Sender<Result<(), String>>);

async fn ensure_publisher(room: &Room, runtime: &Mutex<MusicRuntime>) -> Result<(), String> {
    let gate = runtime.lock().await.publisher_gate.clone();
    let _publisher = gate.lock().await;
    let participant = room.local_participant();
    if !grant_allows_microphone(participant.permission().as_ref()) {
        return Err("You do not have permission to play music in this call.".into());
    }
    if runtime.lock().await.publisher.is_some() {
        return Ok(());
    }
    let (track, source, pcm, frames) = publisher::native_music_track();
    let publication = participant
        .publish_track(
            LocalTrack::Audio(track),
            TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not publish the music track.")?;
    let mut locked = runtime.lock().await;
    locked.publisher = Some((
        publication.sid(),
        tokio::spawn(publisher::capture(source, frames)),
    ));
    locked.pcm_sender = Some(pcm);
    Ok(())
}

pub(super) async fn run(
    room: Arc<Room>,
    mut events: mpsc::UnboundedReceiver<RoomEvent>,
    mut voice: watch::Receiver<VoiceSnapshot>,
    mut inbox: mpsc::Receiver<Command>,
    updates: watch::Sender<MusicSnapshot>,
    output_device: String,
    output_volume: u8,
    gains: VoiceListeningGains,
    cancel: CancellationToken,
) {
    let runtime = Arc::new(Mutex::new(MusicRuntime {
        updates,
        output_device: Some(output_device),
        voice: Some(voice.clone()),
        local_gain: gains.music_gain_handle(),
        output_volume,
        ..Default::default()
    }));
    let mut jobs = JoinSet::new();
    let mut connected = true;
    // A reliable packet is not a reply guarantee: a request can land during
    // the gap between hosts, or before a rejoining peer's data channel is ready.
    let mut sync = tokio::time::interval(std::time::Duration::from_secs(1));
    sync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sync_attempts = 3;
    let mut runtime_reconciling = false;
    let event_loop = async {
        loop {
            tokio::select! {
                biased;
                event = events.recv() => match event {
                    None | Some(RoomEvent::Disconnected { .. }) => break false,
                    Some(RoomEvent::Reconnecting) => {
                        let mut locked = runtime.lock().await;
                        if locked.commands.is_some() && locked.snapshot.active {
                            let snapshot = locked.snapshot.clone();
                            locked.reconnect_host.get_or_insert(snapshot);
                        }
                        // Keep owner-only sources and the active host, as in Tauri.
                        // The same LiveKit Room restores its published tracks.
                        connected = false;
                    }
                    Some(RoomEvent::Reconnected) => {
                        connected = true;
                        runtime_reconciling = true;
                        sync_attempts = 3;
                        sync.reset_immediately();
                    }
                    Some(RoomEvent::ParticipantPermissionChanged { participant: Participant::Local(_), permission }) if !grant_allows_microphone(permission.as_ref()) => {
                        jobs.shutdown().await;
                        let local = room.local_participant().identity().to_string();
                        let mut locked = runtime.lock().await;
                        locked.owned_items.clear();
                        locked.owned_sources.clear();
                        locked.prepared_media.clear();
                        for (_, task) in locked.resolver_tasks.drain() { task.abort(); }
                        locked.resolver_generations.clear();
                        if locked.snapshot.host_id.as_deref() == Some(&local) {
                            // The host drains via the normal stop path and broadcasts inactive state.
                            if let Some(commands) = &locked.commands { let _ = commands.send(host::MusicCommand::Stop); }
                        }
                        drop(locked);
                        stop_publisher(&room, &runtime).await;
                    }
                    Some(RoomEvent::ParticipantDisconnected(participant)) => orchestration::participant_disconnected(&room, &runtime, &participant.identity().to_string()).await,
                    Some(RoomEvent::DataReceived { topic, participant, payload, .. }) if topic.as_deref() == Some(transport::MUSIC_TOPIC) => {
                        orchestration::receive_packet(&room, &runtime, participant.map(|p| p.identity().to_string()), &payload).await;
                    }
                    _ => {}
                },
                _ = sync.tick(), if connected && (sync_attempts > 0 || runtime_reconciling) => {
                    if sync_attempts > 0 {
                        sync_attempts -= 1;
                        orchestration::request_sync(&room).await;
                    } else {
                        let resume_advertising = runtime.lock().await.reconnect_host.take().is_some();
                        runtime_reconciling = false;
                        if resume_advertising {
                            orchestration::advertise_local_state(&room, &runtime).await;
                        }
                    }
                }
                _ = jobs.join_next(), if !jobs.is_empty() => {},
                command = inbox.recv(), if jobs.is_empty() => {
                    let Some((operation, reply)) = command else { break true; };
                    if reply.is_closed() { continue; }
                    let state = voice.borrow_and_update().clone();
                    if !connected || state.phase != VoicePhase::Connected {
                        let _ = reply.send(Err("The voice room is disconnected.".into()));
                        continue;
                    }
                    let room = room.clone();
                    let runtime = runtime.clone();
                    jobs.spawn(async move {
                        let result = match operation {
                            Operation::Append(_) if !state.can_speak => Err("You do not have permission to play music in this call.".into()),
                            Operation::Append(source) => commands::append(room, runtime, source).await,
                            Operation::Control(action, item, target, revision, epoch, request) => commands::control(room, runtime, action, item, target, revision, epoch, request).await,
                        };
                        let _ = reply.send(result);
                    });
                }
            }
        }
    };
    let graceful = tokio::select! {
        biased;
        _ = cancel.cancelled() => true,
        graceful = event_loop => graceful,
    };
    jobs.shutdown().await;
    if graceful && room.connection_state() == ConnectionState::Connected {
        orchestration::leave(&room, &runtime).await;
    }
    reset(&room, &runtime).await;
}

async fn stop_publisher(room: &Room, runtime: &Mutex<MusicRuntime>) {
    let gate = runtime.lock().await.publisher_gate.clone();
    let _publisher = gate.lock().await;
    let publisher = {
        let mut locked = runtime.lock().await;
        locked.pcm_sender = None;
        locked.publisher.take()
    };
    if let Some((sid, task)) = publisher {
        task.abort();
        let _ = task.await;
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            room.local_participant().unpublish_track(&sid),
        )
        .await;
    }
}

async fn reset(room: &Room, runtime: &Mutex<MusicRuntime>) {
    let tasks = {
        let mut locked = runtime.lock().await;
        let mut tasks: Vec<_> = locked
            .resolver_tasks
            .drain()
            .map(|(_, task)| task)
            .collect();
        tasks.extend(locked.host_task.take());
        locked.stop_tasks();
        for task in &tasks {
            task.abort();
        }
        locked.owned_items.clear();
        locked.owned_sources.clear();
        locked.prepared_media.clear();
        locked.set_snapshot(MusicSnapshot::default());
        tasks
    };
    for task in tasks {
        let _ = task.await;
    }
    stop_publisher(room, runtime).await;
}
