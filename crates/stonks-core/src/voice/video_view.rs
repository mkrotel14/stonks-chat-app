use std::sync::Arc;

use futures_util::StreamExt;
use livekit::{
    prelude::*,
    webrtc::{
        prelude::{VideoFormatType, VideoRotation},
        video_frame::native::VideoFrameBufferExt,
        video_stream::native::NativeVideoStream,
    },
};
use tokio::{sync::watch, task::JoinHandle};

/// Latest decoded remote video frame. Pixels are tightly packed RGBA8, top row first.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub participant_id: String,
    pub track_sid: String,
    pub width: u32,
    pub height: u32,
    pub rotation_degrees: u16,
    pub rgba: Arc<[u8]>,
}

pub(super) type FrameSender = watch::Sender<Option<Arc<VideoFrame>>>;

pub(super) struct Viewing {
    pub target: Option<String>,
    state: FrameSender,
    source: TrackSource,
    task: Option<(TrackSid, JoinHandle<()>)>,
}

impl Viewing {
    pub fn new(state: FrameSender, source: TrackSource) -> Self {
        Self {
            target: None,
            state,
            source,
            task: None,
        }
    }

    pub async fn reconcile(&mut self, room: &Room, connected: bool) {
        let mut next = None;
        for participant in room.remote_participants().into_values() {
            let id = participant.identity().to_string();
            let mut videos: Vec<_> = participant
                .track_publications()
                .into_values()
                .filter(|p| p.source() == self.source && p.kind() == TrackKind::Video)
                .collect();
            videos.sort_by_key(|p| p.sid().to_string());
            for (index, p) in videos.into_iter().enumerate() {
                let wanted = connected && index == 0 && self.target.as_deref() == Some(id.as_str());
                if p.is_desired() != wanted {
                    p.set_subscribed(wanted);
                }
                if wanted && !p.is_muted() {
                    if let Some(RemoteTrack::Video(track)) = p.track() {
                        next = Some((id.clone(), p.sid(), track));
                    }
                }
            }
        }
        if let Some((sid, job)) = &self.task {
            if !job.is_finished()
                && next
                    .as_ref()
                    .is_some_and(|(_, next_sid, _)| sid == next_sid)
            {
                return;
            }
        }
        self.stop().await;
        if let Some((id, sid, track)) = next {
            let track_sid = sid.to_string();
            let state = self.state.clone();
            self.task = Some((
                sid,
                tokio::spawn(async move {
                    let mut stream = NativeVideoStream::new(track.rtc_track());
                    while let Some(frame) = stream.next().await {
                        let (width, height) = (frame.buffer.width(), frame.buffer.height());
                        if !(2..=8192).contains(&width) || !(2..=8192).contains(&height) {
                            continue;
                        }
                        let scale = (1920.0 / f64::from(width))
                            .min(1080.0 / f64::from(height))
                            .min(1.0);
                        let w = ((f64::from(width) * scale) as u32).max(2) & !1;
                        let h = ((f64::from(height) * scale) as u32).max(2) & !1;
                        let mut i420 = frame.buffer.to_i420();
                        let i420 = if (w, h) == (width, height) {
                            i420
                        } else {
                            i420.scale(w as i32, h as i32)
                        };
                        let mut rgba = vec![0; (w * h * 4) as usize];
                        i420.to_argb(VideoFormatType::ABGR, &mut rgba, w * 4, w as i32, h as i32);
                        let rotation_degrees = match frame.rotation {
                            VideoRotation::VideoRotation0 => 0,
                            VideoRotation::VideoRotation90 => 90,
                            VideoRotation::VideoRotation180 => 180,
                            VideoRotation::VideoRotation270 => 270,
                        };
                        state.send_replace(Some(Arc::new(VideoFrame {
                            participant_id: id.clone(),
                            track_sid: track_sid.clone(),
                            width: w,
                            height: h,
                            rotation_degrees,
                            rgba: rgba.into(),
                        })));
                    }
                    state.send_replace(None);
                }),
            ));
        }
    }

    pub async fn stop(&mut self) {
        if let Some((_, task)) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        if self.state.borrow().is_some() {
            self.state.send_replace(None);
        }
    }
}

impl Drop for Viewing {
    fn drop(&mut self) {
        if let Some((_, task)) = self.task.take() {
            task.abort();
        }
    }
}
