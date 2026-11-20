mod viewer;

use super::{Voice, VoicePhase};
use gpui_kit::*;
use stonks_core::voice::{VideoFrame, VoiceSession};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Screen,
    Camera,
}

pub(super) struct View {
    participant: String,
    kind: Kind,
    feed: viewer::FrameFeed,
    audio_present: watch::Sender<bool>,
}

pub(super) struct OpenViewer {
    participant: String,
    kind: Kind,
    generation: u64,
    handle: AnyWindowHandle,
}

impl Voice {
    pub(super) fn can_view_video(&self, channel: &str) -> bool {
        self.roster_state()
            .is_some_and(|s| s.channel_id == channel && s.phase == VoicePhase::Connected)
    }

    pub(super) fn view_video(
        &mut self,
        kind: Kind,
        channel: &str,
        participant: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_view_video(channel) {
            return;
        }
        if let Some(viewer) = self
            .viewer
            .as_ref()
            .filter(|v| v.participant == participant && v.kind == kind)
        {
            if viewer
                .handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
        }
        self.viewer_generation += 1;
        let generation = self.viewer_generation;
        let voice = cx.weak_entity();
        let display = window.display(cx).map(|display| display.id());
        let (audio_present, audio_rx) = watch::channel(false);
        let audio = (kind == Kind::Screen).then(|| {
            let voice = voice.clone();
            let id = participant.to_owned();
            viewer::AudioControls::new(
                audio_rx,
                self.listening.gains.screen_listening_gain(participant),
                move |gain, cx| {
                    let _ = voice.update(cx, |voice, cx| voice.set_screen_gain(&id, gain, cx));
                },
            )
        });
        let opened = viewer::open(
            name.to_owned().into(),
            display,
            audio,
            move |cx| {
                let _ = voice.update(cx, |voice, cx| {
                    if voice
                        .viewer
                        .as_ref()
                        .is_some_and(|v| v.generation == generation)
                    {
                        voice.viewer = None;
                        cx.notify();
                    }
                });
            },
            cx,
        );
        let (handle, feed) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                eprintln!("voice video viewer: {error}");
                return;
            }
        };
        self.close_viewer(cx);
        self.viewer = Some(OpenViewer {
            participant: participant.into(),
            kind,
            generation,
            handle,
        });
        if let Some(call) = &self.call {
            let _ = call.video.send(View {
                participant: participant.into(),
                kind,
                feed,
                audio_present,
            });
        }
    }

    pub(super) fn close_viewer(&mut self, cx: &mut Context<Self>) {
        if let Some(viewer) = self.viewer.take() {
            let _ = viewer
                .handle
                .update(cx, |_, window, _| window.remove_window());
        }
    }
}

pub(super) async fn run(room: &VoiceSession, mut commands: mpsc::UnboundedReceiver<View>) {
    let mut screen = room.subscribe_screen_frames();
    let mut camera = room.subscribe_camera_frames();
    let mut audio = room.subscribe_viewed_screen_audio();
    let mut active: Option<View> = None;
    loop {
        tokio::select! {
            biased;
            request = commands.recv() => {
                let Some(request) = request else { break };
                if let Some(previous) = active.take().filter(|prev| prev.kind != request.kind) {
                    select(room, previous.kind, None).await;
                }
                if select(room, request.kind, Some(request.participant.clone())).await {
                    // Reconcile clears the previous track before acknowledging a switch.
                    update_audio(&request, audio.borrow_and_update().as_deref());
                    active = Some(request);
                } else {
                    active = None;
                    select(room, request.kind, None).await;
                }
            }
            _ = async { if let Some(viewer) = &active { viewer.feed.closed().await } }, if active.is_some() => {
                if let Some(viewer) = active.take() { select(room, viewer.kind, None).await; }
            }
            changed = audio.changed() => {
                if changed.is_err() { break; }
                if let Some(view) = &active {
                    update_audio(view, audio.borrow_and_update().as_deref());
                } else {
                    audio.borrow_and_update();
                }
            }
            changed = screen.changed() => {
                if changed.is_err() { break; }
                deliver(active.as_ref(), Kind::Screen, screen.borrow_and_update().clone());
            }
            changed = camera.changed() => {
                if changed.is_err() { break; }
                deliver(active.as_ref(), Kind::Camera, camera.borrow_and_update().clone());
            }
        }
    }
    if let Some(viewer) = active {
        select(room, viewer.kind, None).await;
    }
}

fn update_audio(view: &View, publisher: Option<&str>) {
    view.audio_present
        .send_replace(view.kind == Kind::Screen && publisher == Some(view.participant.as_str()));
}

