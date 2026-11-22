//! Native Stoat voice transport with PipeWire on Linux and WASAPI on Windows.
#[cfg_attr(target_os = "windows", path = "windows_audio.rs")]
mod audio;
pub mod camera;
mod credentials;
#[cfg_attr(target_os = "windows", path = "windows_devices.rs")]
pub mod devices;
mod listening;
pub mod music;
pub use listening::VoiceListeningGains;
pub use music::{MusicQueueItem, MusicSnapshot};
mod presence;
mod room;
pub use presence::VoicePresence;
pub mod screen;
mod video_view;
pub use video_view::VideoFrame;
mod soundboard;

use crate::soundboard::{validate_id, SoundboardError, SoundboardStore};
pub use soundboard::{preview_soundboard_clip, JoinSoundOptions, SOUNDBOARD_CAPACITY};

use std::{collections::BTreeSet, fmt, sync::Arc};

use tokio::{
    sync::{mpsc, oneshot, watch, Semaphore},
    task::JoinHandle,
};

use crate::http::{ContextError, HttpPolicy};

#[derive(Clone, Debug)]
pub struct VoiceJoinRequest {
    pub channel_id: String,
    /// Effective Stoat channel permissions, resolved by the caller. The server
    /// still authorizes the join and controls the LiveKit publish grant.
    pub can_connect: bool,
    pub can_speak: bool,
    pub audio: VoiceAudioOptions,
}

#[derive(Clone, Debug)]
pub struct VoiceAudioOptions {
    pub input_device: String,
    pub output_device: String,
    pub rnnoise: bool,
    pub echo_cancellation: bool,
    pub automatic_gain_control: bool,
    /// Linear PCM gains in percent; values above 100 boost with saturation.
    pub microphone_volume: u8,
    pub output_volume: u8,
}

