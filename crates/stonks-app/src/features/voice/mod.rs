//! One audio call per authenticated shell, independent of text navigation.
mod camera;
mod cues;
mod dock;
mod listening;
mod music;
pub(super) mod roster;
mod screens;
mod soundboard;
mod video;

use crate::{runtime::spawn_core, session::Session};
use futures_util::{stream::FuturesUnordered, StreamExt};
use gpui_kit::*;
use std::{sync::Arc, time::Duration};
use stonks_core::{
    connection::ConnectionManager,
    http::HttpPolicy,
    voice::{
        screen::ScreenShareState, JoinSoundOptions, VoiceError, VoiceJoinRequest,
        VoiceListeningGains, VoicePhase, VoiceSession, VoiceSnapshot, SOUNDBOARD_CAPACITY,
    },
};
use tokio::sync::{mpsc, oneshot, watch};

const JOIN: u64 = (1 << 20) | (1 << 30) | (1 << 36); // ViewChannel, Connect, Listen
const SPEAK: u64 = 1 << 31;

#[derive(Default)]
pub struct Voice {
    call: Option<Call>,
    listening: listening::Listening,
    participant_volume: Option<roster::VolumePopover>,
    dock_overflow: Option<roster::Overflow>,
    error: Option<VoiceError>,
    suspended: bool,
    soundboard: Option<soundboard::Panel>,
    dock_position: Option<Point<Pixels>>,
    drag_offset: Point<Pixels>,
    viewer: Option<video::OpenViewer>,
    viewer_generation: u64,
    camera_setup_generation: u64,
}

struct Call {
    id: String,
    name: String,
    stop: Option<oneshot::Sender<()>>,
    controls: mpsc::Sender<Control>,
    sounds: mpsc::Sender<soundboard::Play>,
    snapshot: Option<VoiceSnapshot>,
    hidden: bool,
    screen: screens::ScreenUi,
    camera: camera::CameraUi,
    video: mpsc::UnboundedSender<video::View>,
    music: music::session::Music,
}

#[derive(Clone, Copy)]
enum Control {
    Mute(bool),
    Deafen(bool),
}

impl Voice {
    pub fn new(cx: &App) -> Self {
        Self {
            listening: listening::Listening::load(cx),
            ..Self::default()
        }
    }

