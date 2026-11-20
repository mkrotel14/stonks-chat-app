//! Credential-free attachment playback. One worker owns the platform playback session;
//! controls and decoded frames use bounded latest-value mailboxes.
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};
mod network;
#[cfg(target_os = "linux")]
mod probe;
#[cfg(target_os = "linux")]
pub use probe::probe_audio;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioMetadata {
    pub duration: f64,
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::probe_audio;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub async fn probe_audio(_source: Source) -> Result<AudioMetadata, PlaybackError> {
    Err(PlaybackError::PlatformUnsupported)
}
#[cfg(target_os = "linux")]
mod worker;
pub use network::{resolve_source, validate_url};

pub const MAX_VIDEO_WIDTH: u32 = 1280;
pub const MAX_VIDEO_HEIGHT: u32 = 720;
pub const MAX_FRAME_BYTES: usize = MAX_VIDEO_WIDTH as usize * MAX_VIDEO_HEIGHT as usize * 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    #[default]
    Idle,
    Loading,
    Paused,
    Playing,
    Ended,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackError {
    PlatformUnsupported,
    VideoTools,
    InvalidUrl,
    InvalidHeaders,
    Unavailable,
    Unsupported,
    Seek,
    Engine,
}
impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PlatformUnsupported => {
                "Native attachment playback is not supported on this platform."
            }
            Self::VideoTools => "Video playback requires FFmpeg and FFprobe. Install them or use a Stonks build that bundles media tools.",
            Self::InvalidUrl => {
                "Choose a valid HTTP or HTTPS media URL without embedded credentials."
            }
            Self::InvalidHeaders => "The media request headers are invalid.",
            Self::Unavailable => "The media could not be reached. Try again.",
            Self::Unsupported => {
                "This media format could not be played. Try again or download the file."
            }
            Self::Seek => "This media could not seek to that position.",
            Self::Engine => "The media player could not be started.",
        })
    }
}
impl std::error::Error for PlaybackError {}
#[derive(Clone, Debug)]
pub struct Source {
    pub key: String,
    pub url: String,
    pub kind: MediaKind,
    pub autoplay: bool,
    pub headers: Vec<(String, String)>,
}
impl Source {
    pub fn new(
        key: String,
        url: String,
        kind: MediaKind,
        autoplay: bool,
        headers: Vec<(String, String)>,
    ) -> Result<Self, PlaybackError> {
        if key.is_empty() || key.len() > 512 || key.chars().any(char::is_control) {
            return Err(PlaybackError::InvalidUrl);
        }
        let url = validate_url(&url)?.to_string();
        network::validate_headers(&headers)?;
        Ok(Self {
            key,
            url,
            kind,
            autoplay,
            headers,
        })
    }
}
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<[u8]>,
    pub serial: u64,
}
impl fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.bgra.len())
            .field("serial", &self.serial)
            .finish()
    }
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub key: Option<String>,
    pub status: PlaybackStatus,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub error: Option<PlaybackError>,
    pub frame: Option<VideoFrame>,
    pub revision: u64,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            key: None,
            status: PlaybackStatus::Idle,
            position: 0.,
            duration: 0.,
            volume: 1.,
            error: None,
            frame: None,
            revision: 0,
        }
    }
}
#[derive(Default)]
struct Desired {
    generation: u64,
    source: Option<Source>,
    key: Option<String>,
    playing: bool,
    seek: Option<f64>,
    volume: f64,
}
struct Shared {
    desired: Mutex<Desired>,
    snapshot: Mutex<Snapshot>,
    shutdown: AtomicBool,
    #[cfg(target_os = "linux")]
    finished: AtomicBool,
}
/// Playback handles never block the GUI thread. Drop signals the worker to
/// stop and release its media resources.
pub struct Player {
    shared: Arc<Shared>,
    thread: thread::Thread,
    _worker: thread::JoinHandle<()>,
}
impl Player {
    pub fn new() -> Result<Self, PlaybackError> {
        #[cfg(target_os = "linux")]
        {
            Self::with_audio(worker::AudioOutput::System)
        }
        #[cfg(target_os = "windows")]
        {
            Self::spawn(windows::run)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(PlaybackError::PlatformUnsupported)
        }
    }
    #[cfg(target_os = "linux")]
    fn with_audio(audio: worker::AudioOutput) -> Result<Self, PlaybackError> {
        Self::spawn(move |state| worker::run(state, audio))
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn spawn(run: impl FnOnce(Arc<Shared>) + Send + 'static) -> Result<Self, PlaybackError> {
        let shared = Arc::new(Shared {
            desired: Mutex::new(Desired {
                volume: 1.,
                ..Default::default()
            }),
            snapshot: Mutex::new(Snapshot::default()),
            shutdown: AtomicBool::new(false),
            #[cfg(target_os = "linux")]
            finished: AtomicBool::new(false),
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("stonks-media".into())
            .spawn(move || run(state))
            .map_err(|_| PlaybackError::Engine)?;
        Ok(Self {
            shared,
            thread: worker.thread().clone(),
            _worker: worker,
        })
    }
    pub fn load(&self, source: Source) -> Result<(), PlaybackError> {
        self.load_with_volume(source, 1.)
    }
    pub fn load_with_volume(&self, source: Source, volume: f64) -> Result<(), PlaybackError> {
        validate_url(&source.url)?;
        network::validate_headers(&source.headers)?;
        if !volume.is_finite() {
            return Err(PlaybackError::Engine);
        }
        let volume = volume.clamp(0., 1.);
        let mut desired = self
            .shared
            .desired
            .lock()
            .map_err(|_| PlaybackError::Engine)?;
        desired.generation = desired.generation.wrapping_add(1);
        desired.key = Some(source.key.clone());
        desired.playing = source.autoplay;
        desired.seek = None;
        desired.volume = volume;
        desired.source = Some(source.clone());
        if let Ok(mut snapshot) = self.shared.snapshot.lock() {
            let revision = snapshot.revision.wrapping_add(1);
            *snapshot = Snapshot {
                key: Some(source.key),
                status: PlaybackStatus::Loading,
                volume,
                revision,
                ..Default::default()
            };
        }
        drop(desired);
        self.thread.unpark();
        Ok(())
    }
    fn change(&self, key: &str, change: impl FnOnce(&mut Desired)) {
        if let Ok(mut desired) = self.shared.desired.lock() {
            if desired.key.as_deref() != Some(key) {
                return;
            }
            change(&mut desired);
        }
        self.thread.unpark();
    }
    pub fn play(&self, key: &str) {
        self.change(key, |desired| desired.playing = true);
    }
    pub fn pause(&self, key: &str) {
        self.change(key, |desired| desired.playing = false);
    }
    pub fn seek(&self, key: &str, position: f64) {
        if position.is_finite() {
            self.change(key, |desired| desired.seek = Some(position.max(0.)));
        }
    }
    pub fn set_volume(&self, key: &str, volume: f64) {
        if volume.is_finite() {
            self.change(key, |desired| desired.volume = volume.clamp(0., 1.));
        }
    }
    pub fn stop(&self) {
        if let Ok(mut desired) = self.shared.desired.lock() {
            desired.generation = desired.generation.wrapping_add(1);
            desired.source = None;
            desired.key = None;
            desired.playing = false;
            desired.seek = None;
        }
        if let Ok(mut snapshot) = self.shared.snapshot.lock() {
            let revision = snapshot.revision.wrapping_add(1);
            *snapshot = Snapshot {
                revision,
                ..Default::default()
            };
        }
        self.thread.unpark();
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|_| Snapshot {
                status: PlaybackStatus::Failed,
                error: Some(PlaybackError::Engine),
                ..Default::default()
            })
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.thread.unpark();
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests;
