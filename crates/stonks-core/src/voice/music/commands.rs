use super::{
    ensure_publisher,
    host::{start_local_host, HostAssignment, MusicCommand, MusicSeekRequest},
    preparation::spawn_media_preparation,
    protocol::{queue_is_bounded, MusicPacket, MusicQueueItem},
    queue::{insert_item, queue_item, BatchInsertError},
    resolver::{is_collection_url, resolve_collection},
    runtime::{discard_requested_item, import_observation_is_current, MusicRuntime},
    transport::publish_packet,
    *,
};
use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) const QUEUE_FULL_ERROR: &str = "The shared music queue is full (100 tracks).";
const PLAYLIST_STALE_ERROR: &str = "The shared music session changed while importing. Try again.";
const PLAYLIST_DUPLICATE_ERROR: &str = "This playlist already has tracks in the shared queue.";

pub(super) async fn append(
    room: Arc<Room>,
    music_runtime: Arc<Mutex<MusicRuntime>>,
    url: String,
) -> Result<(), String> {
    let requested = url.trim();
    if requested.is_empty() || requested.len() > 4096 {
        return Err("Enter a valid music URL or absolute local audio path.".into());
    }
    let local_file = PathBuf::from(requested).is_absolute();
    if !local_file
        && !url::Url::parse(requested)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    {
        return Err("Enter a valid music URL or absolute local audio path.".into());
    }
    if !local_file && is_collection_url(requested) {
        return append_collection(room, music_runtime, requested.to_owned()).await;
    }
    if local_file && !PathBuf::from(requested).is_file() {
        return Err("The local audio file does not exist or cannot be read.".into());
    }
    if local_file
        && !PathBuf::from(requested)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac"
                )
            })
    {
        return Err("Choose an MP3, WAV, FLAC, OGG, M4A, or AAC audio file.".into());
    }
    let local_id = room.local_participant().identity().to_string();
    ensure_publisher(&room, &music_runtime).await?;
    let item = queue_item(requested.into(), local_id.clone(), local_file);
    let snapshot;
    let local_commands;
    {
        let mut runtime = music_runtime.lock().await;
        snapshot = runtime.snapshot.clone();
        if snapshot.active && snapshot.host_id.as_deref() != Some(&local_id) {
            let mut visible_queue = VecDeque::from(snapshot.queue.clone());
            insert_item(
                &mut visible_queue,
                snapshot.current_item_id.as_deref(),
                item.clone(),
            )
            .map_err(|_| QUEUE_FULL_ERROR)?;
        }
        runtime
            .owned_sources
            .insert(item.id.clone(), (requested.into(), local_file));
        runtime.owned_items.insert(item.id.clone(), item.clone());
        local_commands = runtime.commands.clone();
    }
    if snapshot.active {
        if snapshot.host_id.as_deref() == Some(&local_id) {
            let (result, accepted) = tokio::sync::oneshot::channel();
            let Some(local_commands) = local_commands else {
                discard_requested_item(&music_runtime, &item.id).await;
                return Err("The shared music host is not ready.".into());
            };
            if local_commands
                .send(MusicCommand::Add {
                    item: item.clone(),
                    result: Some(result),
                })
                .is_err()
            {
                discard_requested_item(&music_runtime, &item.id).await;
                return Err("The shared music host stopped.".into());
            }
            match accepted.await {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => {
                    discard_requested_item(&music_runtime, &item.id).await;
                    return Err(QUEUE_FULL_ERROR.into());
                }
                Err(_) => {
                    discard_requested_item(&music_runtime, &item.id).await;
                    return Err("The shared music host stopped.".into());
                }
            }
        } else {
            let (result, accepted) = tokio::sync::oneshot::channel();
            music_runtime
                .lock()
                .await
                .pending_additions
                .insert(item.id.clone(), result);
            if let Err(error) = publish_packet(
                &room,
                &MusicPacket::Add {
                    session_id: snapshot.session_id.clone(),
                    item: item.clone(),
                },
            )
            .await
            {
                discard_requested_item(&music_runtime, &item.id).await;
                return Err(error);
            }
            match tokio::time::timeout(Duration::from_secs(60), accepted).await {
                Ok(Ok(Ok(_))) => {}
                Ok(Ok(Err(_))) => {
                    discard_requested_item(&music_runtime, &item.id).await;
                    return Err(QUEUE_FULL_ERROR.into());
                }
                Ok(Err(_)) | Err(_) => {
                    discard_requested_item(&music_runtime, &item.id).await;
                    return Err("The shared music host stopped.".into());
                }
            }
        }
    } else {
        let session_id = format!(
            "{local_id}:{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        start_local_host(
            room.clone(),
            music_runtime.clone(),
            HostAssignment {
                session_id,
                coordinator_id: local_id.clone(),
                revision: 0,
                item: item.clone(),
                queue: Vec::new(),
                validate_snapshot: false,
            },
        )
        .await?;
    }
    if !local_file && snapshot.active {
        spawn_media_preparation(room, music_runtime, item.id, requested.to_owned(), local_id).await;
    }
    Ok(())
}

