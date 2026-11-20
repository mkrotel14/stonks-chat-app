use super::{
    host::{
        participant_is_present, start_local_host, HostAssignment, MusicCommand, MusicSeekRequest,
    },
    protocol::{
        session_priority, MusicPacket, MusicQueueItem, MusicSnapshot, MAX_MUSIC_PACKET_BYTES,
    },
    queue::{item_owner, BatchInsert, BatchInsertError, QueueFull, QueueInsert},
    runtime::MusicRuntime,
    transport::publish_packet,
    *,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(in crate::voice) async fn receive_packet(
    room: &Arc<Room>,
    runtime: &Arc<Mutex<MusicRuntime>>,
    participant_id: Option<String>,
    payload: &[u8],
) {
    if payload.len() > MAX_MUSIC_PACKET_BYTES {
        return;
    }
    let Ok(packet) = serde_json::from_slice::<MusicPacket>(payload) else {
        return;
    };
    if !packet.is_valid_inbound() {
        return;
    };
    let local_id = room.local_participant().identity().to_string();
    match packet {
        MusicPacket::State { state } => {
            let Some(packet_sender) = participant_id.as_deref() else {
                return;
            };
            if state.host_id.as_deref() != Some(packet_sender)
                || packet_sender == local_id
                || !participant_is_present(room, packet_sender)
            {
                return;
            }
            let mut locked = runtime.lock().await;
            let current = locked.snapshot.clone();
            let same_session =
                !state.session_id.is_empty() && state.session_id == current.session_id;
            let reconnect_successor = locked
                .reconnect_host
                .as_ref()
                .is_some_and(|before| reconnect_successor(before, &state, packet_sender));
            let current_host_present = current
                .host_id
                .as_deref()
                .is_some_and(|host| participant_is_present(room, host));
            let incoming_wins_conflict = state.active
                && current.active
                && !same_session
                && (session_priority(&state), &state.session_id)
                    < (session_priority(&current), &current.session_id);
            if !accepts_state(
                &current,
                &state,
                packet_sender,
                current_host_present && !locked.provisional_host,
                incoming_wins_conflict,
            ) && !reconnect_successor
            {
                return;
            }
            let losing_items = if incoming_wins_conflict {
                locked.owned_items.values().cloned().collect::<Vec<_>>()
            } else {
                Default::default()
            };
            if incoming_wins_conflict || reconnect_successor {
                if let Some(commands) = locked.commands.clone() {
                    let _ = commands.send(MusicCommand::Supersede);
                }
                locked.generation = locked.generation.saturating_add(1);
                if let Some(task) = locked.host_task.take() {
                    task.abort();
                }
                locked.commands = None;
                locked.pending_additions.clear();
                locked.pending_imports.clear();
                locked.pending_seeks.clear();
            }
            locked.set_snapshot(state.clone());
            if !state.active {
                locked.owned_items.clear();
                locked.owned_sources.clear();
                locked.prepared_media.clear();
                for (_, task) in locked.resolver_tasks.drain() {
                    task.abort();
                }
                locked.resolver_generations.clear();
                locked.pending_additions.clear();
                locked.pending_imports.clear();
                locked.pending_seeks.clear();
            }
            let old_pcm = reconnect_successor
                .then(|| locked.pcm_sender.clone())
                .flatten();
            drop(locked);
            if let Some(pcm) = old_pcm {
                let _ = pcm.try_send(publisher::MusicPcmCommand::Flush);
            }

            for item in losing_items {
                let already_present = state.current_item_id.as_deref() == Some(&item.id)
                    || state.queue.iter().any(|queued| queued.id == item.id);
                if !already_present {
                    let _ = publish_packet(
                        room,
                        &MusicPacket::Add {
                            session_id: state.session_id.clone(),
                            item,
                        },
                    )
                    .await;
                }
            }
        }
        MusicPacket::Add { session_id, item } => {
            if item_owner(&item) != participant_id.as_deref() {
                return;
            }
            let commands = {
                let locked = runtime.lock().await;
                if locked.snapshot.session_id != session_id
                    || locked.snapshot.host_id.as_deref() != Some(&local_id)
                {
                    return;
                }
                locked.commands.clone()
            };
            let (result, accepted) = tokio::sync::oneshot::channel();
            let insertion = if commands.is_some_and(|commands| {
                commands
                    .send(MusicCommand::Add {
                        item: item.clone(),
                        result: Some(result),
                    })
                    .is_ok()
            }) {
                accepted.await.ok()
            } else {
                None
            };
            if let Some(insertion) = insertion {
                let _ = publish_packet(
                    room,
                    &MusicPacket::AddResult {
                        session_id,
                        item_id: item.id,
                        accepted: insertion.is_ok(),
                    },
                )
                .await;
            }
        }
        MusicPacket::AddResult {
            session_id,
            item_id,
            accepted,
        } => {
            let mut locked = runtime.lock().await;
            if locked.snapshot.session_id != session_id
                || locked.snapshot.host_id.as_deref() != participant_id.as_deref()
            {
                return;
            }
            if let Some(result) = locked.pending_additions.remove(&item_id) {
                let _ = result.send(if accepted {
                    Ok(QueueInsert::Added)
                } else {
                    Err(QueueFull)
                });
            }
        }
        MusicPacket::AddBatch {
            session_id,
            request_id,
            observed_revision,
            items,
        } => {
            if items
                .iter()
                .any(|item| item_owner(item) != participant_id.as_deref())
            {
                return;
            }
            let commands = {
                let locked = runtime.lock().await;
                if locked.snapshot.session_id != session_id
                    || locked.snapshot.host_id.as_deref() != Some(&local_id)
                {
                    return;
                }
                locked.commands.clone()
            };
            let (result, accepted) = tokio::sync::oneshot::channel();
            let insertion = if commands.is_some_and(|commands| {
                commands
                    .send(MusicCommand::AddBatch {
                        observed_revision,
                        items,
                        result,
                    })
                    .is_ok()
            }) {
                accepted.await.ok()
            } else {
                None
            };
            if let Some(insertion) = insertion {
                let _ = publish_packet(
                    room,
                    &MusicPacket::AddBatchResult {
                        session_id,
                        request_id,
                        accepted: insertion.is_ok(),
                        stale: insertion == Err(BatchInsertError::Stale),
                    },
                )
                .await;
            }
        }
        MusicPacket::AddBatchResult {
            session_id,
            request_id,
            accepted,
            stale,
        } => {
            let mut locked = runtime.lock().await;
            if locked.snapshot.session_id != session_id
                || locked.snapshot.host_id.as_deref() != participant_id.as_deref()
            {
                return;
            }
            if let Some(result) = locked.pending_imports.remove(&request_id) {
                let outcome = if accepted {
                    Ok(BatchInsert::Added)
                } else if stale {
                    Err(BatchInsertError::Stale)
                } else {
                    Err(BatchInsertError::Full)
                };
                let _ = result.send(outcome);
            }
        }
        MusicPacket::Metadata {
            session_id,
            item_id,
            requester,
            title,
            source_url,
            artwork_url,
        } => {
            if participant_id.as_deref() != Some(&requester) {
                return;
            }
            let locked = runtime.lock().await;
            if locked.snapshot.session_id == session_id
                && locked.snapshot.host_id.as_deref() == Some(&local_id)
            {
                if let Some(commands) = locked.commands.clone() {
                    let _ = commands.send(MusicCommand::UpdateMetadata {
                        id: item_id,
                        requester,
                        title,
                        source_url,
                        artwork_url,
                    });
                }
            }
        }
        MusicPacket::Control {
            session_id,
            action,
            item_id,
        } => {
            let locked = runtime.lock().await;
            if locked.snapshot.session_id == session_id
                && locked.snapshot.host_id.as_deref() == Some(&local_id)
            {
                let command = match action.as_str() {
                    "remove" => item_id.map(MusicCommand::Remove),
                    "skip" => Some(MusicCommand::Skip),
                    "pause" => Some(MusicCommand::Pause),
                    "resume" => Some(MusicCommand::Resume),
                    "stop" => Some(MusicCommand::Stop),
                    _ => None,
                };
                if let (Some(sender), Some(command)) = (locked.commands.clone(), command) {
                    let _ = sender.send(command);
                }
            }
        }
        MusicPacket::Seek {
            session_id,
            item_id,
            observed_revision,
            observed_playback_epoch,
            request_id,
            target_seconds,
        } => {
            let Some(requester_id) = participant_id else {
                return;
            };
            let commands = {
                let locked = runtime.lock().await;
                if locked.snapshot.session_id != session_id
                    || locked.snapshot.host_id.as_deref() != Some(&local_id)
                    || locked.snapshot.current_item_id.as_deref() != Some(&item_id)
                {
                    return;
                }
                locked.commands.clone()
            };
            let Some(commands) = commands else {
                return;
            };
            let (result, accepted) = tokio::sync::oneshot::channel();
            let request = MusicSeekRequest {
                session_id: session_id.clone(),
                item_id: item_id.clone(),
                requester_id,
                observed_revision,
                observed_playback_epoch,
                request_id: request_id.clone(),
                target_seconds,
            };
            if commands
                .send(MusicCommand::Seek { request, result })
                .is_err()
            {
                return;
            }
            let room = room.clone();
            let runtime = runtime.clone();
            tokio::spawn(async move {
                let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), accepted)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .and_then(Result::ok);
                let (accepted, revision, playback_epoch, elapsed_seconds) = {
                    let locked = runtime.lock().await;
                    let current = &locked.snapshot;
                    match outcome.filter(|outcome| {
                        current.session_id == session_id
                            && current.current_item_id.as_deref() == Some(&item_id)
                            && current.playback_epoch == outcome.playback_epoch
                            && current.revision >= outcome.revision
                    }) {
                        Some(outcome) => (
                            true,
                            outcome.revision,
                            outcome.playback_epoch,
                            outcome.elapsed_seconds,
                        ),
                        None => (
                            false,
                            current.revision,
                            current.playback_epoch,
                            current.elapsed_seconds,
                        ),
                    }
                };
                let _ = publish_packet(
                    &room,
                    &MusicPacket::SeekResult {
                        session_id,
                        item_id,
                        request_id,
                        accepted,
                        revision,
                        playback_epoch,
                        elapsed_seconds,
                    },
                )
                .await;
            });
        }
        MusicPacket::SeekResult {
            session_id,
            item_id,
            request_id,
            accepted,
            revision,
            playback_epoch,
            elapsed_seconds,
        } => {
            let mut locked = runtime.lock().await;
            if locked.snapshot.session_id != session_id
                || locked.snapshot.current_item_id.as_deref() != Some(&item_id)
                || locked.snapshot.host_id.as_deref() != participant_id.as_deref()
                || (accepted
                    && (locked.snapshot.playback_epoch != playback_epoch
                        || locked.snapshot.revision < revision
                        || locked.snapshot.elapsed_seconds < elapsed_seconds))
            {
                return;
            }
            if let Some(result) = locked.pending_seeks.remove(&request_id) {
                let _ = result.send(accepted);
            }
        }
        MusicPacket::Handoff { state, mut item } => {
            let Some(packet_sender) = participant_id.as_deref() else {
                return;
            };
            let mut locked = runtime.lock().await;
            if !accepts_handoff(&locked.snapshot, &state, &item, packet_sender) {
                return;
            }
            item.playback_source.clear();
            locked.set_snapshot(state.clone());
            drop(locked);

            if state.host_id.as_deref() == Some(&local_id) {
                start_assignment(
                    room.clone(),
                    runtime.clone(),
                    HostAssignment {
                        session_id: state.session_id.clone(),
                        coordinator_id: state
                            .coordinator_id
                            .clone()
                            .unwrap_or_else(|| packet_sender.to_owned()),
                        revision: state.revision,
                        item,
                        queue: state.queue.clone(),
                        validate_snapshot: true,
                    },
                )
                .await;
            }
        }
        MusicPacket::SyncRequest => advertise_local_state(room, runtime).await,
    }
}