    pub fn join(
        &mut self,
        id: &str,
        session: &Entity<Session>,
        policy: Arc<HttpPolicy>,
        cx: &mut Context<Self>,
    ) {
        if self.suspended || self.call.is_some() {
            return;
        }
        let session = session.read(cx);
        let Some(channel) = session.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .channels
                .iter()
                .find(|channel| channel.id == id && channel.is_voice())
        }) else {
            return;
        };
        let manager = session.authorization();
        if !session
            .server
            .as_ref()
            .is_some_and(|server| server.endpoints.livekit_enabled)
            || manager.require_channel_permission(id, JOIN).is_err()
        {
            self.error = Some(VoiceError::ConnectDenied);
            cx.notify();
            return;
        }
        let (music_jobs, music_inbox) = mpsc::channel(1);
        let (music_state, mut music_updates) =
            watch::channel(stonks_core::voice::MusicSnapshot::default());
        let (stop, stopped) = oneshot::channel();
        let (controls, inbox) = mpsc::channel(1);
        let (sounds, plays) = mpsc::channel(SOUNDBOARD_CAPACITY);
        let (updates, mut snapshots) = watch::channel(None);
        let (share, sharing) = mpsc::channel(1);
        let (view, viewing) = mpsc::unbounded_channel();
        let (screen_state, mut screen_updates) = watch::channel(ScreenShareState::Idle);
        let (camera, camera_worker, mut camera_updates) = camera::channel();
        self.error = None;
        self.call = Some(Call {
            id: id.to_owned(),
            name: channel.name.clone().unwrap_or_else(|| id.to_owned()),
            stop: Some(stop),
            controls,
            sounds,
            snapshot: None,
            hidden: false,
            screen: screens::ScreenUi {
                state: ScreenShareState::Idle,
                sharing: share,
                setup: None,
            },
            camera,
            video: view,
            music: music::session::Music::new(music_jobs),
        });
        let id = id.to_owned();
        let preferences = cx.global::<crate::preferences::NativePreferences>().clone();
        let worker = spawn_core(run(
            policy,
            manager,
            id,
            stopped,
            (
                inbox,
                plays,
                screens::ScreenWorker {
                    state: screen_state,
                    sharing,
                },
                camera_worker,
                viewing,
            ),
            updates,
            (music_inbox, music_state),
            preferences,
            self.listening.gains.clone(),
        ));
        cx.spawn(async move |this, cx| {
            loop {
                tokio::select! {
                    biased;
                    changed = snapshots.changed() => if changed.is_err() { break },
                    Ok(()) = music_updates.changed() => {
                        let playback = music_updates.borrow_and_update().clone();
                        if this.update(cx, |this, cx| {
                            if let Some(call) = &mut this.call { call.music.playback = playback; }
                            cx.notify();
                        }).is_err() { return; }
                        continue;
                    }
                    Ok(()) = camera_updates.changed() => {
                        let state = *camera_updates.borrow_and_update();
                        if this.update(cx, |this, cx| {
                            if let Some(call) = &mut this.call { call.camera.state = state; }
                            cx.notify();
                        }).is_err() { return; }
                        continue;
                    }
                    Ok(()) = screen_updates.changed() => {
                        let screen = *screen_updates.borrow_and_update();
                        if this.update(cx, |this, cx| {
                            if let Some(call) = &mut this.call { call.screen.state = screen; }
                            cx.notify();
                        }).is_err() { return; }
                        continue;
                    }
                }
                let snapshot = snapshots.borrow_and_update().clone();
                if this
                    .update(cx, |this, cx| {
                        if !snapshot
                            .as_ref()
                            .is_some_and(|state| state.phase == VoicePhase::Connected)
                        {
                            this.participant_volume = None;
                            this.dock_overflow = None;
                        }
                        if snapshot.is_none() {
                            this.close_viewer(cx);
                        }
                        if let Some(call) = &mut this.call {
                            if !snapshot
                                .as_ref()
                                .is_some_and(|s| s.phase == VoicePhase::Connected)
                            {
                                call.music.reset_card();
                                call.screen.setup = None;
                                call.camera.clear_setup(cx);
                            }
                            if !call.hidden
                                && snapshot.as_ref().and_then(|s| s.error.as_ref())
                                    == Some(&VoiceError::Capture)
                                && call.snapshot.as_ref().and_then(|s| s.error.as_ref())
                                    != Some(&VoiceError::Capture)
                            {
                                this.error = Some(VoiceError::Capture);
                            }
                            call.snapshot = snapshot;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| this.close_viewer(cx));
            let result = worker.await;
            if let Err(error) = &result {
                eprintln!("voice call: controller ended ({error})");
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(mut call) = this.call.take() {
                    call.camera.clear_setup(cx);
                    if !call.hidden {
                        this.error = result.err();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn active(&self, id: &str) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| !call.hidden && call.id == id)
    }

    pub(super) fn roster_state(&self) -> Option<&VoiceSnapshot> {
        self.call
            .as_ref()
            .filter(|call| !call.hidden && call.stop.is_some())?
            .snapshot
            .as_ref()
    }

    pub fn can_join(&self, session: &Session, id: &str) -> bool {
        !self.suspended
            && self.call.is_none()
            && session
                .server
                .as_ref()
                .is_some_and(|server| server.endpoints.livekit_enabled)
            && session.can_channel_permission(id, JOIN)
    }

    pub fn set_suspended(&mut self, suspended: bool, cx: &mut Context<Self>) {
        self.suspended = suspended;
        if suspended {
            self.reset(cx);
        }
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.soundboard = None;
        self.participant_volume = None;
        self.dock_overflow = None;
        self.close_viewer(cx);
        if let Some(call) = &mut self.call {
            call.hidden = true;
            call.music.reset_card();
            call.camera.clear_setup(cx);
            call.stop.take();
        }
        cx.notify();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.participant_volume = None;
        self.dock_overflow = None;
        self.close_viewer(cx);
        if let Some(call) = &mut self.call {
            call.camera.clear_setup(cx);
            call.music.reset_card();
            if let Some(stop) = call.stop.take() {
                let _ = stop.send(());
            }
        }
        cx.notify();
    }
}

fn require_live(
    manager: &ConnectionManager,
    id: &str,
    speaking_grant: bool,
) -> Result<(), VoiceError> {
    manager
        .require_channel_permission(id, JOIN)
        .map_err(|_| VoiceError::ConnectDenied)?;
    if speaking_grant {
        manager
            .require_channel_permission(id, SPEAK)
            .map_err(|_| VoiceError::SpeakDenied)?;
    }
    Ok(())
}

async fn run(
    policy: Arc<HttpPolicy>,
    manager: Arc<ConnectionManager>,
    id: String,
    mut stop: oneshot::Receiver<()>,
    (mut controls, mut plays, screen, camera, video): (
        mpsc::Receiver<Control>,
        mpsc::Receiver<soundboard::Play>,
        screens::ScreenWorker,
        camera::Worker,
        mpsc::UnboundedReceiver<video::View>,
    ),
    updates: watch::Sender<Option<VoiceSnapshot>>,
    (music_jobs, music_updates): (
        mpsc::Receiver<music::session::Job>,
        watch::Sender<stonks_core::voice::MusicSnapshot>,
    ),
    preferences: crate::preferences::NativePreferences,
    listening_gains: VoiceListeningGains,
) -> Result<(), VoiceError> {
    require_live(&manager, &id, false)?;
    let can_speak = manager.require_channel_permission(&id, SPEAK).is_ok();
    let request = VoiceJoinRequest {
        channel_id: id.clone(),
        can_connect: true,
        can_speak,
        audio: preferences.saved.voice_audio(),
    };
    // Check the manager directly: gateway permission state can change before GPUI renders.
    let mut permission_tick = tokio::time::interval(Duration::from_millis(200));
    permission_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let store = soundboard::store(&preferences.path);
    let mut room = {
        let join_sound = store.clone().map(|store| JoinSoundOptions {
            store,
            // This controller only initiates fresh joins, not Tauri-style rejoins.
            suppress_join_sound: false,
        });
        let join =
            VoiceSession::join_with_listening_gains(policy, request, join_sound, listening_gains);
        tokio::pin!(join);
        loop {
            tokio::select! {
                biased;
                _ = &mut stop => return Ok(()),
                _ = permission_tick.tick() => require_live(&manager, &id, can_speak)?,
                result = &mut join => break result?,
            }
        }
    };
    let mut presence = room.take_presence();
    let mut state = room.subscribe();
    let control_cues = cues::control_player(preferences.path.clone());
    let cues = cues::player(preferences.path);
    let mut established = false;
    let mut explicit_leave = false;
    let result = async {
        let mut playing = FuturesUnordered::new();
        let screens = screens::run(&room, screen, &cues);
        let camera = camera::run(&room, camera);
        let video = video::run(&room, video);
        let music = music::session::run(&room, music_jobs, music_updates);
        // Feature workers borrow the room but do not own its lifetime. In particular,
        // a side stream may close before the core publishes the terminal snapshot.
        let auxiliaries = async {
            tokio::join!(screens, camera, video, music);
            std::future::pending::<()>().await;
        };
        tokio::pin!(auxiliaries);
        loop {
            let current = state.borrow_and_update().clone();
            if current.phase == VoicePhase::Disconnected {
                return Err(current.error.unwrap_or(VoiceError::Disconnected));
            }
            require_live(&manager, &id, can_speak)?;
            let connected = matches!(current.phase, VoicePhase::Connected | VoicePhase::Reconnecting);
            if updates.send(Some(current)).is_err() {
                return Ok(());
            }
            if connected && !established {
                established = true;
                let _ = cues.try_send(cues::Cue::Join);
            }
            loop {
                tokio::select! {
                    biased;
                    stopped = &mut stop => {
                        explicit_leave = stopped.is_ok();
                        return Ok(());
                    }
                    changed = state.changed() => {
                        changed.map_err(|_| VoiceError::Disconnected)?;
                        break;
                    }
                    _ = permission_tick.tick() => require_live(&manager, &id, can_speak)?,
                    _ = &mut auxiliaries => unreachable!("auxiliary workers cannot end the call"),
                    event = async {
                        match &mut presence {
                            Some(events) => events.recv().await,
                            None => std::future::pending().await,
                        }
                    } => match event {
                        Some(event) if room.snapshot().phase == VoicePhase::Connected => {
                            let _ = cues.try_send(event.into());
                        }
                        Some(_) => {},
                        None => presence = None,
                    },
                    _ = playing.next(), if !playing.is_empty() => {},
                    Some(play) = plays.recv() => {
                        require_live(&manager, &id, can_speak)?;
                        if playing.len() < SOUNDBOARD_CAPACITY {
                            playing.push(play.run(&room, store.as_ref()));
                        } else {
                            play.reject_busy();
                        }
                    }
                    command = controls.recv() => {
                        require_live(&manager, &id, can_speak)?;
                        let Some(command) = command else { return Ok(()) };
                        let before = room.snapshot();
                        let result = tokio::select! {
                            biased;
                            stopped = &mut stop => {
                                explicit_leave = stopped.is_ok();
                                return Ok(());
                            }
                            result = async {
                                match command {
                                    Control::Mute(muted) => room.set_muted(muted).await,
                                    Control::Deafen(deafened) => room.set_deafened(deafened).await,
                                }
                            } => result,
                        };
                        // LiveKit can revoke publish independently of Stoat. Its snapshot
                        // disables unmute; a click racing that update is harmless.
                        match result {
                            Ok(after) => {
                                if let Some(cue) = cues::Cue::acknowledged(command, &before, &after) {
                                    let _ = control_cues.send(Some(cue));
                                }
                            }
                            // A command can lose its reply during core shutdown. Let
                            // the room's state stream report the terminal cause.
                            Err(VoiceError::SpeakDenied | VoiceError::Disconnected) => {},
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
        }
    }
    .await;
    // Clear the visible state before awaiting room and audio teardown.
    let _ = updates.send(None);
    drop(control_cues);
    if explicit_leave && established {
        let _ = cues.try_send(cues::Cue::Leave);
    }
    // Core has no setter for a changed Speak grant. Close instead of retaining
    // a stale publishing grant; the user can rejoin in listen-only mode.
    result.and(room.leave().await)
}

#[cfg(test)]
mod tests {
    use super::{require_live, ConnectionManager, VoiceError};

    #[test]
    fn voice_access_fails_closed_without_gateway_permissions() {
        let manager = ConnectionManager::default();
        assert_eq!(
            require_live(&manager, "channel", false),
            Err(VoiceError::ConnectDenied)
        );
        assert_eq!(
            require_live(&manager, "channel", true),
            Err(VoiceError::ConnectDenied)
        );
    }
}