async fn append_collection(
    room: Arc<Room>,
    music_runtime: Arc<Mutex<MusicRuntime>>,
    requested: String,
) -> Result<(), String> {
    let local_id = room.local_participant().identity().to_string();
    let (observed_session, observed_revision, host_generation, import_generation) = {
        let mut locked = music_runtime.lock().await;
        locked.import_generation = locked.import_generation.saturating_add(1);
        (
            locked.snapshot.session_id.clone(),
            locked.snapshot.revision,
            locked.generation,
            locked.import_generation,
        )
    };
    let resolved = resolve_collection(&requested).await?;
    let mut items = Vec::with_capacity(resolved.len());
    let mut sources = Vec::with_capacity(resolved.len());
    for (index, entry) in resolved.into_iter().enumerate() {
        let mut hasher = DefaultHasher::new();
        local_id.hash(&mut hasher);
        requested.hash(&mut hasher);
        index.hash(&mut hasher);
        entry.playback_source.hash(&mut hasher);
        let id = format!("{local_id}:import:{:016x}", hasher.finish());
        sources.push((id.clone(), entry.playback_source));
        items.push(MusicQueueItem {
            id,
            title: entry.title,
            source_url: entry.source_url,
            artwork_url: entry.artwork_url,
            requested_by: Some(local_id.clone()),
            playback_source: String::new(),
            local_file: false,
        });
    }
    if !queue_is_bounded(&items, None) {
        return Err(QUEUE_FULL_ERROR.into());
    }

    {
        let locked = music_runtime.lock().await;
        if !import_observation_is_current(
            &locked,
            import_generation,
            host_generation,
            &observed_session,
            observed_revision,
        ) {
            return Err(PLAYLIST_STALE_ERROR.into());
        }
        let duplicate_count = items
            .iter()
            .filter(|item| locked.owned_sources.contains_key(&item.id))
            .count();
        if duplicate_count == items.len() {
            return Ok(());
        }
        if duplicate_count != 0 {
            return Err(PLAYLIST_DUPLICATE_ERROR.into());
        }
    }

    ensure_publisher(&room, &music_runtime).await?;

    let snapshot = {
        let mut locked = music_runtime.lock().await;
        if !import_observation_is_current(
            &locked,
            import_generation,
            host_generation,
            &observed_session,
            observed_revision,
        ) {
            return Err(PLAYLIST_STALE_ERROR.into());
        }
        for (item, (_, source)) in items.iter().zip(&sources) {
            locked
                .owned_sources
                .insert(item.id.clone(), (source.clone(), false));
            locked.owned_items.insert(item.id.clone(), item.clone());
        }
        locked.snapshot.clone()
    };

    let result = if snapshot.active {
        if snapshot.host_id.as_deref() == Some(&local_id) {
            let commands = music_runtime.lock().await.commands.clone();
            if let Some(commands) = commands {
                let (result, accepted) = tokio::sync::oneshot::channel();
                if commands
                    .send(MusicCommand::AddBatch {
                        observed_revision,
                        items: items.clone(),
                        result,
                    })
                    .is_err()
                {
                    Err(BatchInsertError::Stale)
                } else {
                    accepted.await.unwrap_or(Err(BatchInsertError::Stale))
                }
            } else {
                Err(BatchInsertError::Stale)
            }
        } else {
            let request_id = format!("{local_id}:import:{import_generation}");
            let (result, accepted) = tokio::sync::oneshot::channel();
            music_runtime
                .lock()
                .await
                .pending_imports
                .insert(request_id.clone(), result);
            if publish_packet(
                &room,
                &MusicPacket::AddBatch {
                    session_id: observed_session.clone(),
                    request_id: request_id.clone(),
                    observed_revision,
                    items: items.clone(),
                },
            )
            .await
            .is_err()
            {
                music_runtime
                    .lock()
                    .await
                    .pending_imports
                    .remove(&request_id);
                Err(BatchInsertError::Stale)
            } else {
                let outcome = tokio::time::timeout(Duration::from_secs(90), accepted)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(Err(BatchInsertError::Stale));
                music_runtime
                    .lock()
                    .await
                    .pending_imports
                    .remove(&request_id);
                outcome
            }
        }
    } else {
        let mut remaining = items.clone();
        let first = remaining.remove(0);
        let session_id = format!(
            "{local_id}:{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        if start_local_host(
            room,
            music_runtime.clone(),
            HostAssignment {
                session_id,
                coordinator_id: local_id,
                revision: 0,
                item: first,
                queue: remaining,
                validate_snapshot: false,
            },
        )
        .await
        .is_err()
        {
            Err(BatchInsertError::Full)
        } else {
            let locked = music_runtime.lock().await;
            if locked.generation == host_generation || locked.commands.is_none() {
                Err(BatchInsertError::Stale)
            } else {
                Ok(super::queue::BatchInsert::Added)
            }
        }
    };

    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            let mut locked = music_runtime.lock().await;
            for item in &items {
                locked.owned_items.remove(&item.id);
                locked.owned_sources.remove(&item.id);
                locked.prepared_media.remove(&item.id);
            }
            Err(match error {
                BatchInsertError::Full => QUEUE_FULL_ERROR.into(),
                BatchInsertError::Stale => PLAYLIST_STALE_ERROR.into(),
            })
        }
    }
}