pub(super) async fn advertise_local_state(room: &Room, runtime: &Mutex<MusicRuntime>) {
    let state = {
        let locked = runtime.lock().await;
        if locked.reconnect_host.is_some()
            || locked.snapshot.host_id.as_deref()
                != Some(&room.local_participant().identity().to_string())
        {
            return;
        }
        locked.snapshot.clone()
    };
    let _ = publish_packet(room, &MusicPacket::State { state }).await;
}

pub(in crate::voice) async fn request_sync(room: &Room) {
    let _ = publish_packet(room, &MusicPacket::SyncRequest).await;
}

pub(in crate::voice) async fn participant_disconnected(
    room: &Arc<Room>,
    runtime: &Arc<Mutex<MusicRuntime>>,
    participant_id: &str,
) {
    let mut locked = runtime.lock().await;
    if !locked.snapshot.active {
        return;
    }
    if locked.snapshot.host_id.as_deref() != Some(participant_id) {
        if locked.snapshot.host_id.as_deref()
            == Some(&room.local_participant().identity().to_string())
        {
            if let Some(commands) = locked.commands.clone() {
                let _ = commands.send(MusicCommand::ParticipantLeft(participant_id.into()));
            }
        }
        locked
            .snapshot
            .queue
            .retain(|item| item_owner(item) != Some(participant_id));

        locked.notify();
        return;
    }
    locked.pending_additions.clear();
    locked.pending_imports.clear();
    locked.pending_seeks.clear();
    let Some((state, next)) = elect_successor(room, &locked.snapshot, participant_id) else {
        locked.stop_tasks();
        let state = departed_state(&locked.snapshot);
        locked.set_snapshot(state);
        locked.owned_items.clear();
        locked.owned_sources.clear();
        locked.prepared_media.clear();
        return;
    };
    let next_owner = state.host_id.clone().unwrap_or_default();
    locked.set_snapshot(state.clone());
    locked.provisional_host = next_owner != room.local_participant().identity().to_string();
    drop(locked);

    if next_owner == room.local_participant().identity().to_string() {
        start_assignment(
            room.clone(),
            runtime.clone(),
            HostAssignment {
                session_id: state.session_id.clone(),
                coordinator_id: state
                    .coordinator_id
                    .clone()
                    .unwrap_or_else(|| next_owner.clone()),
                revision: state.revision,
                item: next,
                queue: state.queue,
                validate_snapshot: true,
            },
        )
        .await;
    }
}

