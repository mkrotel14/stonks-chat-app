//! Stable per-message media views. Only geometry events reach the timeline;
//! video frames and progress repaint the active view directly.
use super::media_playback::{MediaKind, MediaPlayback, PlaybackSnapshot, PlaybackStatus};
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use stonks_core::{http::HttpPolicy, messages::models::TimelineAttachment};
use stonks_ui::{
    components::{Button, ButtonVariant, UiRange},
    i18n::t,
    icons::Icon,
    Theme,
};
mod download;
mod fullscreen;
mod metadata;
mod render;
#[derive(Clone, Debug, PartialEq)]
struct Spec {
    key: String,
    url: String,
    kind: MediaKind,
    title: String,
    size: Option<u64>,
    dimensions: Option<(u64, u64)>,
    width: f32,
    embedded: bool,
    poster: bool,
    autoplay: bool,
}
struct Work {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
struct Entry {
    view: Entity<MediaPlayer>,
    _layout: Subscription,
}
pub enum MediaPlayersEvent {
    LayoutChanged,
}
pub struct MediaPlayers {
    playback: Entity<MediaPlayback>,
    policy: Arc<HttpPolicy>,
    players: HashMap<String, Entry>,
    active: Option<String>,
    scope_revision: u64,
    _playback: Subscription,
}
impl EventEmitter<MediaPlayersEvent> for MediaPlayers {}
impl MediaPlayers {
    pub fn new(
        playback: Entity<MediaPlayback>,
        policy: Arc<HttpPolicy>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scope_revision = playback.read(cx).scope_revision();
        let subscription = cx.observe(&playback, |this, playback, cx| {
            let revision = playback.read(cx).scope_revision();
            if revision != this.scope_revision {
                this.scope_revision = revision;
                for entry in this.players.values() {
                    entry.view.update(cx, |player, cx| player.cancel(cx));
                }
                this.players.clear();
                this.active = None;
                cx.emit(MediaPlayersEvent::LayoutChanged);
                return;
            }
            let active = playback.read(cx).active_snapshot();
            let key = active.as_ref().map(|(key, _)| key.clone());
            if this.active != key {
                if let Some(previous) = this
                    .active
                    .as_ref()
                    .filter(|old| Some(*old) != key.as_ref())
                {
                    if let Some(entry) = this.players.get(previous) {
                        let idle = playback.read(cx).snapshot(previous);
                        entry
                            .view
                            .update(cx, |player, cx| player.snapshot(idle, cx));
                    }
                }
            }
            this.active = key;
            if let Some((key, snapshot)) = active {
                if let Some(entry) = this.players.get(&key) {
                    entry
                        .view
                        .update(cx, |player, cx| player.snapshot(snapshot, cx));
                }
            }
        });
        Self {
            playback,
            policy,
            players: HashMap::new(),
            active: None,
            scope_revision,
            _playback: subscription,
        }
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.playback.update(cx, |playback, cx| playback.stop(cx));
        for entry in self.players.values() {
            entry.view.update(cx, |player, cx| player.cancel(cx));
        }
        self.players.clear();
        self.active = None;
    }
    pub fn retain_keys(&mut self, keys: &HashSet<String>, cx: &mut Context<Self>) {
        if self
            .playback
            .read(cx)
            .active_key()
            .is_some_and(|key| !keys.contains(key))
        {
            self.playback.update(cx, |playback, cx| playback.stop(cx));
        }
        for (key, entry) in &self.players {
            if !keys.contains(key) {
                entry.view.update(cx, |player, cx| player.cancel(cx));
            }
        }
        self.players.retain(|key, _| keys.contains(key));
    }
    fn view(
        &mut self,
        spec: Spec,
        poster: Option<Arc<RenderImage>>,
        cx: &mut Context<Self>,
    ) -> Entity<MediaPlayer> {
        if let Some(entry) = self.players.get(&spec.key) {
            entry
                .view
                .update(cx, |player, cx| player.configure(spec, poster, cx));
            return entry.view.clone();
        }
        let key = spec.key.clone();
        let view = cx.new(|cx| {
            MediaPlayer::new(spec, poster, self.playback.clone(), self.policy.clone(), cx)
        });
        view.update(cx, |player, cx| player.preload_audio_metadata(cx));
        let subscription = cx.subscribe(&view, |this, _, event: &PlayerEvent, cx| match event {
            PlayerEvent::LayoutChanged => cx.emit(MediaPlayersEvent::LayoutChanged),
            PlayerEvent::Activated(key) => {
                if this
                    .playback
                    .read(cx)
                    .active_key()
                    .is_some_and(|active| active != key)
                {
                    this.playback.update(cx, |playback, cx| playback.stop(cx));
                }
            }
        });
        self.players.insert(
            key,
            Entry {
                view: view.clone(),
                _layout: subscription,
            },
        );
        view
    }
}
pub fn render_attachment(
    players: &Entity<MediaPlayers>,
    key: &str,
    file: &TimelineAttachment,
    width: f32,
    cx: &mut App,
) -> Option<AnyElement> {
    let kind = match file.kind.as_str() {
        "Audio" => MediaKind::Audio,
        "Video" => MediaKind::Video,
        _ => return None,
    };
    let url = file
        .original_url
        .as_deref()
        .or(file.url.as_deref())?
        .to_owned();
    let spec = Spec {
        key: key.into(),
        url,
        kind,
        title: file.filename.clone(),
        size: Some(file.size),
        dimensions: file.width.zip(file.height),
        width: width.clamp(1., 520.),
        embedded: false,
        poster: false,
        autoplay: kind == MediaKind::Audio,
    };
    Some(
        players
            .update(cx, |players, cx| players.view(spec, None, cx))
            .into_any_element(),
    )
}
pub fn render_video(
    players: &Entity<MediaPlayers>,
    key: &str,
    url: &str,
    title: &str,
    poster: Option<Arc<RenderImage>>,
    width: f32,
    dimensions: Option<(u64, u64)>,
    embedded: bool,
    autoplay: bool,
    cx: &mut App,
) -> AnyElement {
    let spec = Spec {
        key: key.into(),
        url: url.into(),
        kind: MediaKind::Video,
        title: title.into(),
        size: None,
        dimensions,
        width: width.clamp(1., 520.),
        embedded,
        poster: embedded || poster.is_some(),
        autoplay,
    };
    players
        .update(cx, |players, cx| players.view(spec, poster, cx))
        .into_any_element()
}
enum PlayerEvent {
    LayoutChanged,
    Activated(String),
}
pub struct MediaPlayer {
    enabled: bool,
    spec: Spec,
    poster: Option<Arc<RenderImage>>,
    playback: Entity<MediaPlayback>,
    policy: Arc<HttpPolicy>,
    current: PlaybackSnapshot,
    seek: Entity<SliderState>,
    volume: Entity<SliderState>,
    preferred_volume: f64,
    volume_hover: bool,
    volume_focus: FocusHandle,
    focus_wired: bool,
    scrub: Option<(f64, Instant)>,

