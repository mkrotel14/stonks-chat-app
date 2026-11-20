//! Explicit, room-owned camera publication. No audio or desktop capture.
mod capture;
pub(super) mod devices;
#[cfg(any(windows, test))]
mod media_foundation;
mod preview;
pub use devices::{devices, resolve_device, CameraDevice};

pub use preview::{run_preview, PreviewFrame};

use livekit::prelude::Room;
use livekit_protocol::{ParticipantPermission, TrackSource};
use std::{fmt, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CameraQuality {
    #[default]
    Hd30,
    FullHd30,
    FullHd60,
}
impl CameraQuality {
    pub const ALL: [Self; 3] = [Self::Hd30, Self::FullHd30, Self::FullHd60];
    pub fn label(self) -> &'static str {
        match self {
            Self::Hd30 => "720p · 30 FPS",
            Self::FullHd30 => "1080p · 30 FPS",
            Self::FullHd60 => "1080p · 60 FPS",
        }
    }
    fn profile(self) -> (u32, u32, u32, u64) {
        match self {
            Self::Hd30 => (1280, 720, 30, 4_000_000),
            Self::FullHd30 => (1920, 1080, 30, 8_000_000),
            Self::FullHd60 => (1920, 1080, 60, 12_000_000),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CameraOptions {
    /// `default` resolves using platform metadata or the first discovered camera.
    pub device: String,
    pub quality: CameraQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraError {
    Disconnected,
    Busy,
    PublishDenied,
    Discovery,
    DeviceMissing,
    CaptureUnavailable,
    Capture,
    Publish,
    Unpublish,
}
impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disconnected => "Join a connected voice room first.",
            Self::Busy => "The camera is already active or stopping.",
            Self::PublishDenied => "This room does not allow camera video.",
            #[cfg(windows)]
            Self::Discovery => "Could not discover Media Foundation cameras through GStreamer.",
            #[cfg(target_os = "linux")]
            Self::Discovery => "Could not discover cameras through PipeWire.",
            Self::DeviceMissing => "The selected camera is unavailable.",
            #[cfg(target_os = "windows")]
            Self::CaptureUnavailable => {
                "Camera capture requires GStreamer with the Media Foundation plugin."
            }
            #[cfg(target_os = "linux")]
            Self::CaptureUnavailable => "Camera capture requires GStreamer camera plugins.",
            Self::Capture => "Camera capture failed. Check the device or try a different quality.",
            Self::Publish => "Could not publish camera video.",
            Self::Unpublish => {
                "Capture stopped, but the camera track could not be removed from the room."
            }
        })
    }
}
impl std::error::Error for CameraError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraState {
    Idle,
    Starting,
    Publishing,
    Stopping,
    Failed(CameraError),
}

type Reply = oneshot::Sender<Result<(), CameraError>>;
pub(super) enum Command {
    Start(CameraOptions, Reply),
    Stop(Reply),
    View(Option<String>, Reply),
}

pub(super) struct CameraRoom {
    pub inbox: mpsc::Receiver<Command>,
    pub viewing: super::video_view::Viewing,
    pub jobs: JoinSet<Result<(), CameraError>>,
    stop: Option<CancellationToken>,
    stop_reply: Option<Reply>,
    state: watch::Sender<CameraState>,
}

impl CameraRoom {
    pub fn new(
        inbox: mpsc::Receiver<Command>,
        state: watch::Sender<CameraState>,
        frames: super::video_view::FrameSender,
    ) -> Self {
        Self {
            inbox,
            viewing: super::video_view::Viewing::new(frames, livekit::prelude::TrackSource::Camera),
            jobs: JoinSet::new(),
            stop: None,
            stop_reply: None,
            state,
        }
    }
    pub fn start(
        &mut self,
        room: Arc<Room>,
        connected: bool,
        options: CameraOptions,
    ) -> Result<(), CameraError> {
        if !connected {
            return Err(CameraError::Disconnected);
        }
        if self.stop.is_some() {
            return Err(CameraError::Busy);
        }
        if !allows_camera(room.local_participant().permission().as_ref()) {
            return Err(CameraError::PublishDenied);
        }
        let stop = CancellationToken::new();
        self.stop = Some(stop.clone());
        self.state.send_replace(CameraState::Starting);
        self.jobs
            .spawn(capture::run(room, options, stop, self.state.clone()));
        Ok(())
    }
    pub fn stop(&mut self, reply: Option<Reply>) {
        if let Some(stop) = &self.stop {
            self.state.send_replace(CameraState::Stopping);
            stop.cancel();
            if let Some(reply) = reply {
                if self.stop_reply.is_some() {
                    let _ = reply.send(Err(CameraError::Busy));
                } else {
                    self.stop_reply = Some(reply);
                }
            }
        } else if let Some(reply) = reply {
            self.state.send_replace(CameraState::Idle);
            let _ = reply.send(Ok(()));
        }
    }
    pub fn finished(&mut self, result: Result<(), CameraError>) {
        self.stop = None;
        self.state
            .send_replace(result.map_or_else(CameraState::Failed, |()| CameraState::Idle));
        if let Some(reply) = self.stop_reply.take() {
            let _ = reply.send(result);
        }
    }
}

pub(super) fn allows_camera(permission: Option<&ParticipantPermission>) -> bool {
    permission.is_some_and(|p| {
        p.can_publish
            && (p.can_publish_sources.is_empty()
                || p.can_publish_sources
                    .contains(&(TrackSource::Camera as i32)))
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn camera_grant_is_source_specific_and_fails_closed() {
        assert!(!allows_camera(None));
        let mut grants = ParticipantPermission {
            can_publish: true,
            can_publish_sources: vec![
                TrackSource::ScreenShare as i32,
                TrackSource::Microphone as i32,
            ],
            ..Default::default()
        };
        assert!(!allows_camera(Some(&grants)));
        grants.can_publish_sources.push(TrackSource::Camera as i32);
        assert!(allows_camera(Some(&grants)));
        grants.can_publish = false;
        assert!(!allows_camera(Some(&grants)));
    }

    #[tokio::test]
    async fn stop_cancels_startup_and_waits_for_cleanup() {
        let (_, inbox) = mpsc::channel(1);
        let (state, updates) = watch::channel(CameraState::Starting);
        let mut camera = CameraRoom::new(inbox, state, watch::channel(None).0);
        let cancel = CancellationToken::new();
        camera.stop = Some(cancel.clone());
        let (reply, mut result) = oneshot::channel();
        camera.stop(Some(reply));
        assert!(cancel.is_cancelled());
        assert_eq!(*updates.borrow(), CameraState::Stopping);
        assert!(matches!(
            result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        camera.finished(Ok(()));
        assert_eq!(result.await.unwrap(), Ok(()));
        assert_eq!(*updates.borrow(), CameraState::Idle);
    }
}