async fn start_assignment(
    room: Arc<Room>,
    runtime: Arc<Mutex<MusicRuntime>>,
    assignment: HostAssignment,
) {
    let revision = assignment.revision;
    let session_id = assignment.session_id.clone();
    let item_id = assignment.item.id.clone();
    if let Err(error) = start_local_host(room.clone(), runtime.clone(), assignment).await {
        let state = {
            let mut locked = runtime.lock().await;
            if locked.snapshot.session_id != session_id
                || locked.snapshot.revision != revision
                || locked.snapshot.current_item_id.as_deref() != Some(&item_id)
                || locked.snapshot.host_id.as_deref()
                    != Some(&room.local_participant().identity().to_string())
                || locked.commands.is_some()
            {
                return;
            }
            let state = failed_assignment(&locked.snapshot, error);
            locked.stop_tasks();
            locked.owned_items.clear();
            locked.owned_sources.clear();
            locked.prepared_media.clear();
            locked.set_snapshot(state.clone());
            state
        };
        let _ = publish_packet(&room, &MusicPacket::State { state }).await;
    }
}

fn failed_assignment(previous: &MusicSnapshot, error: String) -> MusicSnapshot {
    MusicSnapshot {
        revision: previous.revision.saturating_add(1),
        error,
        ..departed_state(previous)
    }
}

