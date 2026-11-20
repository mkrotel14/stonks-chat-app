//! Bridge the call worker's authoritative state and operations to the native card.
use super::{MusicActions, MusicCard, MusicCommand, MusicQueueItem, MusicSeek, MusicSnapshot};
use crate::{features::voice::Voice, shared::images::Images};
use gpui_kit::*;
use std::{
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use stonks_core::{
    connection::AppSnapshot,
    voice::{MusicSnapshot as Playback, VoicePhase, VoiceSession},
};
use stonks_ui::{i18n::t, Theme};
use tokio::sync::{mpsc, oneshot, watch};

pub(in crate::features::voice) enum Operation {
    Add(String),
    Control(MusicCommand),
    Seek(MusicSeek),
}

pub(in crate::features::voice) struct Job {
    operation: Operation,
    result: oneshot::Sender<Result<(), String>>,
}

type ResultReceiver = oneshot::Receiver<Result<(), String>>;

pub(in crate::features::voice) struct Music {
    pub id: u64,
    pub jobs: mpsc::Sender<Job>,
    pub playback: Playback,
    busy: bool,
    request_error: Option<String>,
    control_error: Option<String>,
    card: Option<(Entity<MusicCard>, Subscription)>,
}

impl Music {
    pub fn new(jobs: mpsc::Sender<Job>) -> Self {
        Self {
            id: next_call(),
            jobs,
            playback: Playback::default(),
            busy: false,
            request_error: None,
            control_error: None,
            card: None,
        }
    }

    pub fn reset_card(&mut self) {
        self.id = next_call();
        self.busy = false;
        self.card = None;
        self.request_error = None;
        self.control_error = None;
    }
}

impl Voice {
    pub(crate) fn music_call_is_current(&self, id: u64) -> bool {
        self.call.as_ref().is_some_and(|call| call.music.id == id)
            && self
                .roster_state()
                .is_some_and(|state| state.phase == VoicePhase::Connected)
    }

    pub(crate) fn request_music(
        &mut self,
        url: String,
        cx: &mut Context<Self>,
    ) -> Result<(u64, ResultReceiver), String> {
        let id = self
            .call
            .as_ref()
            .map(|call| call.music.id)
            .ok_or_else(unavailable)?;
        self.start_music(id, Operation::Add(url), cx)
            .map(|result| (id, result))
    }

    fn start_music(
        &mut self,
        id: u64,
        operation: Operation,
        cx: &mut Context<Self>,
    ) -> Result<ResultReceiver, String> {
        if !self.music_call_is_current(id) {
            return Err(unavailable());
        }
        let music = &mut self.call.as_mut().unwrap().music;
        if music.busy {
            return Err(t("messages.musicQueueBusy"));
        }
        let added = match &operation {
            Operation::Add(url) => Some(url.clone()),
            _ => None,
        };
        let (result, done) = oneshot::channel();
        music
            .jobs
            .try_send(Job { operation, result })
            .map_err(|_| unavailable())?;
        music.busy = true;
        music.request_error = None;
        music.control_error = None;
        cx.notify();
        let (answer, receiver) = oneshot::channel();
        cx.spawn(async move |this, cx| {
            let result = done.await.unwrap_or_else(|_| Err(unavailable()));
            let _ = this.update(cx, |this, cx| {
                let current = this.music_call_is_current(id);
                if let Some(call) = this.call.as_mut().filter(|call| call.music.id == id) {
                    let music = &mut call.music;
                    music.busy = false;
                    if current {
                        match (&added, &result) {
                            (Some(_), Err(error)) => music.request_error = Some(error.clone()),
                            (None, Err(error)) => music.control_error = Some(error.clone()),
                            _ => {}
                        }
                    }
                    cx.notify();
                }
            });
            let _ = answer.send(result);
        })
        .detach();
        Ok(receiver)
    }

    pub(in crate::features::voice) fn music_card(
        &mut self,
        users: Option<&AppSnapshot>,
        images: &Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MusicCard>> {
        const EDGE: u32 = 128;
        if !self
            .roster_state()
            .is_some_and(|state| state.phase == VoicePhase::Connected)
        {
            images.update(cx, |images, _| images.set_sources("voice-music", []));
            return None;
        }
        let call = self.call.as_mut()?;
        let server = users.and_then(|users| {
            users
                .channels
                .iter()
                .find(|channel| channel.id == call.id)?
                .server
                .as_deref()
        });
        let music = &mut call.music;
        let id = music.id;
        let card = music
            .card
            .get_or_insert_with(|| {
                let weak = cx.weak_entity();
                let control = action(weak.clone(), id, Operation::Control);
                let seek = action(weak.clone(), id, Operation::Seek);
                let volume = Rc::new(move |gain, _: &mut Window, cx: &mut App| {
                    let _ = weak.update(cx, |voice, cx| {
                        if voice.music_call_is_current(id) {
                            voice.set_music_gain(gain, cx);
                        }
                    });
                });
                let weak = cx.weak_entity();
                let request = Rc::new(move |url: String, window: &mut Window, cx: &mut App| {
                    window.defer(cx, {
                        let weak = weak.clone();
                        move |window, cx| {
                            let _ = weak.update(cx, |voice, cx| {
                                let Ok(result) =
                                    voice.start_music(id, Operation::Add(url.clone()), cx)
                                else {
                                    return;
                                };
                                cx.spawn_in(window, async move |this, cx| {
                                    if !matches!(result.await, Ok(Ok(()))) {
                                        return;
                                    }
                                    let _ = this.update_in(cx, |this, window, cx| {
                                        if this.music_call_is_current(id) {
                                            if let Some((card, _)) = this
                                                .call
                                                .as_ref()
                                                .and_then(|call| call.music.card.as_ref())
                                            {
                                                card.update(cx, |card, cx| {
                                                    card.request_succeeded(&url, window, cx)
                                                });
                                            }
                                        }
                                    });
                                })
                                .detach();
                            });
                        }
                    });
                });
                let gain = self.listening.gains.music_listening_gain();
                let actions = MusicActions {
                    control: Some(control),
                    seek: Some(seek),
                    volume: Some(volume),
                    request: Some(request),
                };
                let card = cx.new(|cx| MusicCard::new(gain, actions, window, cx));
                let subscription = cx.observe(&card, |_, _, cx| cx.notify());
                (card, subscription)
            })
            .0
            .clone();
        let playback = &music.playback;
        let requester = |id: Option<&str>| {
            let (users, id) = (users?, id?);
            server
                .and_then(|server| {
                    users
                        .members
                        .iter()
                        .find(|member| member.id.server == server && member.id.user == id)
                        .and_then(|member| member.nickname.clone())
                })
                .or_else(|| {
                    std::iter::once(&users.current_user)
                        .chain(&users.users)
                        .find(|user| user.id == id)
                        .map(|user| {
                            user.display_name
                                .clone()
                                .unwrap_or_else(|| user.username.clone())
                        })
                })
                .or_else(|| Some(id.to_owned()))
        };
        let (artwork, queue) = images.update(cx, |images, cx| {
            images.set_sources(
                "voice-music",
                playback
                    .artwork_url
                    .iter()
                    .chain(
                        playback
                            .queue
                            .iter()
                            .filter_map(|item| item.artwork_url.as_ref()),
                    )
                    .map(|url| (url.clone(), EDGE)),
            );
            let artwork = images.get(playback.artwork_url.as_deref(), EDGE, cx);
            let queue = playback
                .queue
                .iter()
                .map(|item| MusicQueueItem {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    source_url: item.source_url.clone(),
                    requester_name: requester(item.requested_by.as_deref()),
                    artwork: images.get(item.artwork_url.as_deref(), EDGE, cx),
                })
                .collect();
            (artwork, queue)
        });
        card.update(cx, |card, cx| {
            card.set_snapshot(
                MusicSnapshot {
                    active: playback.active,
                    loading: playback.loading,
                    paused: playback.paused,
                    seeking: playback.seeking,
                    busy: music.busy,
                    error: music
                        .control_error
                        .clone()
                        .or_else(|| (!playback.error.is_empty()).then(|| playback.error.clone())),
                    request_error: music.request_error.clone(),
                    current_id: playback.current_item_id.clone(),
                    revision: playback.revision,
                    playback_epoch: playback.playback_epoch,
                    title: playback.title.clone(),
                    artist: playback.artist.clone(),
                    requester_name: requester(playback.requested_by.as_deref()),
                    artwork,
                    queue,
                    elapsed_seconds: seconds(playback.elapsed_seconds),
                    duration_seconds: playback.duration_seconds.map(seconds),
                },
                window,
                cx,
            )
        });
        Some(card)
    }

    pub(crate) fn music_playlist(&self, theme: Theme, cx: &App) -> Option<AnyElement> {
        let call = self.call.as_ref()?;
        if !self.music_call_is_current(call.music.id) {
            return None;
        }
        let (card, _) = call.music.card.as_ref()?;
        card.read(cx).render_playlist(card, theme, cx)
    }

    pub(crate) fn dismiss_music_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((card, _)) = self.call.as_ref().and_then(|call| call.music.card.as_ref()) {
            card.update(cx, |card, cx| card.close_playlist(window, cx));
        }
    }

    pub(crate) fn music_playlist_open(&self, cx: &App) -> bool {
        self.call
            .as_ref()
            .filter(|call| self.music_call_is_current(call.music.id))
            .and_then(|call| call.music.card.as_ref())
            .is_some_and(|(card, _)| card.read(cx).playlist_open)
    }
}

fn next_call() -> u64 {
    static NEXT_CALL: AtomicU64 = AtomicU64::new(0);
    NEXT_CALL.fetch_add(1, Ordering::Relaxed)
}

fn unavailable() -> String {
    t("messages.musicQueueUnavailable")
}
fn seconds(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

fn action<T: 'static>(
    weak: WeakEntity<Voice>,
    id: u64,
    operation: fn(T) -> Operation,
) -> Rc<dyn Fn(T, &mut Window, &mut App)> {
    Rc::new(move |value, _, cx| {
        let weak = weak.clone();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |voice, cx| voice.start_music(id, operation(value), cx));
        });
    })
}