impl Default for VoiceAudioOptions {
    fn default() -> Self {
        Self {
            input_device: "default".into(),
            output_device: "default".into(),
            rnnoise: true,
            echo_cancellation: true,
            automatic_gain_control: true,
            microphone_volume: 100,
            output_volume: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoicePhase {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSnapshot {
    pub channel_id: String,
    pub phase: VoicePhase,
    pub can_speak: bool,
    /// The user's mute preference, independent of deafen.
    pub muted: bool,
    pub deafened: bool,
    pub error: Option<VoiceError>,
    /// Last known LiveKit remote membership, retained during reconnect.
    pub remote_participants: BTreeSet<String>,
    /// LiveKit identities in the latest active-speaker snapshot for this room.
    pub active_speakers: BTreeSet<String>,
}

impl VoiceSnapshot {
    pub fn is_speaking(&self, participant_id: &str) -> bool {
        self.phase == VoicePhase::Connected && self.active_speakers.contains(participant_id)
    }

    fn set_phase(&mut self, phase: VoicePhase) {
        self.phase = phase;
        self.active_speakers.clear();
        if phase == VoicePhase::Disconnected {
            self.remote_participants.clear();
        }
    }

    pub fn effectively_muted(&self) -> bool {
        !self.can_speak || self.muted || self.deafened || self.phase != VoicePhase::Connected
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceError {
    InvalidChannel,
    ConnectDenied,
    SpeakDenied,
    Authentication(ContextError),
    Credentials(String),
    Connection,
    Publish,
    Capture,
    Playback,
    Disconnected,
    Close,
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidChannel => "Choose a voice channel first.",
            Self::ConnectDenied => "You do not have permission to join this voice channel.",
            Self::SpeakDenied => "You do not have permission to speak in this voice channel.",
            Self::Authentication(_) => "An active Stoat session is required for voice.",
            Self::Credentials(message) => message,
            Self::Connection => "Could not connect to the voice room.",
            Self::Publish => "Could not publish the microphone.",
            Self::Capture => "Microphone capture failed.",
            Self::Playback => "Voice audio playback failed.",
            Self::Disconnected => "The voice room is disconnected.",
            Self::Close => "Could not close the voice room cleanly.",
        })
    }
}

impl std::error::Error for VoiceError {}

enum Control {
    Mute(bool),
    Deafen(bool),
}
type Command = (Control, oneshot::Sender<Result<VoiceSnapshot, VoiceError>>);

/// Owns one room. Await `leave` before switching channels; dropping the handle
/// requests cleanup on the Tokio runtime without waiting for it.
pub struct VoiceSession {
    listening_gains: VoiceListeningGains,
    music: mpsc::Sender<music::Command>,
    music_state: watch::Receiver<MusicSnapshot>,
    commands: mpsc::Sender<Command>,
    state: watch::Receiver<VoiceSnapshot>,
    presence: Option<mpsc::Receiver<VoicePresence>>,
    sounds: mpsc::Sender<soundboard::Play>,
    sound_slot: Arc<Semaphore>,
    worker: JoinHandle<Result<(), VoiceError>>,
    camera: mpsc::Sender<camera::Command>,
    camera_state: watch::Receiver<camera::CameraState>,
    camera_frames: watch::Receiver<Option<Arc<VideoFrame>>>,
    screen: mpsc::Sender<screen::Command>,
    screen_state: watch::Receiver<screen::ScreenShareState>,
    viewed_screen_audio: watch::Receiver<Option<String>>,
    screen_frames: watch::Receiver<Option<Arc<screen::ScreenFrame>>>,
}

impl VoiceSession {
    pub async fn join(
        policy: Arc<HttpPolicy>,
        request: VoiceJoinRequest,
    ) -> Result<Self, VoiceError> {
        Self::join_with_soundboard(policy, request, None).await
    }

    /// The selected clip is resolved once after joining, without delaying ready
    /// or failing the voice connection if the selection cannot be played.
    pub async fn join_with_soundboard(
        policy: Arc<HttpPolicy>,
        request: VoiceJoinRequest,
        join_sound: Option<JoinSoundOptions>,
    ) -> Result<Self, VoiceError> {
        Self::join_with_listening_gains(policy, request, join_sound, VoiceListeningGains::default())
            .await
    }

    /// Join with shared local gains, including settings for the first join-cue frame.
    pub async fn join_with_listening_gains(
        policy: Arc<HttpPolicy>,
        mut request: VoiceJoinRequest,
        join_sound: Option<JoinSoundOptions>,
        listening_gains: VoiceListeningGains,
    ) -> Result<Self, VoiceError> {
        request.channel_id = request.channel_id.trim().to_owned();
        if request.channel_id.is_empty() {
            return Err(VoiceError::InvalidChannel);
        }
        if !request.can_connect {
            return Err(VoiceError::ConnectDenied);
        }
        let context = policy
            .authenticated_context()
            .map_err(VoiceError::Authentication)?;
        let (commands, inbox) = mpsc::channel(1);
        let (state_tx, state) = watch::channel(VoiceSnapshot {
            channel_id: request.channel_id.clone(),
            phase: VoicePhase::Connecting,
            can_speak: request.can_speak,
            muted: !request.can_speak,
            deafened: false,
            error: None,
            remote_participants: Default::default(),
            active_speakers: Default::default(),
        });
        let (ready_tx, ready) = oneshot::channel();
        let (presence_tx, presence) = mpsc::channel(16);
        let (sounds, sound_inbox) = mpsc::channel(SOUNDBOARD_CAPACITY);
        let sound_slot = Arc::new(Semaphore::new(SOUNDBOARD_CAPACITY));
        let auto = join_sound
            .filter(|options| !options.suppress_join_sound && request.can_speak)
            .and_then(|options| {
                sound_slot
                    .clone()
                    .try_acquire_owned()
                    .ok()
                    .map(|permit| (options, permit))
            });
        let soundboard = room::SoundboardStartup {
            inbox: sound_inbox,
            auto,
        };
        let (screen, screen_inbox) = mpsc::channel(1);
        let (screen_state_tx, screen_state) = watch::channel(screen::ScreenShareState::Idle);
        let (screen_frames_tx, screen_frames) = watch::channel(None);
        let (viewed_audio, viewed_screen_audio) = watch::channel(None);
        let screen_room = screen::ScreenRoom {
            inbox: screen_inbox,
            viewed_audio,
            sharing: screen::Sharing::new(screen_state_tx),
            viewing: video_view::Viewing::new(
                screen_frames_tx,
                livekit::prelude::TrackSource::Screenshare,
            ),
        };
        let (camera, camera_inbox) = mpsc::channel(1);
        let (camera_state_tx, camera_state) = watch::channel(camera::CameraState::Idle);
        let (camera_frames_tx, camera_frames) = watch::channel(None);
        let camera_room = camera::CameraRoom::new(camera_inbox, camera_state_tx, camera_frames_tx);
        let (music, music_inbox) = mpsc::channel(4);
        let (music_updates, music_state) = watch::channel(MusicSnapshot::default());
        let worker = tokio::spawn(room::run(
            policy,
            context,
            request,
            inbox,
            state_tx,
            presence_tx,
            ready_tx,
            soundboard,
            screen_room,
            camera_room,
            listening_gains.clone(),
            music_inbox,
            music_updates,
        ));
        ready.await.map_err(|_| VoiceError::Disconnected)??;
        Ok(Self {
            music,
            music_state,
            listening_gains,
            commands,
            state,
            presence: Some(presence),
            sounds,
            sound_slot,
            worker,
            camera,
            camera_state,
            camera_frames,
            screen,
            screen_state,
            viewed_screen_audio,
            screen_frames,
        })
    }

    /// Submit a URL, playlist, or absolute local audio path to the shared queue.
    /// Use the snapshot stream for resolution and playback results.
    pub async fn request_music(&self, url: impl Into<String>) -> Result<(), String> {
        self.music_operation(music::Operation::Append(url.into()))
            .await
    }

    /// Actions: pause, resume, stop, skip, remove and seek. Remove requires an
    /// item ID; seek requires all observation fields and a unique request ID.
    pub async fn control_music(
        &self,
        action: impl Into<String>,
        item_id: Option<String>,
        target_seconds: Option<u64>,
        observed_revision: Option<u64>,
        observed_playback_epoch: Option<u64>,
        request_id: Option<String>,
    ) -> Result<(), String> {
        self.music_operation(music::Operation::Control(
            action.into(),
            item_id,
            target_seconds,
            observed_revision,
            observed_playback_epoch,
            request_id,
        ))
        .await
    }

    pub fn music_snapshot(&self) -> MusicSnapshot {
        self.music_state.borrow().clone()
    }

    pub fn subscribe_music(&self) -> watch::Receiver<MusicSnapshot> {
        self.music_state.clone()
    }

    /// Local listening gain only, clamped to 0–100. Does not change published PCM.
    pub fn set_music_volume(&self, percent: u8) {
        self.listening_gains.set_music_listening_gain(percent);
    }

    async fn music_operation(&self, operation: music::Operation) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.music
            .send((operation, reply))
            .await
            .map_err(|_| "The voice room is disconnected.")?;
        result
            .await
            .map_err(|_| "The music operation was interrupted.")?
    }

    pub fn listening_gains(&self) -> VoiceListeningGains {
        self.listening_gains.clone()
    }

    pub fn snapshot(&self) -> VoiceSnapshot {
        self.state.borrow().clone()
    }
    /// Take the single consumer of live remote presence changes for this room.
    /// This is a best-effort cue stream, not a roster: no baseline is replayed
    /// and events may be dropped if the consumer falls behind.
    pub fn take_presence(&mut self) -> Option<mpsc::Receiver<VoicePresence>> {
        self.presence.take()
    }

    pub fn subscribe(&self) -> watch::Receiver<VoiceSnapshot> {
        self.state.clone()
    }

    pub async fn set_muted(&self, muted: bool) -> Result<VoiceSnapshot, VoiceError> {
        self.control(Control::Mute(muted)).await
    }

    pub async fn set_deafened(&self, deafened: bool) -> Result<VoiceSnapshot, VoiceError> {
        self.control(Control::Deafen(deafened)).await
    }

    async fn control(&self, control: Control) -> Result<VoiceSnapshot, VoiceError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send((control, reply))
            .await
            .map_err(|_| VoiceError::Disconnected)?;
        result.await.map_err(|_| VoiceError::Disconnected)?
    }

    /// Publish a clip at nominal gain, independent of local listening settings.
    /// Distinct IDs overlap; another submission of the same ID replaces it. Awaits the clip and queue drain
    /// (or replacement), not acknowledgement from listeners. Microphone mute
    /// does not mute this separate track.
    pub async fn play_soundboard_clip(
        &self,
        store: &SoundboardStore,
        id: &str,
    ) -> Result<(), SoundboardError> {
        validate_id(id)?;
        soundboard::check_state(&self.state.borrow())?;
        let permit = self
            .sound_slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| SoundboardError::Busy)?;
        let (store, id) = (store.clone(), id.to_owned());
        let (id, pcm, permit) =
            tokio::task::spawn_blocking(move || store.pcm(&id).map(|pcm| (id, pcm, permit)))
                .await
                .map_err(|_| SoundboardError::Playback)??;
        let (reply, result) = oneshot::channel();
        self.sounds
            .send(soundboard::Play {
                id,
                pcm,
                reply: Some(reply),
                _permit: permit,
            })
            .await
            .map_err(|_| SoundboardError::Disconnected)?;
        result.await.map_err(|_| SoundboardError::Disconnected)?
    }

