use std::{collections::HashMap, sync::Arc, time::Duration};

use livekit::{
    options::TrackPublishOptions,
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioSourceOptions, RtcAudioSource},
    },
};
use livekit_protocol::{ParticipantPermission, TrackSource as GrantSource};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{AbortHandle, JoinSet},
};

use crate::http::{AuthenticatedContext, ContextError, HttpPolicy};

use super::{
    apply_control,
    audio::{self, Capture},
    camera, credentials,
    listening::RemoteGain,
    music,
    presence::Presence,
    screen, soundboard, Command, VoiceError, VoiceJoinRequest, VoiceListeningGains, VoicePhase,
    VoicePresence, VoiceSnapshot,
};

pub(super) struct SoundboardStartup {
    pub inbox: mpsc::Receiver<soundboard::Play>,
    pub auto: Option<(
        soundboard::JoinSoundOptions,
        tokio::sync::OwnedSemaphorePermit,
    )>,
}

fn remote_participant_ids(room: &Room) -> std::collections::BTreeSet<String> {
    room.remote_participants()
        .into_keys()
        .map(|id| id.to_string())
        .collect()
}

fn refresh_remote_participants(room: &Room, state: &mut VoiceSnapshot) -> bool {
    if state.phase != VoicePhase::Connected {
        return false;
    }
    // Events can be queued while the room map advances. Read the map rather
    // than replaying deltas into a snapshot that may already include them.
    let current = remote_participant_ids(room);
    if current == state.remote_participants {
        return false;
    }
    state.remote_participants = current;
    true
}