pub(in crate::features::voice) async fn run(
    room: &VoiceSession,
    mut jobs: mpsc::Receiver<Job>,
    updates: watch::Sender<Playback>,
) {
    let mut playback = room.subscribe_music();
    let publish = async {
        loop {
            updates.send_replace(playback.borrow_and_update().clone());
            if playback.changed().await.is_err() {
                break;
            }
        }
    };
    let operate = async {
        while let Some(job) = jobs.recv().await {
            let result = if room.snapshot().phase == VoicePhase::Connected {
                execute(room, job.operation).await
            } else {
                Err(unavailable())
            };
            let _ = job.result.send(result);
        }
    };
    tokio::select! { _ = publish => {}, _ = operate => {} }
}

async fn execute(room: &VoiceSession, operation: Operation) -> Result<(), String> {
    match operation {
        Operation::Add(url) => room.request_music(url).await,
        Operation::Control(command) => {
            let (action, id) = match command {
                MusicCommand::Pause => ("pause", None),
                MusicCommand::Resume => ("resume", None),
                MusicCommand::Skip => ("skip", None),
                MusicCommand::Stop => ("stop", None),
                MusicCommand::Remove(id) => ("remove", Some(id)),
            };
            room.control_music(action, id, None, None, None, None).await
        }
        Operation::Seek(seek) => {
            static NEXT_SEEK: AtomicU64 = AtomicU64::new(0);
            let nonce = format!(
                "gpui-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT_SEEK.fetch_add(1, Ordering::Relaxed)
            );
            room.control_music(
                "seek",
                Some(seek.item_id),
                Some(seek.seconds.into()),
                Some(seek.revision),
                Some(seek.playback_epoch),
                Some(nonce),
            )
            .await
        }
    }
}