fn reconnect_successor(before: &MusicSnapshot, state: &MusicSnapshot, sender: &str) -> bool {
    before.active
        && state.session_id == before.session_id
        && before.host_id.as_deref() != Some(sender)
        && state.revision > before.revision
        && state.current_item_id != before.current_item_id
}

fn departed_state(previous: &MusicSnapshot) -> MusicSnapshot {
    // Keep the revision fence; an in-flight snapshot must not revive the departed host.
    MusicSnapshot {
        session_id: previous.session_id.clone(),
        revision: previous.revision,
        host_id: previous.host_id.clone(),
        ..Default::default()
    }
}

fn accepts_state(
    current: &MusicSnapshot,
    state: &MusicSnapshot,
    sender: &str,
    current_host_present: bool,
    incoming_wins_conflict: bool,
) -> bool {
    let same_session = !state.session_id.is_empty() && state.session_id == current.session_id;
    if same_session {
        let same_host = current.host_id.as_deref() == Some(sender);
        state.revision > current.revision
            && if same_host {
                current.active
            } else {
                !current.active || !current_host_present
            }
    } else if !current.active {
        state.active
    } else {
        incoming_wins_conflict
    }
}

fn accepts_handoff(
    current: &MusicSnapshot,
    state: &MusicSnapshot,
    item: &MusicQueueItem,
    sender: &str,
) -> bool {
    state.host_id.as_deref() == item_owner(item)
        && state.current_item_id.as_deref() == Some(&item.id)
        && state.active
        && current.session_id == state.session_id
        && current.host_id.as_deref() == Some(sender)
        && state.revision > current.revision
        && state.current_item_id != current.current_item_id
}

