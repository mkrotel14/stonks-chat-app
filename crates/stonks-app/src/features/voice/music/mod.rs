//! Native port of Tauri's MusicCard. The voice parent owns playback and persistence.
pub(super) mod session;
mod view;

use gpui_kit::{
    component::{
        input::{InputEvent, InputState},
        slider::{SliderEvent, SliderState, SliderValue},
    },
    *,
};
use std::{rc::Rc, sync::Arc};
use stonks_ui::i18n::t;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum MusicLayout {
    #[default]
    Standard,
    Horizontal,
    Compact,
    Inline,
}

#[derive(Default, PartialEq)]
pub(crate) struct MusicSnapshot {
    pub active: bool,
    pub loading: bool,
    pub paused: bool,
    pub seeking: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub request_error: Option<String>,
    pub current_id: Option<String>,
    pub revision: u64,
    pub playback_epoch: u64,
    pub title: String,
    pub artist: Option<String>,
    pub requester_name: Option<String>,
    pub artwork: Option<Arc<RenderImage>>,
    pub elapsed_seconds: u32,
    pub duration_seconds: Option<u32>,
    pub queue: Vec<MusicQueueItem>,
}

#[derive(PartialEq)]
pub(crate) struct MusicQueueItem {
    pub id: String,
    pub title: String,
    pub requester_name: Option<String>,
    pub source_url: String,
    pub artwork: Option<Arc<RenderImage>>,
}

#[derive(Clone)]
pub(crate) enum MusicCommand {
    Pause,
    Resume,
    Skip,
    Stop,
    Remove(String),
}

pub(crate) struct MusicSeek {
    pub item_id: String,
    pub seconds: u32,
    pub revision: u64,
    pub playback_epoch: u64,
}

type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// Only supply handlers backed by real operations. The volume handler should persist
/// the local listening preference; the parent supplies its initial value (Tauri: 70).
#[derive(Default)]
pub(crate) struct MusicActions {
    pub control: Option<Handler<MusicCommand>>,
    pub seek: Option<Handler<MusicSeek>>,
    pub volume: Option<Handler<u8>>,
    pub request: Option<Handler<String>>,
}

pub(crate) struct MusicCard {
    snapshot: MusicSnapshot,
    actions: MusicActions,
    seek: Entity<SliderState>,
    volume: Entity<SliderState>,
    url: Entity<InputState>,
    seek_subscription: Subscription,
    _subscriptions: Vec<Subscription>,
    preview: Option<u32>,
    seek_committed: bool,
    volume_open: bool,
    playlist_open: bool,
    page: usize,
    dialog_focus: FocusHandle,
    opener: Option<FocusHandle>,
}

const PAGE_SIZE: usize = 5;