async fn select(room: &VoiceSession, kind: Kind, target: Option<String>) -> bool {
    match kind {
        Kind::Screen => room.set_screen_view(target).await.is_ok(),
        Kind::Camera => room.set_camera_view(target).await.is_ok(),
    }
}

fn deliver(active: Option<&View>, kind: Kind, frame: Option<std::sync::Arc<VideoFrame>>) {
    if let Some(viewer) = active.filter(|viewer| viewer.kind == kind) {
        match frame
            .filter(|frame| frame.participant_id == viewer.participant)
            .as_deref()
        {
            Some(frame) => {
                if let Some((width, height, bgra)) = to_bgra(frame) {
                    let _ = viewer.feed.push_bgra(width, height, bgra);
                }
            }
            None => viewer.feed.clear(),
        }
    }
}

fn to_bgra(frame: &VideoFrame) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h)?.checked_mul(4)? != frame.rgba.len() {
        return None;
    }
    let (out_w, out_h) = match frame.rotation_degrees {
        0 | 180 => (w, h),
        90 | 270 => (h, w),
        _ => return None,
    };
    let mut out = vec![0; frame.rgba.len()];
    for (i, p) in frame.rgba.chunks_exact(4).enumerate() {
        let (x, y) = (i % w, i / w);
        let (x, y) = match frame.rotation_degrees {
            90 => (h - 1 - y, x),
            180 => (w - 1 - x, h - 1 - y),
            270 => (y, w - 1 - x),
            _ => (x, y),
        };
        let offset = (y * out_w + x) * 4;
        out[offset..offset + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    Some((out_w as u32, out_h as u32, out))
}

#[cfg(test)]
mod tests {
    use super::{to_bgra, VideoFrame};

    #[test]
    fn camera_view_ignores_screen_frames_and_clears_on_camera_loss() {
        use super::{deliver, viewer::FrameFeed, Kind, View};
        let (feed, rx) = FrameFeed::channel();
        let viewer = View {
            participant: "peer".into(),
            kind: Kind::Camera,
            feed,
            audio_present: tokio::sync::watch::channel(false).0,
        };
        let frame = std::sync::Arc::new(VideoFrame {
            participant_id: "peer".into(),
            track_sid: "track".into(),
            width: 1,
            height: 1,
            rotation_degrees: 0,
            rgba: vec![1, 2, 3, 255].into(),
        });
        deliver(Some(&viewer), Kind::Screen, Some(frame.clone()));
        assert!(rx.borrow().is_none());
        deliver(Some(&viewer), Kind::Camera, Some(frame));
        assert!(rx.borrow().is_some());
        deliver(Some(&viewer), Kind::Screen, None);
        assert!(rx.borrow().is_some());
        deliver(Some(&viewer), Kind::Camera, None);
        assert!(rx.borrow().is_none());
    }

    #[test]
    fn audio_controls_follow_only_the_matching_remote_screen_track() {
        use super::{update_audio, viewer::FrameFeed, Kind, View};
        let (feed, _frames) = FrameFeed::channel();
        let (audio_present, state) = tokio::sync::watch::channel(false);
        let mut view = View {
            participant: "peer".into(),
            kind: Kind::Screen,
            feed,
            audio_present,
        };
        for (publisher, expected) in [(Some("other"), false), (Some("peer"), true), (None, false)] {
            update_audio(&view, publisher);
            assert_eq!(*state.borrow(), expected);
        }
        view.kind = Kind::Camera;
        update_audio(&view, Some("peer"));
        assert!(!*state.borrow());
    }

    #[test]
    fn rotates_clockwise_and_swizzles_as_bgra() {
        let mut frame = VideoFrame {
            participant_id: "peer".into(),
            track_sid: "track".into(),
            width: 3,
            height: 2,
            rotation_degrees: 0,
            rgba: (1..=6)
                .flat_map(|r| [r, 10, 20, 255])
                .collect::<Vec<_>>()
                .into(),
        };
        for (rotation, w, h, expected) in [
            (0, 3, 2, [1, 2, 3, 4, 5, 6]),
            (90, 2, 3, [4, 1, 5, 2, 6, 3]),
            (180, 3, 2, [6, 5, 4, 3, 2, 1]),
            (270, 2, 3, [3, 6, 2, 5, 1, 4]),
        ] {
            frame.rotation_degrees = rotation;
            let expected = expected
                .into_iter()
                .flat_map(|r| [20, 10, r, 255])
                .collect();
            assert_eq!(to_bgra(&frame), Some((w, h, expected)));
        }
        frame.rotation_degrees = 45;
        assert!(to_bgra(&frame).is_none());
        frame.rotation_degrees = 0;
        frame.width = 1;
        assert!(to_bgra(&frame).is_none());
    }
}
