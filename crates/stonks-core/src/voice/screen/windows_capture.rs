use super::{
    audio, convert_frame, grant_allows_screen, grant_allows_screen_audio, publish_options, Profile,
    ScreenQuality, ScreenShareError as Error, ScreenShareState,
};
use livekit::{
    prelude::*,
    webrtc::{
        prelude::{I420Buffer, RtcVideoSource, VideoFrame, VideoResolution, VideoRotation},
        video_source::native::NativeVideoSource,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use webrtc_sys::desktop_capturer::{
    ffi, CaptureError, DesktopCapturerCallback, DesktopCapturerCallbackWrapper,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSourceKind {
    Screen,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSelection {
    pub kind: ScreenSourceKind,
    pub id: u64,
}

#[derive(Debug, Clone)]
pub struct ScreenSource {
    pub selection: ScreenSelection,
    pub name: String,
}

fn capturer(kind: ScreenSourceKind, include_cursor: bool) -> cxx::UniquePtr<ffi::DesktopCapturer> {
    ffi::new_desktop_capturer(ffi::DesktopCapturerOptions {
        source_type: match kind {
            ScreenSourceKind::Screen => ffi::SourceType::Screen,
            ScreenSourceKind::Window => ffi::SourceType::Window,
        },
        include_cursor,
        allow_sck_system_picker: false,
    })
}

/// Enumerates metadata only; capturing requires a separate explicit start.
pub async fn sources() -> Result<Vec<ScreenSource>, Error> {
    tokio::task::spawn_blocking(|| {
        let mut sources = Vec::new();
        for (kind, fallback) in [
            (ScreenSourceKind::Screen, "Display"),
            (ScreenSourceKind::Window, "Window"),
        ] {
            let capture = capturer(kind, false);
            let Some(capture) = capture.as_ref() else {
                continue;
            };
            sources.extend(
                capture
                    .get_source_list()
                    .into_iter()
                    .map(|source| ScreenSource {
                        selection: ScreenSelection {
                            kind,
                            id: source.id,
                        },
                        name: if source.title.trim().is_empty() {
                            format!("{fallback} {}", source.id)
                        } else {
                            source.title
                        },
                    }),
            );
        }
        if sources.is_empty() {
            Err(Error::SourceUnavailable)
        } else {
            Ok(sources)
        }
    })
    .await
    .map_err(|_| Error::CaptureUnavailable)?
}

struct Frames {
    profile: Profile,
    frames: mpsc::Sender<I420Buffer>,
    failed: Arc<std::sync::atomic::AtomicBool>,
}

impl DesktopCapturerCallback for Frames {
    fn on_capture_result(
        &mut self,
        result: Result<cxx::UniquePtr<ffi::DesktopFrame>, CaptureError>,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        let frame = match result {
            Ok(frame) => frame,
            Err(CaptureError::Temporary) => return,
            Err(CaptureError::Permanent) => {
                self.failed.store(true, Relaxed);
                return;
            }
        };
        let (Some(frame), Ok(slot)) = (frame.as_ref(), self.frames.try_reserve()) else {
            return;
        };
        let (Ok(width), Ok(height), Ok(stride)) = (
            u32::try_from(frame.width()),
            u32::try_from(frame.height()),
            u32::try_from(frame.stride()),
        ) else {
            return;
        };
        if !(2..=8192).contains(&width)
            || !(2..=8192).contains(&height)
            || stride < width * 4
            || frame.data().is_null()
        {
            return;
        }
        let Some(len) = stride.checked_mul(height) else {
            return;
        };
        if len as u64 > isize::MAX as u64 {
            return;
        }
        // The SDK owns stride * height bytes for the lifetime of this DesktopFrame.
        let data = unsafe { std::slice::from_raw_parts(frame.data(), len as usize) };
        if let Ok(buffer) = convert_frame(self.profile, data, stride, width, height) {
            slot.send(buffer);
        }
    }
}

fn capture(
    selection: ScreenSelection,
    profile: Profile,
    stop: CancellationToken,
    frames: mpsc::Sender<I420Buffer>,
) -> Result<(), Error> {
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    let mut capture = capturer(selection.kind, true);
    let handle = capture.as_ref().ok_or(Error::CaptureUnavailable)?;
    if !handle
        .get_source_list()
        .iter()
        .any(|source| source.id == selection.id)
        || !handle.select_source(selection.id)
    {
        return Err(Error::SourceUnavailable);
    }
    if stop.is_cancelled() {
        return Ok(());
    }
    let failed = Arc::new(AtomicBool::new(false));
    capture
        .pin_mut()
        .start(Box::new(DesktopCapturerCallbackWrapper::new(Box::new(
            Frames {
                profile,
                frames,
                failed: failed.clone(),
            },
        ))));
    let interval = Duration::from_secs_f64(1.0 / profile.fps as f64);
    while !stop.is_cancelled() {
        let start = Instant::now();
        capture.capture_frame();
        if failed.load(Relaxed) {
            return Err(Error::Capture);
        }
        std::thread::sleep(interval.saturating_sub(start.elapsed()));
    }
    Ok(())
}

pub(super) async fn run(
    room: Arc<Room>,
    stop: CancellationToken,
    state: watch::Sender<ScreenShareState>,
    quality: ScreenQuality,
    selection: ScreenSelection,
    share_audio: bool,
) -> Result<(), Error> {
    let participant = room.local_participant();
    if stop.is_cancelled() {
        return Ok(());
    }
    check_grants(&participant, share_audio)?;
    let profile = quality.profile();
    let (frames, mut receive) = mpsc::channel(1);
    let capture_stop = stop.child_token();
    let _cancel_on_drop = capture_stop.clone().drop_guard();
    let worker = tokio::task::spawn_blocking({
        let stop = capture_stop.clone();
        move || capture(selection, profile, stop, frames)
    });
    let result = publish(
        &participant,
        &stop,
        &capture_stop,
        &state,
        profile,
        &mut receive,
        share_audio,
    )
    .await;
    capture_stop.cancel();
    let worker_result = worker.await.map_err(|_| Error::Capture)?;
    if stop.is_cancelled() {
        return result;
    }
    worker_result.and(result)
}

async fn publish(
    participant: &LocalParticipant,
    stop: &CancellationToken,
    capture_stop: &CancellationToken,
    state: &watch::Sender<ScreenShareState>,
    profile: Profile,
    frames: &mut mpsc::Receiver<I420Buffer>,
    share_audio: bool,
) -> Result<(), Error> {
    let first = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        frame = next_frame(frames) => frame?,
    };
    check_grants(participant, share_audio)?;
    let source = NativeVideoSource::new(
        VideoResolution {
            width: profile.width,
            height: profile.height,
        },
        true,
    );
    let track =
        LocalVideoTrack::create_video_track("screen-share", RtcVideoSource::Native(source.clone()));
    let _ = source.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, first));
    let publishing = tokio::time::timeout(
        Duration::from_secs(10),
        participant.publish_track(LocalTrack::Video(track.clone()), publish_options(profile)),
    );
    tokio::pin!(publishing);
    let publication = tokio::select! {
        biased;
        _ = stop.cancelled() => { capture_stop.cancel(); publishing.await }
        result = &mut publishing => result,
    };
    let publication = match publication {
        Ok(Ok(publication)) => publication,
        _ if stop.is_cancelled() => return Ok(()),
        _ => return Err(Error::Publish),
    };
    let mut audio = None;
    let mut audio_sid = None;
    let result = async {
        if stop.is_cancelled() {
            return Ok(());
        }
        check_grants(participant, share_audio)?;
        let permission = participant.permission();
        audio = tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            result = audio::Audio::open_if_requested(share_audio, permission.as_ref()) => result?,
        };
        check_grants(participant, share_audio)?;
        if let Some(capture) = audio.as_ref() {
            let publishing = tokio::time::timeout(
                Duration::from_secs(6),
                participant.publish_track(
                    LocalTrack::Audio(capture.track.clone()),
                    audio::publish_options(),
                ),
            );
            tokio::pin!(publishing);
            let published = tokio::select! {
                biased;
                _ = stop.cancelled() => {
                    capture_stop.cancel();
                    track.mute();
                    drop(audio.take());
                    // Collect a racing successful publish so cleanup has its SID.
                    if let Ok(Ok(publication)) = publishing.await {
                        audio_sid = Some(publication.sid());
                    }
                    return Ok(());
                }
                result = &mut publishing => result,
            };
            audio_sid = Some(published.map_err(|_| Error::AudioPublish)?
                .map_err(|_| Error::AudioPublish)?.sid());
        }
        if stop.is_cancelled() {
            return Ok(());
        }
        check_grants(participant, share_audio)?;
        if !state.send_if_modified(|current| {
            if *current == ScreenShareState::Starting && !stop.is_cancelled() {
                *current = ScreenShareState::Sharing;
                true
            } else {
                false
            }
        }) {
            return Ok(());
        }
        let pump = async {
            match audio.as_mut() {
                Some(capture) => capture.run().await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(pump);
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break Ok(()),
                result = &mut pump => break result,
                frame = next_frame(frames) => {
                    let _ = source.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, frame?));
                }
            }
        }
    }
    .await;
    capture_stop.cancel();
    drop(audio);
    track.mute();
    let video_sid = publication.sid();
    let remove_audio = async {
        match audio_sid {
            Some(sid) => unpublish(participant, &sid).await,
            None => Ok(()),
        }
    };
    let (video, audio) = tokio::join!(unpublish(participant, &video_sid), remove_audio);
    result.and(video).and(audio)
}

fn check_grants(participant: &LocalParticipant, share_audio: bool) -> Result<(), Error> {
    let permission = participant.permission();
    if !grant_allows_screen(permission.as_ref()) {
        return Err(Error::PublishDenied);
    }
    if share_audio && !grant_allows_screen_audio(permission.as_ref()) {
        return Err(Error::AudioPublishDenied);
    }
    Ok(())
}

async fn unpublish(participant: &LocalParticipant, sid: &TrackSid) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(3), participant.unpublish_track(sid))
        .await
        .map_err(|_| Error::Unpublish)?
        .map_err(|_| Error::Unpublish)?;
    Ok(())
}

async fn next_frame(frames: &mut mpsc::Receiver<I420Buffer>) -> Result<I420Buffer, Error> {
    tokio::time::timeout(Duration::from_secs(6), frames.recv())
        .await
        .map_err(|_| Error::Capture)?
        .ok_or(Error::Capture)
}