    /// Requests fresh portal consent. Success means startup was accepted;
    /// observe `subscribe_screen_share()` for the publication outcome.
    pub async fn start_screen_share(
        &self,
        quality: screen::ScreenQuality,
    ) -> Result<(), screen::ScreenShareError> {
        self.start_screen_share_with_options(screen::ScreenShareOptions {
            quality,
            share_audio: false,
            #[cfg(windows)]
            selection: None,
        })
        .await
    }

    /// Requests fresh portal consent and optionally captures application playback.
    /// Observe `subscribe_screen_share()` for the publication outcome.
    pub async fn start_screen_share_with_options(
        &self,
        options: screen::ScreenShareOptions,
    ) -> Result<(), screen::ScreenShareError> {
        self.screen_control(|reply| screen::Command::Start(options, reply))
            .await
    }

    /// Cancels pending consent or stops capture and awaits cleanup.
    pub async fn stop_screen_share(&self) -> Result<(), screen::ScreenShareError> {
        self.screen_control(screen::Command::Stop).await
    }

    pub fn subscribe_screen_share(&self) -> watch::Receiver<screen::ScreenShareState> {
        self.screen_state.clone()
    }

    /// Selects one remote LiveKit identity for screen video and audio viewing.
    /// `None` unsubscribes. A selected participant may start sharing later.
    pub async fn set_screen_view(
        &self,
        participant_id: Option<String>,
    ) -> Result<(), screen::ScreenShareError> {
        self.screen_control(|reply| screen::Command::View(participant_id, reply))
            .await
    }