    local_error: Option<String>,

    downloading: bool,
    download_generation: u64,
    download: Option<Work>,
    saved: bool,
    resume: Option<(f64, bool)>,
    audio_duration: f64,
    metadata: Option<Work>,
    metadata_generation: u64,
    fullscreen: Option<fullscreen::FullscreenWindow>,
    fullscreen_generation: u64,
    fullscreen_opening: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<PlayerEvent> for MediaPlayer {}
impl MediaPlayer {
    fn new(
        spec: Spec,
        poster: Option<Arc<RenderImage>>,
        playback: Entity<MediaPlayback>,
        policy: Arc<HttpPolicy>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|player, cx| fullscreen::release_window(player, cx))
            .detach();
        let enabled = playback.read(cx).has_session();
        let current = playback.read(cx).snapshot(&spec.key);
        let seek = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.000_001)
                .default_value(0.)
        });
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(1.)
        });
        let subscriptions = vec![
            cx.subscribe(&seek, |this, _, event, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    if this.current.duration.is_finite() && this.current.duration > 0. {
                        let next = f64::from(*value) * this.current.duration;
                        this.scrub = Some((next, Instant::now() + Duration::from_millis(450)));
                        this.current.position = next;
                        let key = this.spec.key.clone();
                        this.playback
                            .update(cx, |playback, cx| playback.seek(&key, next, cx));
                        cx.notify();
                    }
                }
            }),
            cx.subscribe(&volume, |this, _, event, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    this.preferred_volume = f64::from(*value).clamp(0., 1.);
                    let key = this.spec.key.clone();
                    this.playback.update(cx, |playback, cx| {
                        playback.set_volume(&key, this.preferred_volume, cx)
                    });
                    cx.notify();
                }
            }),
        ];
        Self {
            enabled,
            spec,
            poster,
            playback,
            policy,
            current,
            seek,
            volume,
            preferred_volume: 1.,
            volume_hover: false,
            volume_focus: cx.focus_handle(),
            focus_wired: false,
            scrub: None,

            local_error: None,

            downloading: false,
            download_generation: 0,
            download: None,
            saved: false,
            resume: None,
            audio_duration: 0.,
            metadata: None,
            metadata_generation: 0,
            fullscreen: None,
            fullscreen_generation: 0,
            fullscreen_opening: false,
            _subscriptions: subscriptions,
        }
    }
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.enabled = false;
        self.metadata = None;
        self.metadata_generation = self.metadata_generation.wrapping_add(1);
        self.close_fullscreen(cx);

        self.download = None;
        self.download_generation = self.download_generation.wrapping_add(1);

        self.downloading = false;
        cx.notify();
    }
    fn configure(&mut self, spec: Spec, poster: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        let source_changed = self.spec.url != spec.url || self.spec.kind != spec.kind;
        if source_changed {
            self.metadata = None;
            self.metadata_generation = self.metadata_generation.wrapping_add(1);
            self.audio_duration = 0.;
            self.close_fullscreen(cx);

            self.download = None;
            self.download_generation = self.download_generation.wrapping_add(1);

            self.downloading = false;
            self.saved = false;
            self.local_error = None;
            self.resume = None;
            self.scrub = None;
            if self.playback.read(cx).active_key() == Some(self.spec.key.as_str()) {
                self.playback.update(cx, |playback, cx| playback.stop(cx));
            }
            self.current = PlaybackSnapshot::default();
            cx.emit(PlayerEvent::LayoutChanged);
        }
        let poster_changed =
            self.poster.as_ref().map(|image| image.id) != poster.as_ref().map(|image| image.id);
        if self.spec != spec || poster_changed {
            self.spec = spec;
            self.poster = poster;
            cx.notify();
        }
        if source_changed {
            self.preload_audio_metadata(cx);
        }
    }
    fn geometry(&self) -> (bool, bool, bool, Option<(i32, i32)>) {
        let video = self.spec.kind == MediaKind::Video
            && !matches!(
                self.current.status,
                PlaybackStatus::Idle | PlaybackStatus::Failed
            );
        let dimensions = (!self.spec.embedded)
            .then(|| {
                self.current.frame.as_ref().map(|frame| {
                    let size = frame.size(0);
                    (size.width.0, size.height.0)
                })
            })
            .flatten();
        (video, self.error().is_some(), self.saved, dimensions)
    }
    fn snapshot(&mut self, mut snapshot: PlaybackSnapshot, cx: &mut Context<Self>) {
        let before = self.geometry();
        if matches!(
            snapshot.status,
            PlaybackStatus::Idle | PlaybackStatus::Failed
        ) {
            self.close_fullscreen(cx);
        }
        if self.spec.kind == MediaKind::Audio {
            if snapshot.duration > 0. {
                self.audio_duration = snapshot.duration;
            } else {
                snapshot.duration = self.audio_duration;
            }
        }
        if snapshot.status == PlaybackStatus::Idle && self.spec.kind == MediaKind::Audio {
            snapshot.position = self.current.position;
            snapshot.duration = self.current.duration;
        }
        if snapshot.status != PlaybackStatus::Idle {
            self.preferred_volume = snapshot.volume;
        }
        self.current = snapshot;
        if let Some((position, autoplay)) = self.resume.filter(|_| {
            matches!(
                self.current.status,
                PlaybackStatus::Paused | PlaybackStatus::Playing
            ) && self.current.duration > 0.
        }) {
            self.resume = None;
            let key = self.spec.key.clone();
            self.playback.update(cx, |playback, cx| {
                playback.seek(&key, position, cx);
                if autoplay {
                    playback.play(&key, cx);
                }
            });
        }
        if before != self.geometry() {
            cx.emit(PlayerEvent::LayoutChanged);
        }
        cx.notify();
    }
    fn error(&self) -> Option<&str> {
        self.local_error
            .as_deref()
            .or(self.current.error.as_deref())
    }
    fn toggle(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || self.current.status == PlaybackStatus::Loading {
            return;
        }
        self.local_error = None;
        self.saved = false;
        let key = self.spec.key.clone();
        match self.current.status {
            PlaybackStatus::Playing => self
                .playback
                .update(cx, |playback, cx| playback.pause(&key, cx)),
            PlaybackStatus::Paused => self
                .playback
                .update(cx, |playback, cx| playback.play(&key, cx)),
            PlaybackStatus::Ended => self.playback.update(cx, |playback, cx| {
                playback.seek(&key, 0., cx);
                playback.play(&key, cx);
            }),
            PlaybackStatus::Failed => self
                .playback
                .update(cx, |playback, cx| playback.retry(&key, cx)),
            _ => self.start(cx),
        }
        cx.emit(PlayerEvent::LayoutChanged);
        cx.notify();
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        self.local_error = None;
        cx.emit(PlayerEvent::Activated(self.spec.key.clone()));
        let mut autoplay = self.spec.autoplay;
        if self.spec.kind == MediaKind::Audio
            && self.current.position > 0.
            && self.current.position < self.current.duration
        {
            self.resume = Some((self.current.position, autoplay));
            autoplay = false;
        }
        let key = self.spec.key.clone();
        self.playback.update(cx, |playback, cx| {
            playback.load(
                key.clone(),
                self.spec.url.clone(),
                self.spec.kind,
                autoplay,
                cx,
            );
            playback.set_volume(&key, self.preferred_volume, cx);
        });
        cx.emit(PlayerEvent::LayoutChanged);
        cx.notify();
    }
    fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let waiting = self.scrub.is_some_and(|(target, deadline)| {
            Instant::now() < deadline && (self.current.position - target).abs() > 0.2
        });
        if !waiting {
            self.scrub = None;
            let desired = if self.current.duration > 0. {
                (self.current.position / self.current.duration).clamp(0., 1.) as f32
            } else {
                0.
            };
            if (self.seek.read(cx).value().start() - desired).abs() > 0.000_001 {
                self.seek
                    .update(cx, |seek, cx| seek.set_value(desired, window, cx));
            }
        }
        let volume = self.preferred_volume.clamp(0., 1.) as f32;
        if (self.volume.read(cx).value().start() - volume).abs() > 0.001 {
            self.volume
                .update(cx, |state, cx| state.set_value(volume, window, cx));
        }
    }
    fn download(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || self.downloading {
            return;
        }
        let url = self.spec.url.clone();
        self.local_error = None;
        self.saved = false;
        self.downloading = true;
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let name = download::filename(&self.spec.title);
        let selected = cx.prompt_for_new_path(&directory, Some(&name));
        let policy = self.policy.clone();
        self.download_generation = self.download_generation.wrapping_add(1);
        let generation = self.download_generation;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let operation = async move {
                match selected.await {
                    Ok(Ok(Some(path))) => spawn_core(Abortable::new(
                        async move { download::save(&policy, &url, &path).await },
                        registration,
                    ))
                    .await
                    .map_err(|_| "The download was cancelled.".to_owned())?
                    .map(|_| true),
                    Ok(Ok(None)) => Ok(false),
                    _ => Err("The save dialog could not be opened.".into()),
                }
            };
            let result = operation.await;
            this.update(cx, |this, cx| {
                if !this.enabled || this.download_generation != generation {
                    return;
                }
                let before = this.geometry();
                this.downloading = false;
                match result {
                    Ok(saved) => this.saved = saved,
                    Err(error) => this.local_error = Some(error),
                }
                if before != this.geometry() {
                    cx.emit(PlayerEvent::LayoutChanged);
                }
                cx.notify();
            })
            .ok();
        });
        self.download = Some(Work { abort, _task: task });
        cx.emit(PlayerEvent::LayoutChanged);
        cx.notify();
    }
}
fn clock(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "0:00".into();
    }
    let whole = seconds.max(0.).floor() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}
#[cfg(test)]
mod tests {
    use super::clock;
    #[test]
    fn clocks_match_browser_flooring_and_reject_nonfinite_values() {
        assert_eq!(clock(61.99), "1:01");
        assert_eq!(clock(-1.), "0:00");
        assert_eq!(clock(f64::NAN), "0:00");
        assert_eq!(clock(3600.), "60:00");
    }
}