impl MusicCard {
    pub(crate) fn new(
        volume_percent: u8,
        actions: MusicActions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let seek = cx.new(|_| SliderState::new().min(0.).max(1.).step(1.));
        let seek_subscription = Self::subscribe_seek(&seek, window, cx);
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(volume_percent.min(100)))
        });
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(t("voice.pasteMusicLink")));
        let subscriptions = vec![
            cx.subscribe_in(&volume, window, |this, _, event, window, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    if this.volume_open && this.snapshot.active {
                        if let Some(handler) = this.actions.volume.clone() {
                            handler(value.round().clamp(0., 100.) as u8, window, cx);
                        }
                        cx.notify();
                    }
                }
            }),
            cx.subscribe_in(&url, window, |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.request(window, cx),
                _ => {}
            }),
        ];
        Self {
            snapshot: MusicSnapshot::default(),
            actions,
            seek,
            volume,
            url,
            seek_subscription,
            _subscriptions: subscriptions,
            preview: None,
            seek_committed: false,
            volume_open: false,
            playlist_open: false,
            page: 0,
            dialog_focus: cx.focus_handle(),
            opener: None,
        }
    }

    pub(super) fn visible(&self) -> bool {
        self.snapshot.active || self.snapshot.error.is_some()
    }

    /// Call on each authoritative playback update. Keep one entity per voice session.
    pub(crate) fn set_snapshot(
        &mut self,
        next: MusicSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if next == self.snapshot {
            return;
        }
        let changed = next.current_id != self.snapshot.current_id
            || next.playback_epoch != self.snapshot.playback_epoch;
        let max = seek_max(next.duration_seconds).unwrap_or(1);
        if changed || next.duration_seconds != self.snapshot.duration_seconds {
            self.preview = None;
            self.seek_committed = false;
            let seek = cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(max as f32)
                    .step(1.)
                    .default_value(next.elapsed_seconds.min(max) as f32)
            });
            self.seek_subscription = Self::subscribe_seek(&seek, window, cx);
            self.seek = seek;
        } else if self.preview.is_none() || (!next.busy && !next.seeking && self.seek_committed) {
            self.preview = None;
            self.seek_committed = false;
            self.seek.update(cx, |seek, cx| {
                seek.set_value(next.elapsed_seconds.min(max) as f32, window, cx)
            });
        }
        self.page = self.page.min(page_count(next.queue.len()) - 1);
        if !next.active {
            self.volume_open = false;
        }
        if !next.active && next.error.is_none() && self.playlist_open {
            self.close_playlist(window, cx);
        }
        self.snapshot = next;
        cx.notify();
    }

    /// Call only on successful completion; do not erase a newer edit.
    pub(crate) fn request_succeeded(
        &mut self,
        submitted_url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.playlist_open && self.url.read(cx).value().trim() == submitted_url {
            self.url
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn seek_enabled(&self) -> bool {
        let s = &self.snapshot;
        s.active
            && !s.loading
            && !s.seeking
            && !s.busy
            && s.current_id.is_some()
            && seek_max(s.duration_seconds).is_some()
            && self.actions.seek.is_some()
    }

    fn subscribe_seek(
        seek: &Entity<SliderState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(seek, window, |this, _, event, window, cx| {
            if !this.seek_enabled() {
                return;
            }
            match event {
                SliderEvent::Change(SliderValue::Single(value)) => {
                    this.preview = Some(value.round() as u32);
                    cx.notify();
                }
                SliderEvent::Release(SliderValue::Single(value)) => {
                    this.commit_seek(value.round() as u32, window, cx)
                }
                _ => {}
            }
        })
    }

    fn commit_seek(&mut self, seconds: u32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.seek_enabled() {
            return;
        }
        let (Some(item_id), Some(max), Some(handler)) = (
            self.snapshot.current_id.clone(),
            seek_max(self.snapshot.duration_seconds),
            self.actions.seek.clone(),
        ) else {
            return;
        };
        let seconds = seconds.min(max);
        self.preview = Some(seconds);
        self.seek_committed = true;
        handler(
            MusicSeek {
                item_id,
                seconds,
                revision: self.snapshot.revision,
                playback_epoch: self.snapshot.playback_epoch,
            },
            window,
            cx,
        );
        cx.notify();
    }

    fn request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.playlist_open && !self.snapshot.busy {
            let url = self.url.read(cx).value();
            if let (Some(handler), Some(url)) = (self.actions.request.clone(), request_url(&url)) {
                handler(url.to_owned(), window, cx);
            }
        }
    }

    fn command_enabled(&self, command: &MusicCommand) -> bool {
        if self.snapshot.busy || self.actions.control.is_none() {
            return false;
        }
        match command {
            MusicCommand::Pause | MusicCommand::Resume | MusicCommand::Skip => {
                self.snapshot.active && !self.snapshot.loading
            }
            MusicCommand::Remove(id) => {
                self.playlist_open && self.snapshot.queue.iter().any(|item| &item.id == id)
            }
            MusicCommand::Stop => self.playlist_open,
        }
    }

    fn close_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.playlist_open = false;
        if let Some(opener) = self.opener.take() {
            opener.focus(window, cx);
        }
        cx.notify();
    }
}

fn seek_max(duration: Option<u32>) -> Option<u32> {
    duration
        .filter(|duration| *duration > 1)
        .map(|duration| duration - 1)
}

fn page_count(items: usize) -> usize {
    items.div_ceil(PAGE_SIZE).max(1)
}

fn request_url(value: &str) -> Option<&str> {
    let value = value.trim();
    url::Url::parse(value).ok().map(|_| value)
}

fn music_time(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::{music_time, page_count, request_url, seek_max};

    #[test]
    fn queue_has_five_items_per_page_and_an_empty_page() {
        assert_eq!([0, 1, 5, 6, 10, 11].map(page_count), [1, 1, 1, 2, 2, 3]);
    }

    #[test]
    fn seek_excludes_end_and_short_tracks_and_time_formats_minutes() {
        assert_eq!(seek_max(None), None);
        assert_eq!(seek_max(Some(1)), None);
        assert_eq!(seek_max(Some(61)), Some(60));
        assert_eq!(music_time(65), "1:05");
    }

    #[test]
    fn request_requires_an_absolute_url_and_preserves_trimmed_input() {
        assert_eq!(request_url(" song "), None);
        assert_eq!(
            request_url(" https://example.com/song "),
            Some("https://example.com/song")
        );
    }
}