    /// Identity of the selected remote screen viewer target with a published audio track.
    pub fn subscribe_viewed_screen_audio(&self) -> watch::Receiver<Option<String>> {
        self.viewed_screen_audio.clone()
    }

    /// Latest frame only; `None` means no current frame. Keep a receiver for
    /// updates, and explicitly clear the view when no renderer needs it.
    pub fn subscribe_screen_frames(&self) -> watch::Receiver<Option<Arc<screen::ScreenFrame>>> {
        self.screen_frames.clone()
    }

    async fn screen_control(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<(), screen::ScreenShareError>>) -> screen::Command,
    ) -> Result<(), screen::ScreenShareError> {
        let (reply, result) = oneshot::channel();
        self.screen
            .send(make(reply))
            .await
            .map_err(|_| screen::ScreenShareError::Disconnected)?;
        result
            .await
            .map_err(|_| screen::ScreenShareError::Disconnected)?
    }

    /// Starts only on an explicit request. Observe the state for publication outcome.
    pub async fn start_camera(
        &self,
        options: camera::CameraOptions,
    ) -> Result<(), camera::CameraError> {
        self.camera_control(|reply| camera::Command::Start(options, reply))
            .await
    }
    pub async fn stop_camera(&self) -> Result<(), camera::CameraError> {
        self.camera_control(camera::Command::Stop).await
    }
    pub fn subscribe_camera(&self) -> watch::Receiver<camera::CameraState> {
        self.camera_state.clone()
    }
    pub async fn set_camera_view(&self, target: Option<String>) -> Result<(), camera::CameraError> {
        self.camera_control(|reply| camera::Command::View(target, reply))
            .await
    }
    pub fn subscribe_camera_frames(&self) -> watch::Receiver<Option<Arc<VideoFrame>>> {
        self.camera_frames.clone()
    }
    async fn camera_control(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<(), camera::CameraError>>) -> camera::Command,
    ) -> Result<(), camera::CameraError> {
        let (reply, result) = oneshot::channel();
        self.camera
            .send(make(reply))
            .await
            .map_err(|_| camera::CameraError::Disconnected)?;
        result
            .await
            .map_err(|_| camera::CameraError::Disconnected)?
    }

    pub async fn leave(self) -> Result<(), VoiceError> {
        drop(self.commands);
        self.worker.await.map_err(|_| VoiceError::Close)?
    }
}