pub(super) async fn control(
    room: Arc<Room>,
    music_runtime: Arc<Mutex<MusicRuntime>>,
    action: String,
    item_id: Option<String>,
    target_seconds: Option<u64>,
    observed_revision: Option<u64>,
    observed_playback_epoch: Option<u64>,
    request_id: Option<String>,
) -> Result<(), String> {
    if !matches!(
        action.as_str(),
        "pause" | "resume" | "stop" | "skip" | "remove" | "seek"
    ) {
        return Err("Unknown music control.".into());
    }
    if action == "remove" && item_id.as_deref().is_none_or(str::is_empty) {
        return Err("Choose a queued song to remove.".into());
    }
    let seek_request = if action == "seek" {
        let item_id = item_id
            .clone()
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .ok_or("The music seek request is invalid.")?;
        let request_id = request_id
            .filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            })
            .ok_or("The music seek request is invalid.")?;
        Some(MusicSeekRequest {
            session_id: String::new(),
            item_id,
            requester_id: String::new(),
            observed_revision: observed_revision.ok_or("The music seek request is invalid.")?,
            observed_playback_epoch: observed_playback_epoch
                .ok_or("The music seek request is invalid.")?,
            request_id,
            target_seconds: target_seconds.ok_or("The music seek request is invalid.")?,
        })
    } else {
        if target_seconds.is_some()
            || observed_revision.is_some()
            || observed_playback_epoch.is_some()
            || request_id.is_some()
        {
            return Err("The music seek request is invalid.".into());
        }
        None
    };
    let local_id = room.local_participant().identity().to_string();
    let runtime = music_runtime.lock().await;
    if !runtime.snapshot.active {
        return Ok(());
    }
    if let Some(mut request) = seek_request {
        request.session_id = runtime.snapshot.session_id.clone();
        request.requester_id = local_id.clone();
        if runtime.snapshot.current_item_id.as_deref() != Some(&request.item_id)
            || runtime
                .snapshot
                .duration_seconds
                .is_none_or(|duration| duration <= 1 || request.target_seconds >= duration)
        {
            return Err("This track cannot be seeked.".into());
        }
        let session_id = request.session_id.clone();
        let item_id = request.item_id.clone();
        let request_id = request.request_id.clone();
        if runtime.snapshot.host_id.as_deref() == Some(&local_id) {
            let sender = runtime
                .commands
                .clone()
                .ok_or("The music player stopped.")?;
            drop(runtime);
            let (result, accepted) = tokio::sync::oneshot::channel();
            sender
                .send(MusicCommand::Seek { request, result })
                .map_err(|_| "The music player stopped.".to_owned())?;
            return match tokio::time::timeout(Duration::from_secs(10), accepted).await {
                Ok(Ok(Ok(_))) => Ok(()),
                _ => Err("This track cannot be seeked.".into()),
            };
        }
        let (result, accepted) = tokio::sync::oneshot::channel();
        drop(runtime);
        music_runtime
            .lock()
            .await
            .pending_seeks
            .insert(request_id.clone(), result);
        if let Err(error) = publish_packet(
            &room,
            &MusicPacket::Seek {
                session_id,
                item_id,
                observed_revision: request.observed_revision,
                observed_playback_epoch: request.observed_playback_epoch,
                request_id: request_id.clone(),
                target_seconds: request.target_seconds,
            },
        )
        .await
        {
            music_runtime.lock().await.pending_seeks.remove(&request_id);
            return Err(error);
        }
        return match tokio::time::timeout(Duration::from_secs(10), accepted).await {
            Ok(Ok(true)) => Ok(()),
            _ => {
                music_runtime.lock().await.pending_seeks.remove(&request_id);
                Err("This track cannot be seeked.".into())
            }
        };
    }
    if runtime.snapshot.host_id.as_deref() == Some(&local_id) {
        let sender = runtime
            .commands
            .clone()
            .ok_or("The music player stopped.")?;
        drop(runtime);
        let command = match action.as_str() {
            "pause" => MusicCommand::Pause,
            "resume" => MusicCommand::Resume,
            "skip" => MusicCommand::Skip,
            "remove" => MusicCommand::Remove(item_id.expect("validated queue item id")),
            _ => MusicCommand::Stop,
        };
        sender
            .send(command)
            .map_err(|_| "The music player stopped.".into())
    } else {
        let session_id = runtime.snapshot.session_id.clone();
        drop(runtime);
        publish_packet(
            &room,
            &MusicPacket::Control {
                session_id,
                action,
                item_id,
            },
        )
        .await
    }
}