fn elect_successor(
    room: &Room,
    state: &MusicSnapshot,
    departing: &str,
) -> Option<(MusicSnapshot, MusicQueueItem)> {
    let local = room.local_participant();
    let local_id = local.identity().to_string();
    let participants = room.remote_participants();
    departure_successor(state, departing, |owner| {
        if owner == local_id {
            return grant_allows_microphone(local.permission().as_ref());
        }
        participants.iter().any(|(id, participant)| {
            id.to_string() == owner && grant_allows_microphone(participant.permission().as_ref())
        })
    })
}

// The same election runs before a graceful leave and after an abrupt disconnect.
fn departure_successor(
    previous: &MusicSnapshot,
    departing: &str,
    is_present: impl Fn(&str) -> bool,
) -> Option<(MusicSnapshot, MusicQueueItem)> {
    if !previous.active || previous.host_id.as_deref() != Some(departing) {
        return None;
    }
    let mut queue = previous.queue.iter().filter(|item| {
        item_owner(item).is_some_and(|owner| owner != departing && is_present(owner))
    });
    let next = queue.next()?.clone();
    let owner = item_owner(&next)?.to_owned();
    let state = MusicSnapshot {
        session_id: previous.session_id.clone(),
        revision: previous.revision.saturating_add(1),
        coordinator_id: previous.coordinator_id.clone(),
        active: true,
        host_id: Some(owner),
        current_item_id: Some(next.id.clone()),
        title: next.title.clone(),
        source_url: next.source_url.clone(),
        artwork_url: next.artwork_url.clone(),
        requested_by: next.requested_by.clone(),
        loading: true,
        queue: queue.cloned().collect(),
        ..Default::default()
    };
    Some((state, next))
}