fn apply_control(state: &mut VoiceSnapshot, control: Control) -> Result<(), VoiceError> {
    match control {
        Control::Mute(false) if !state.can_speak => return Err(VoiceError::SpeakDenied),
        Control::Mute(muted) => state.muted = muted,
        Control::Deafen(deafened) => state.deafened = deafened,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_transitions_preserve_membership_only_until_disconnect() {
        let mut state = VoiceSnapshot {
            channel_id: "channel".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: false,
            deafened: false,
            error: None,
            remote_participants: ["remote".into()].into(),
            active_speakers: ["me".into(), "remote".into()].into(),
        };
        assert!(state.is_speaking("me") && state.is_speaking("remote"));
        state.set_phase(VoicePhase::Reconnecting);
        assert!(state.remote_participants.contains("remote"));
        assert!(state.active_speakers.is_empty());
        state.set_phase(VoicePhase::Connected);
        assert!(!state.is_speaking("me") && !state.is_speaking("remote"));
        state.active_speakers.insert("remote".into());
        state.set_phase(VoicePhase::Disconnected);
        assert!(state.remote_participants.is_empty());
        assert!(state.active_speakers.is_empty());
    }

    #[test]
    fn deafen_preserves_the_mute_preference_and_cannot_grant_speak() {
        let mut state = VoiceSnapshot {
            channel_id: "channel".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: false,
            deafened: false,
            error: None,
            remote_participants: Default::default(),
            active_speakers: Default::default(),
        };
        apply_control(&mut state, Control::Deafen(true)).unwrap();
        assert!(state.effectively_muted());
        assert!(!state.muted);
        apply_control(&mut state, Control::Deafen(false)).unwrap();
        assert!(!state.effectively_muted());
        state.can_speak = false;
        state.muted = true;
        assert_eq!(
            apply_control(&mut state, Control::Mute(false)),
            Err(VoiceError::SpeakDenied)
        );
        assert!(state.effectively_muted());
    }
}
