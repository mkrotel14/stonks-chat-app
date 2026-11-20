//! One channel/account-owned attachment player. The renderer receives a
//! bounded latest frame, never a queue of decoded media buffers.
#[cfg(not(target_os = "windows"))]
use crate::runtime::spawn_core;
use crate::session::Session;
use futures_util::future::AbortHandle;
#[cfg(not(target_os = "windows"))]
use futures_util::future::Abortable;
use gpui_kit::{Context, Entity, RenderImage, Subscription, Task};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub use stonks_core::media_playback::{MediaKind, PlaybackStatus};
use stonks_core::media_playback::{Player, Source};

#[derive(Clone, Debug)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub error: Option<String>,
    pub frame: Option<Arc<RenderImage>>,
    pub revision: u64,
}
impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
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
struct ResolveRequest {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for ResolveRequest {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
pub struct MediaPlayback {
    backend: Option<Player>,
    active: Option<String>,
    source: Option<Source>,
    snapshot: PlaybackSnapshot,
    channel: Option<String>,
    identity: Option<(String, String)>,
    generation: u64,
    scope_revision: u64,
    backend_revision: u64,
    frame_serial: u64,
    want_playing: bool,
    pending_seek: Option<(f64, Instant)>,
    request: Option<ResolveRequest>,
    poll: Option<Task<()>>,
    _session: Subscription,
}
fn identity(session: &Session) -> Option<(String, String)> {
    session
        .snapshot
        .as_ref()
        .zip(session.server.as_ref())
        .map(|(snapshot, server)| {
            (
                server.deployment_url.clone(),
                snapshot.current_user.id.clone(),
            )
        })
}
impl MediaPlayback {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let initial = identity(session.read(cx));
        let observe = cx.observe(&session, |this, session, cx| {
            let next = identity(session.read(cx));
            if next != this.identity {
                this.identity = next;
                this.scope_revision = this.scope_revision.wrapping_add(1);
                this.stop(cx);
            }
        });
        Self {
            backend: None,
            active: None,
            source: None,
            snapshot: Default::default(),
            channel: None,
            identity: initial,
            generation: 0,
            scope_revision: 0,
            backend_revision: 0,
            frame_serial: 0,
            want_playing: false,
            pending_seek: None,
            request: None,
            poll: None,
            _session: observe,
        }
    }
    pub fn has_session(&self) -> bool {
        self.identity.is_some()
    }
    pub fn scope_revision(&self) -> u64 {
        self.scope_revision
    }
    pub fn active_key(&self) -> Option<&str> {
        self.active.as_deref()
    }
    pub fn active_snapshot(&self) -> Option<(String, PlaybackSnapshot)> {
        self.active
            .as_ref()
            .map(|key| (key.clone(), self.snapshot.clone()))
    }
    pub fn snapshot(&self, key: &str) -> PlaybackSnapshot {
        if self.active.as_deref() == Some(key) {
            self.snapshot.clone()
        } else {
            PlaybackSnapshot::default()
        }
    }
    pub fn set_channel(&mut self, channel: Option<String>, cx: &mut Context<Self>) {
        if self.channel != channel {
            self.scope_revision = self.scope_revision.wrapping_add(1);
            self.stop(cx);
            self.channel = channel;
        }
    }
    pub fn load(
        &mut self,
        key: String,
        url: String,
        kind: MediaKind,
        autoplay: bool,
        cx: &mut Context<Self>,
    ) {
        self.load_with_headers(key, url, kind, autoplay, vec![], cx);
    }
    pub fn load_with_headers(
        &mut self,
        key: String,
        url: String,
        kind: MediaKind,
        autoplay: bool,
        headers: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        self.active = Some(key.clone());
        self.want_playing = autoplay;
        let source = match Source::new(key, url, kind, autoplay, headers) {
            Ok(source) => source,
            Err(error) => {
                self.fail(error.to_string(), cx);
                return;
            }
        };
        if self.identity.is_none() {
            self.fail("Log in before playing an attachment.".into(), cx);
            return;
        }
        self.source = Some(source.clone());
        self.snapshot.status = PlaybackStatus::Loading;
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        let generation = self.generation;
        let (abort, registration) = AbortHandle::new_pair();
        #[cfg(target_os = "windows")]
        let _ = registration;
        let task = cx.spawn(async move |this, cx| {
            // The Windows worker validates each redirect on the actual GET.
            // A separate preflight would duplicate requests for signed URLs.
            #[cfg(target_os = "windows")]
            let result = Ok::<_, ()>(Ok::<_, stonks_core::media_playback::PlaybackError>(source));
            #[cfg(not(target_os = "windows"))]
            let result = spawn_core(Abortable::new(
                stonks_core::media_playback::resolve_source(source),
                registration,
            ))
            .await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(mut source)) => {
                        source.autoplay = this.want_playing;
                        if this.backend.is_none() {
                            match Player::new() {
                                Ok(player) => this.backend = Some(player),
                                Err(error) => {
                                    this.fail(error.to_string(), cx);
                                    return;
                                }
                            }
                        }
                        let Some(player) = &this.backend else {
                            return;
                        };
                        if let Err(error) =
                            player.load_with_volume(source.clone(), this.snapshot.volume)
                        {
                            this.fail(error.to_string(), cx);
                            return;
                        }
                        if let Some((position, _)) = this.pending_seek {
                            player.seek(&source.key, position);
                        }
                        this.start_poll(cx);
                    }
                    Ok(Err(error)) => this.fail(error.to_string(), cx),
                    Err(_) => {}
                }
            })
            .ok();
        });
        self.request = Some(ResolveRequest { abort, _task: task });
        cx.notify();
    }
    pub fn play(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(key) {
            return;
        }
        if self.snapshot.status == PlaybackStatus::Failed {
            self.want_playing = true;
            self.retry(key, cx);
            return;
        }
        self.want_playing = true;
        if let Some(player) = &self.backend {
            player.play(key);
        }
        cx.notify();
    }
    pub fn pause(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(key) {
            return;
        }
        self.want_playing = false;
        if let Some(player) = &self.backend {
            player.pause(key);
        }
        cx.notify();
    }
    pub fn seek(&mut self, key: &str, position: f64, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(key) || !position.is_finite() {
            return;
        }
        let position = if self.snapshot.duration > 0. {
            position.clamp(0., self.snapshot.duration)
        } else {
            position.max(0.)
        };
        self.pending_seek = Some((position, Instant::now()));
        self.snapshot.position = position;
        if let Some(player) = &self.backend {
            player.seek(key, position);
        }
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        cx.notify();
    }
    pub fn set_volume(&mut self, key: &str, volume: f64, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(key) || !volume.is_finite() {
            return;
        }
        self.snapshot.volume = volume.clamp(0., 1.);
        if let Some(player) = &self.backend {
            player.set_volume(key, self.snapshot.volume);
        }
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        cx.notify();
    }
    pub fn retry(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(key) {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let volume = self.snapshot.volume;
        let playing = self.want_playing;
        self.load_with_headers(
            source.key,
            source.url,
            source.kind,
            playing,
            source.headers,
            cx,
        );
        self.snapshot.volume = volume;
    }
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.request = None;
        self.poll = None;
        self.active = None;
        self.source = None;
        self.want_playing = false;
        self.pending_seek = None;
        self.backend_revision = 0;
        self.frame_serial = 0;
        let revision = self.snapshot.revision.wrapping_add(1);
        self.snapshot = PlaybackSnapshot {
            revision,
            ..Default::default()
        };
        if let Some(player) = &self.backend {
            player.stop();
        }
        cx.notify();
    }
    fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.snapshot.status = PlaybackStatus::Failed;
        self.snapshot.error = Some(error);
        self.snapshot.frame = None;
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        cx.notify();
    }
    fn start_poll(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let keep = this
                .update(cx, |this, cx| {
                    if this.generation != generation || this.active.is_none() {
                        return false;
                    }
                    let Some(player) = &this.backend else {
                        return false;
                    };
                    let snapshot = player.snapshot();
                    if snapshot.key.as_deref() != this.active.as_deref()
                        || snapshot.revision == this.backend_revision
                    {
                        return true;
                    }
                    this.backend_revision = snapshot.revision;
                    this.snapshot.status = snapshot.status;
                    this.snapshot.duration = snapshot.duration;
                    this.snapshot.volume = snapshot.volume;
                    this.snapshot.error = snapshot.error.map(|error| error.to_string());
                    if let Some((target, at)) = this.pending_seek {
                        if (snapshot.position - target).abs() < 0.2
                            || at.elapsed() > Duration::from_secs(2)
                            || snapshot.status == PlaybackStatus::Failed
                        {
                            this.pending_seek = None;
                        }
                    }
                    if this.pending_seek.is_none() {
                        this.snapshot.position = snapshot.position;
                    }
                    if let Some(frame) = snapshot.frame {
                        if frame.serial != this.frame_serial {
                            this.frame_serial = frame.serial;
                            if let Some(pixels) = image::RgbaImage::from_raw(
                                frame.width,
                                frame.height,
                                frame.bgra.to_vec(),
                            ) {
                                this.snapshot.frame =
                                    Some(Arc::new(RenderImage::new(vec![image::Frame::new(
                                        pixels,
                                    )])));
                            }
                        }
                    }
                    if snapshot.status == PlaybackStatus::Ended {
                        this.want_playing = false;
                    }
                    this.snapshot.revision = this.snapshot.revision.wrapping_add(1);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }
}