pub(super) async fn leave(room: &Room, runtime: &Mutex<MusicRuntime>) {
    let local = room.local_participant().identity().to_string();
    let (assignment, host_task) = {
        let mut locked = runtime.lock().await;
        let assignment = elect_successor(room, &locked.snapshot, &local);
        let host_task = locked.host_task.take();
        locked.stop_tasks();
        (assignment, host_task)
    };
    if let Some(task) = host_task {
        // Fence off the old worker before publishing a newer revision under
        // the same session ID. The normal disconnect fallback needs no ACK.
        task.abort();
        let _ = task.await;
    }
    if let Some((state, item)) = assignment {
        let _ = publish_packet(room, &MusicPacket::Handoff { state, item }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::music::{queue::queue_item, transport::encode_packet};

    fn playing(queue: Vec<MusicQueueItem>) -> MusicSnapshot {
        MusicSnapshot {
            active: true,
            host_id: Some("alice".into()),
            session_id: "alice:session".into(),
            coordinator_id: Some("alice".into()),
            revision: 12,
            queue,
            ..Default::default()
        }
    }

    fn track(owner: &str, source: &str) -> MusicQueueItem {
        queue_item(source.into(), owner.into(), source.starts_with('/'))
    }

    #[test]
    fn leave_packet_and_disconnect_fallback_elect_same_successor() {
        let private_file = "/home/bob/private-dir/song.wav";
        let bob = track("bob", private_file);
        let carol = track("carol", "https://example.com/carol");
        let previous = playing(vec![
            track("alice", "https://example.com/alice/a"),
            track("absent", "https://example.com/absent"),
            bob.clone(),
            track("alice", "https://example.com/alice/b"),
            carol.clone(),
        ]);
        let (fallback, item) =
            departure_successor(&previous, "alice", |owner| matches!(owner, "bob" | "carol"))
                .unwrap();
        assert_eq!(item.id, bob.id);
        assert_eq!(fallback.revision, 13);
        assert_eq!(fallback.coordinator_id, previous.coordinator_id);
        assert_eq!(fallback.queue.len(), 1);
        assert_eq!(fallback.queue[0].id, carol.id);
        let payload = encode_packet(&MusicPacket::Handoff {
            state: fallback.clone(),
            item,
        })
        .unwrap();
        assert!(!std::str::from_utf8(&payload)
            .unwrap()
            .contains("private-dir"));
        let packet: MusicPacket = serde_json::from_slice(&payload).unwrap();
        assert!(packet.is_valid_inbound());
        let MusicPacket::Handoff { state, item } = packet else {
            unreachable!()
        };
        assert!(item.playback_source.is_empty());
        assert!(accepts_handoff(&previous, &state, &item, "alice"));
        assert!(!accepts_handoff(&previous, &state, &item, "carol"));
        assert!(!accepts_handoff(&fallback, &state, &item, "alice"));
        assert!(departure_successor(&state, "alice", |_| true).is_none());
    }

    #[test]
    fn public_current_track_is_removed_even_though_it_is_transferable() {
        let bob = track("bob", "/home/bob/private.wav");
        let previous = MusicSnapshot {
            current_item_id: Some("alice:current".into()),
            requested_by: Some("alice".into()),
            transfer_url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()),
            ..playing(vec![bob.clone()])
        };
        let (state, item) =
            departure_successor(&previous, "alice", |owner| owner == "bob").unwrap();
        assert_eq!(item.id, bob.id);
        assert_eq!(state.current_item_id.as_deref(), Some(bob.id.as_str()));
        assert_eq!(state.requested_by.as_deref(), Some("bob"));
        assert!(state.transfer_url.is_none());
        assert!(state.queue.is_empty());

        let empty = MusicSnapshot {
            queue: vec![],
            ..previous
        };
        assert!(departure_successor(&empty, "alice", |_| true).is_none());
        let stopped = departed_state(&empty);
        assert!(!stopped.active && !stopped.loading);
        assert!(stopped.current_item_id.is_none());
        assert!(!accepts_state(&stopped, &empty, "alice", true, false));
        let fresh = MusicSnapshot {
            session_id: "alice:new".into(),
            ..empty
        };
        assert!(accepts_state(&stopped, &fresh, "alice", true, false));
    }

    #[test]
    fn departing_track_cannot_transfer_a_private_source() {
        for source in [
            "/home/alice/song.mp3",
            "https://example.com/private/token",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&token=private",
        ] {
            let previous = MusicSnapshot {
                current_item_id: Some("alice:current".into()),
                transfer_url: Some(source.into()),
                ..playing(vec![])
            };
            assert!(departure_successor(&previous, "alice", |_| true).is_none());
            let payload = encode_packet(&MusicPacket::State { state: previous }).unwrap();
            assert!(!String::from_utf8_lossy(&payload).contains(source));
        }
    }

    #[test]
    fn departure_without_another_present_owner_does_not_handoff() {
        let previous = playing(vec![
            track("alice", "https://example.com/alice"),
            track("bob", "https://example.com/bob"),
        ]);
        assert!(departure_successor(&previous, "alice", |owner| owner == "alice").is_none());
        assert!(departure_successor(&previous, "bob", |_| true).is_none());
    }

    #[test]
    fn failed_handoff_start_clears_loading_and_accepts_a_new_session() {
        let previous = playing(vec![track("bob", "/home/bob/song.wav")]);
        let (loading, _) = departure_successor(&previous, "alice", |_| true).unwrap();
        let stopped = failed_assignment(&loading, "Could not publish".into());
        assert!(loading.loading);
        assert!(!stopped.loading && !stopped.active);
        assert!(accepts_state(&loading, &stopped, "bob", true, false));
        assert!(!accepts_state(&stopped, &loading, "bob", true, false));
        let fresh = MusicSnapshot {
            session_id: "bob:new".into(),
            ..loading
        };
        assert!(accepts_state(&stopped, &fresh, "bob", true, false));
    }

    #[test]
    fn reconnect_fences_the_old_worker_without_restarting_the_successor_song() {
        let before = MusicSnapshot {
            current_item_id: Some("alice:current".into()),
            ..playing(vec![track("bob", "/home/bob/song.wav")])
        };
        let (loading, _) = departure_successor(&before, "alice", |_| true).unwrap();
        let surviving = MusicSnapshot {
            revision: loading.revision + 1,
            loading: false,
            elapsed_seconds: 24,
            ..loading
        };
        let stale_local = MusicSnapshot {
            revision: surviving.revision + 10,
            ..before.clone()
        };
        assert!(!accepts_state(&stale_local, &surviving, "bob", true, false));
        assert!(reconnect_successor(&before, &surviving, "bob"));
        assert!(!reconnect_successor(&before, &before, "bob"));
        let mut runtime = MusicRuntime::default();
        runtime.reconnect_host = Some(before);
        runtime.set_snapshot(stale_local);
        runtime.set_snapshot(surviving);
        assert!(runtime.reconnect_host.is_none());
        assert_eq!(runtime.snapshot.elapsed_seconds, 24);
        assert!(!runtime.snapshot.loading);
    }

    #[test]
    fn rejoin_accepts_successor_queue_but_not_a_delayed_pre_stop_snapshot() {
        let previous = playing(vec![
            track("bob", "https://example.com/bob"),
            track("carol", "https://example.com/carol"),
        ]);
        let (mut state, _) = departure_successor(&previous, "alice", |_| true).unwrap();
        state.revision += 1;
        let payload = encode_packet(&MusicPacket::State { state }).unwrap();
        let MusicPacket::State { state } = serde_json::from_slice(&payload).unwrap() else {
            unreachable!()
        };
        assert!(accepts_state(
            &MusicSnapshot::default(),
            &state,
            "bob",
            true,
            false
        ));
        assert!(accepts_state(&previous, &state, "bob", false, false));
        assert_eq!(item_owner(&state.queue[0]), Some("carol"));
        let stopped = MusicSnapshot {
            active: false,
            revision: state.revision + 1,
            ..state.clone()
        };
        assert!(!accepts_state(&stopped, &state, "bob", true, false));
    }
}