const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn run(
    policy: Arc<HttpPolicy>,
    context: AuthenticatedContext,
    request: VoiceJoinRequest,
    mut commands: mpsc::Receiver<Command>,
    state_tx: watch::Sender<VoiceSnapshot>,
    presence_tx: mpsc::Sender<VoicePresence>,
    mut ready: oneshot::Sender<Result<(), VoiceError>>,
    sounds: SoundboardStartup,
    mut screen: screen::ScreenRoom,
    mut camera: camera::CameraRoom,
    listening_gains: VoiceListeningGains,
    music_inbox: mpsc::Receiver<music::Command>,
    music_updates: watch::Sender<music::MusicSnapshot>,
) -> Result<(), VoiceError> {
    let scope_end = policy.authenticated_scope_ended(&context);
    tokio::pin!(scope_end);
    let cancelled = VoiceError::Authentication(ContextError::Cancelled);
    let connection = async {
        let credentials = credentials::fetch_credentials(&policy, &context, &request.channel_id)
            .await
            .map_err(VoiceError::Credentials)?;
        let mut options = RoomOptions::default();
        options.auto_subscribe = false;
        options.join_retries = 0;
        options.connect_timeout = Duration::from_secs(12);
        Room::connect(&credentials.url, &credentials.token, options)
            .await
            .map_err(|_| VoiceError::Connection)
    };
    let (room, mut events) = match tokio::select! {
        biased;
        _ = ready.closed() => return Ok(()),
        _ = &mut scope_end => Err(cancelled.clone()),
        result = connection => result,
    } {
        Ok(result) => result,
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };

    let room = Arc::new(room);
    let setup = tokio::select! {
        biased;
        _ = ready.closed() => Err(VoiceError::Disconnected),
        _ = &mut scope_end => Err(cancelled.clone()),
        result = setup_microphone(&room, &request) => result,
    };
    let (mut microphone, mut capture) = match setup {
        Ok(audio) => audio,
        Err(error) => {
            let _ = close(&room).await;
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    let mut state = state_tx.borrow().clone();
    state.set_phase(VoicePhase::Connected);
    state.can_speak = capture.is_some();
    state.muted = !state.can_speak;
    update_microphone(&state, microphone.as_ref(), capture.as_ref());
    let local_id = room.local_participant().identity().to_string();
    state.remote_participants = remote_participant_ids(&room);
    let mut presence = Presence::default();
    let mut playback = Playback {
        gains: listening_gains.clone(),
        viewed_audio: Some(screen.viewed_audio.clone()),
        ..Playback::default()
    };
    playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
    state_tx.send_replace(state.clone());

    let music_cancel = tokio_util::sync::CancellationToken::new();
    let mut music_job = JoinSet::new();
    music_job.spawn(music::run(
        room.clone(),
        room.subscribe(),
        state_tx.subscribe(),
        music_inbox,
        music_updates,
        request.audio.output_device.clone(),
        request.audio.output_volume,
        listening_gains.clone(),
        music_cancel.clone(),
    ));
    let mut sound_jobs = JoinSet::new();
    let result = if ready.send(Ok(())).is_err() {
        Ok(())
    } else {
        sound_jobs.spawn(soundboard::run(
            room.clone(),
            state_tx.subscribe(),
            sounds.inbox,
            sounds.auto,
            request.audio.output_device.clone(),
            listening_gains,
        ));
        loop {
            tokio::select! {
                biased;
                _ = &mut scope_end => break Err(cancelled),
                command = commands.recv() => {
                    let Some((control, reply)) = command else { break Ok(()); };
                    if reply.is_closed() { continue; }
                    let result = apply_control(&mut state, control).map(|()| {
                        if state.effectively_muted() {
                            state.active_speakers.remove(&local_id);
                        }
                        update_microphone(&state, microphone.as_ref(), capture.as_ref());
                        state.clone()
                    });
                    if result.is_ok() {
                        // Stop the sink, including previously buffered PCM, before acknowledging deafen.
                        if state.deafened { playback.stop().await; }
                        playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                        state_tx.send_replace(state.clone());
                    }
                    let _ = reply.send(result);
                }
                command = camera.inbox.recv() => {
                    match command {
                        Some(camera::Command::Start(options, reply)) if !reply.is_closed() => {
                            let _ = reply.send(camera.start(room.clone(), state.phase == VoicePhase::Connected, options));
                        }
                        Some(camera::Command::Stop(reply)) => camera.stop(Some(reply)),
                        Some(camera::Command::View(target, reply)) if !reply.is_closed() => {
                            let connected = state.phase == VoicePhase::Connected;
                            if connected || target.is_none() {
                                camera.viewing.target = target;
                                camera.viewing.reconcile(&room, connected).await;
                                let _ = reply.send(Ok(()));
                            } else {
                                let _ = reply.send(Err(camera::CameraError::Disconnected));
                            }
                        }
                        None => break Ok(()),
                        _ => {}
                    }
                }
                result = camera.jobs.join_next(), if !camera.jobs.is_empty() => {
                    if let Some(result) = result {
                        camera.finished(result.unwrap_or(Err(camera::CameraError::Capture)));
                    }
                }
                command = screen.inbox.recv() => {
                    match command {
                        Some(screen::Command::Start(options, reply)) if !reply.is_closed() => {
                            let _ = reply.send(screen.sharing.start(room.clone(), state.phase == VoicePhase::Connected, options, &request.audio.output_device));
                        }
                        Some(screen::Command::Stop(reply)) => screen.sharing.stop(Some(reply)),
                        Some(screen::Command::View(target, reply)) if !reply.is_closed() => {
                            if state.phase == VoicePhase::Connected || target.is_none() {
                                screen.viewing.target = target;
                                playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                                screen.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                                let _ = reply.send(Ok(()));
                            } else {
                                let _ = reply.send(Err(screen::ScreenShareError::Disconnected));
                            }
                        }
                        None => break Ok(()),
                        _ => {}
                    }
                }
                result = screen.sharing.jobs.join_next(), if !screen.sharing.jobs.is_empty() => {
                    if let Some(result) = result {
                        screen.sharing.finished(result.unwrap_or(Err(screen::ScreenShareError::Capture)));
                    }
                }
                event = events.recv() => {
                    match event {
                        None => {
                            eprintln!("voice room: LiveKit event stream closed");
                            break Err(VoiceError::Disconnected);
                        }
                        Some(RoomEvent::Disconnected { reason }) => {
                            eprintln!("voice room: LiveKit disconnected ({reason:?})");
                            break Err(VoiceError::Disconnected);
                        }
                        Some(RoomEvent::Connected { participants_with_tracks }) => {
                            // Use the ordered event baseline: the live room map may already
                            // include joins whose events are still in this queue.
                            presence.seed(participants_with_tracks.into_iter().map(|(p, _)| p.identity().to_string()), &local_id);
                        }
                        Some(RoomEvent::Reconnecting) => {
                            presence.set_live(false);
                            state.set_phase(VoicePhase::Reconnecting);
                            screen.sharing.stop(None);
                            camera.stop(None);
                            screen.viewing.reconcile(&room, false).await;
                            camera.viewing.reconcile(&room, false).await;
                            update_microphone(&state, microphone.as_ref(), capture.as_ref());
                            state_tx.send_replace(state.clone());
                            playback.stop().await;
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                        }
                        Some(RoomEvent::Reconnected) => {
                            presence.set_live(true);
                            state.set_phase(VoicePhase::Connected);
                            state.remote_participants = remote_participant_ids(&room);
                            screen.viewing.reconcile(&room, true).await;
                            camera.viewing.reconcile(&room, true).await;
                            if !grant_allows_microphone(room.local_participant().permission().as_ref()) {
                                revoke(&mut state, &mut microphone, &mut capture);
                            }
                            update_microphone(&state, microphone.as_ref(), capture.as_ref());
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                            state_tx.send_replace(state.clone());
                        }
                        Some(RoomEvent::ActiveSpeakersChanged { speakers }) if state.phase == VoicePhase::Connected => {
                            let locally_muted = state.effectively_muted();
                            let active_speakers = speakers.into_iter()
                                .filter(|speaker| !locally_muted || !matches!(speaker, Participant::Local(_)))
                                .map(|speaker| speaker.identity().to_string())
                                .collect();
                            if state.active_speakers != active_speakers {
                                state.active_speakers = active_speakers;
                                state_tx.send_replace(state.clone());
                            }
                        }
                        Some(RoomEvent::ParticipantConnected(participant)) => {
                            if refresh_remote_participants(&room, &mut state) {
                                state_tx.send_replace(state.clone());
                            }
                            if let Some(event) = presence.update(participant.identity().to_string(), &local_id, true) {
                                let _ = presence_tx.try_send(event);
                            }
                        }
                        Some(RoomEvent::ParticipantDisconnected(participant)) => {
                            if let Some(event) = presence.update(participant.identity().to_string(), &local_id, false) {
                                let _ = presence_tx.try_send(event);
                            }
                            screen.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                            camera.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                            let membership_changed = refresh_remote_participants(&room, &mut state);
                            if state.active_speakers.remove(&participant.identity().to_string()) || membership_changed {
                                state_tx.send_replace(state.clone());
                            }
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                        }
                        Some(RoomEvent::ParticipantPermissionChanged { participant: Participant::Local(_), permission }) => {
                            screen.sharing.check_grant(permission.as_ref());
                            if !camera::allows_camera(permission.as_ref()) { camera.stop(None); }
                            if !grant_allows_microphone(permission.as_ref()) {
                                state.active_speakers.remove(&local_id);
                                revoke(&mut state, &mut microphone, &mut capture);
                                state_tx.send_replace(state.clone());
                            }
                        }
                        Some(RoomEvent::TrackPublished { publication, participant }) => {
                            if publication.kind() == TrackKind::Audio {
                                let selected = screen.viewing.target.as_deref()
                                    .is_some_and(|id| id == participant.identity().to_string());
                                let enabled = !state.deafened && state.phase == VoicePhase::Connected;
                                // LiveKit exposes the publication in the participant map before
                                // wiring its signaling callback. An earlier scan may have set
                                // is_desired without actually sending the subscription request.
                                publication.set_subscribed(wants_remote_audio(
                                    publication.source(), enabled, selected,
                                ));
                            }
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                            screen.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                            camera.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                        }
                        Some(RoomEvent::TrackSubscribed { .. } | RoomEvent::TrackUnsubscribed { .. }
                            | RoomEvent::TrackUnpublished { .. }) => {
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                            screen.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                            camera.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                        }
                        Some(RoomEvent::TrackMuted { .. } | RoomEvent::TrackUnmuted { .. }) => {
                            playback.reconcile(&room, &state, &request, screen.viewing.target.as_deref());
                            screen.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                            camera.viewing.reconcile(&room, state.phase == VoicePhase::Connected).await;
                        }
                        _ => {}
                    }
                }
                result = read_capture(&mut capture) => {
                    let failure = match (result, capture.as_mut()) {
                        (Err(_), _) => Some("input read failed"),
                        (Ok(()), Some(capture)) => match tokio::time::timeout(
                            Duration::from_millis(250), capture.send_frame(state.effectively_muted()),
                        ).await {
                            Ok(Ok(())) => None,
                            Ok(Err(_)) => Some("frame submission failed"),
                            Err(_) => Some("frame submission timed out"),
                        },
                        (Ok(()), None) => None,
                    };
                    if let Some(reason) = failure {
                        eprintln!("voice room: microphone stopped ({reason}); continuing listen-only");
                        microphone_failed(&mut state, &mut microphone, &mut capture, &local_id);
                        state_tx.send_replace(state.clone());
                    }
                }
                () = playback.reap(), if !playback.jobs.is_empty() => {}
            }
        }
    };
    drop(presence_tx);
    state.set_phase(VoicePhase::Disconnected);
    state.error = result.as_ref().err().cloned();
    state_tx.send_replace(state.clone());
    update_microphone(&state, microphone.as_ref(), capture.as_ref());
    drop(capture);
    drop(commands);
    camera.inbox.close();
    camera.stop(None);
    camera.viewing.stop().await;
    screen.inbox.close();
    screen.sharing.stop(None);
    screen.viewing.stop().await;
    music_cancel.cancel();
    sound_jobs.shutdown().await;
    playback.stop().await;
    let _ = music_job.join_next().await;
    if let Some(sharing) = screen.sharing.jobs.join_next().await {
        screen
            .sharing
            .finished(sharing.unwrap_or(Err(screen::ScreenShareError::Capture)));
    }
    if let Some(job) = camera.jobs.join_next().await {
        camera.finished(job.unwrap_or(Err(camera::CameraError::Capture)));
    }
    let result = result.and(close(&room).await);
    state.error = result.as_ref().err().cloned();
    state_tx.send_replace(state);
    result
}

async fn close(room: &Room) -> Result<(), VoiceError> {
    let close = room.close();
    tokio::time::timeout(CLOSE_TIMEOUT, close)
        .await
        .map_err(|_| VoiceError::Close)?
        .map_err(|_| VoiceError::Close)
}

async fn setup_microphone(
    room: &Room,
    request: &VoiceJoinRequest,
) -> Result<(Option<LocalAudioTrack>, Option<Capture>), VoiceError> {
    let participant = room.local_participant();
    if !request.can_speak || !grant_allows_microphone(participant.permission().as_ref()) {
        return Ok((None, None));
    }
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
    let track =
        LocalAudioTrack::create_audio_track("microphone", RtcAudioSource::Native(source.clone()));
    track.mute();
    tokio::time::timeout(
        Duration::from_secs(8),
        participant.publish_track(
            LocalTrack::Audio(track.clone()),
            TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            },
        ),
    )
    .await
    .map_err(|_| VoiceError::Publish)?
    .map_err(|_| VoiceError::Publish)?;
    if !grant_allows_microphone(participant.permission().as_ref()) {
        return Ok((None, None));
    }
    let capture = Capture::open(&request.audio, source).await?;
    if !grant_allows_microphone(participant.permission().as_ref()) {
        return Ok((None, None));
    }
    Ok((Some(track), Some(capture)))
}

pub(super) fn grant_allows_microphone(permission: Option<&ParticipantPermission>) -> bool {
    permission.is_none_or(|permission| {
        permission.can_publish
            && (permission.can_publish_sources.is_empty()
                || permission
                    .can_publish_sources
                    .contains(&(GrantSource::Microphone as i32)))
    })
}

fn revoke(
    state: &mut VoiceSnapshot,
    microphone: &mut Option<LocalAudioTrack>,
    capture: &mut Option<Capture>,
) {
    state.can_speak = false;
    state.muted = true;
    update_microphone(state, microphone.as_ref(), capture.as_ref());
    *capture = None;
    *microphone = None;
}

fn update_microphone(
    state: &VoiceSnapshot,
    microphone: Option<&LocalAudioTrack>,
    capture: Option<&Capture>,
) {
    if let Some(track) = microphone {
        if state.effectively_muted() {
            track.mute();
            if let Some(capture) = capture {
                capture.clear();
            }
        } else {
            track.unmute();
        }
    }
}

// Match the lifetime boundary in Tauri: a failed microphone is not a failed room.
fn microphone_failed(
    state: &mut VoiceSnapshot,
    microphone: &mut Option<LocalAudioTrack>,
    capture: &mut Option<Capture>,
    local_id: &str,
) {
    revoke(state, microphone, capture);
    state.active_speakers.remove(local_id);
    state.error = Some(VoiceError::Capture);
}

async fn read_capture(capture: &mut Option<Capture>) -> Result<(), VoiceError> {
    match capture {
        Some(capture) => capture.read().await,
        None => std::future::pending().await,
    }
}

#[derive(Default)]
struct Playback {
    gains: VoiceListeningGains,
    viewed_audio: Option<watch::Sender<Option<String>>>,
    jobs: JoinSet<Result<(), VoiceError>>,
    tracks: HashMap<TrackSid, AbortHandle>,
}

fn wants_remote_audio(source: TrackSource, enabled: bool, selected: bool) -> bool {
    source != TrackSource::ScreenshareAudio || (enabled && selected)
}

impl Playback {
    async fn reap(&mut self) {
        // Like Tauri, a sink failure is local to this subscription. Keep its
        // entry until the track goes away so unrelated events don't respawn a
        // failing sink. A returning peer's new publication gets its own sink.
        match self.jobs.join_next().await {
            Some(Ok(Err(error))) => eprintln!("voice room: remote audio stopped: {error}"),
            Some(Err(error)) if !error.is_cancelled() => {
                eprintln!("voice room: remote audio task failed");
            }
            _ => {}
        }
    }

    async fn stop(&mut self) {
        self.tracks.clear();
        self.jobs.shutdown().await;
    }

    fn reconcile(
        &mut self,
        room: &Room,
        state: &VoiceSnapshot,
        request: &VoiceJoinRequest,
        target: Option<&str>,
    ) {
        let enabled = !state.deafened && state.phase == VoicePhase::Connected;
        let mut active = HashMap::new();
        let mut viewed_audio = None;
        for participant in room.remote_participants().into_values() {
            let identity = participant.identity().to_string();
            let selected = target.is_some_and(|id| identity == id);
            for publication in participant.track_publications().into_values() {
                if publication.kind() != TrackKind::Audio {
                    continue;
                }
                let stereo = publication.source() == TrackSource::ScreenshareAudio;
                if stereo && selected && state.phase == VoicePhase::Connected {
                    viewed_audio = Some(identity.clone());
                }
                let wanted = wants_remote_audio(publication.source(), enabled, selected);
                if publication.is_desired() != wanted {
                    publication.set_subscribed(wanted);
                }
                publication.set_enabled(enabled && wanted);
                if enabled && wanted && (!stereo || !publication.is_muted()) {
                    if let Some(RemoteTrack::Audio(track)) = publication.track() {
                        let gain = RemoteGain::for_track(
                            &publication.name(),
                            publication.source(),
                            &identity,
                        );
                        active.insert(publication.sid(), (track, stereo, gain));
                    }
                }
            }
        }
        if let Some(tx) = &self.viewed_audio {
            tx.send_if_modified(|current| {
                if *current == viewed_audio {
                    return false;
                }
                *current = viewed_audio;
                true
            });
        }
        self.tracks.retain(|sid, task| {
            let keep = active.contains_key(sid);
            if !keep {
                task.abort();
            }
            keep
        });
        for (sid, (track, stereo, gain)) in active {
            self.tracks.entry(sid).or_insert_with(|| {
                let device = request.audio.output_device.clone();
                let volume = request.audio.output_volume;
                let gains = self.gains.clone();
                self.jobs
                    .spawn(audio::play(track, device, volume, stereo, gain, gains))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_audio_stays_subscribed_while_deafened_but_screen_audio_requires_a_view() {
        for enabled in [false, true] {
            for selected in [false, true] {
                // Soundboard, music, and voice publish with the microphone source.
                assert!(wants_remote_audio(
                    TrackSource::Microphone,
                    enabled,
                    selected
                ));
                assert_eq!(
                    wants_remote_audio(TrackSource::ScreenshareAudio, enabled, selected),
                    enabled && selected,
                );
            }
        }
    }

    #[tokio::test]
    async fn failed_remote_sink_does_not_stop_other_publications() {
        let mut playback = Playback::default();
        let departed = TrackSid::try_from("TR_departed".to_owned()).unwrap();
        playback.tracks.insert(
            departed.clone(),
            playback.jobs.spawn(async { Err(VoiceError::Playback) }),
        );
        let (finish, running) = oneshot::channel();
        let other = playback.jobs.spawn(async move {
            let _ = running.await;
            Ok(())
        });

        playback.reap().await;
        assert!(playback.tracks[&departed].is_finished());
        assert!(!other.is_finished());

        // Reconciliation drops the old publication and starts a fresh one.
        playback.tracks.remove(&departed);
        let returned = TrackSid::try_from("TR_returned".to_owned()).unwrap();
        playback
            .tracks
            .insert(returned.clone(), playback.jobs.spawn(async { Ok(()) }));
        playback.reap().await;
        assert!(playback.tracks[&returned].is_finished());
        assert!(!other.is_finished());
        finish.send(()).unwrap();
        playback.stop().await;
        assert!(playback.jobs.is_empty());
    }

    #[test]
    fn microphone_failure_keeps_room_listening_and_preserves_remote_speakers() {
        let mut state = VoiceSnapshot {
            channel_id: "room".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: false,
            deafened: false,
            error: None,
            remote_participants: Default::default(),
            active_speakers: ["local".into(), "peer".into()].into(),
        };
        microphone_failed(&mut state, &mut None, &mut None, "local");
        assert_eq!(state.phase, VoicePhase::Connected);
        assert_eq!(state.error, Some(VoiceError::Capture));
        assert!(!state.can_speak);
        assert!(!state.deafened);
        assert!(state.effectively_muted());
        assert!(!state.is_speaking("local"));
        assert!(state.is_speaking("peer"));
        assert_eq!(
            apply_control(&mut state, super::super::Control::Mute(false)),
            Err(VoiceError::SpeakDenied)
        );
    }

    #[test]
    fn livekit_grant_must_allow_microphone_source() {
        let mut grant = ParticipantPermission::default();
        assert!(!grant_allows_microphone(Some(&grant)));
        grant.can_publish = true;
        assert!(grant_allows_microphone(Some(&grant)));
        grant.can_publish_sources.push(GrantSource::Camera as i32);
        assert!(!grant_allows_microphone(Some(&grant)));
        grant
            .can_publish_sources
            .push(GrantSource::Microphone as i32);
        assert!(grant_allows_microphone(Some(&grant)));
    }
}
