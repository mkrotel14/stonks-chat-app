use super::{allows_camera, devices, CameraError as Error, CameraOptions, CameraState};
use livekit::{
    options::{DegradationPreference, TrackPublishOptions, VideoEncoding},
    prelude::*,
    webrtc::{
        prelude::{I420Buffer, RtcVideoSource, VideoFrame, VideoResolution, VideoRotation},
        video_source::native::NativeVideoSource,
    },
};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::{watch, Mutex, MutexGuard},
};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    room: Arc<Room>,
    options: CameraOptions,
    stop: CancellationToken,
    state: watch::Sender<CameraState>,
) -> Result<(), Error> {
    let participant = room.local_participant();
    if !allows_camera(participant.permission().as_ref()) {
        return Err(Error::PublishDenied);
    }
    let mut capture = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        capture = Capture::open(&options) => capture?,
    };
    let result = publish(&participant, &mut capture, options, stop, state).await;
    capture.close().await;
    result
}

async fn publish(
    participant: &LocalParticipant,
    capture: &mut Capture,
    options: CameraOptions,
    stop: CancellationToken,
    state: watch::Sender<CameraState>,
) -> Result<(), Error> {
    let (width, height, fps, bitrate) = options.quality.profile();
    tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        result = capture.read() => result?,
    }
    if stop.is_cancelled() {
        return Ok(());
    }
    if !allows_camera(participant.permission().as_ref()) {
        return Err(Error::PublishDenied);
    }
    let source = NativeVideoSource::new(VideoResolution { width, height }, false);
    let track =
        LocalVideoTrack::create_video_track("camera", RtcVideoSource::Native(source.clone()));
    let publish = tokio::time::timeout(
        Duration::from_secs(10),
        participant.publish_track(
            LocalTrack::Video(track.clone()),
            TrackPublishOptions {
                source: TrackSource::Camera,
                simulcast: false,
                video_encoding: Some(VideoEncoding {
                    max_bitrate: bitrate,
                    max_framerate: f64::from(fps),
                }),
                degradation_preference: Some(DegradationPreference::MaintainResolution),
                ..Default::default()
            },
        ),
    );
    tokio::pin!(publish);
    // Collect an in-flight publication even after cancellation so its SID can be removed.
    let published = tokio::select! {
        biased;
        _ = stop.cancelled() => {
            capture.close().await;
            if let Ok(Ok(publication)) = publish.await {
                unpublish(participant, &publication.sid()).await?;
            }
            return Ok(());
        }
        result = &mut publish => result,
    }
    .map_err(|_| Error::Publish)?
    .map_err(|_| Error::Publish)?;
    state.send_if_modified(|current| {
        if *current == CameraState::Starting && !stop.is_cancelled() {
            *current = CameraState::Publishing;
            true
        } else {
            false
        }
    });
    let result = loop {
        if stop.is_cancelled() {
            break Ok(());
        }
        let buffer = capture.i420();
        let _ = source.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buffer));
        tokio::select! {
            biased;
            _ = stop.cancelled() => break Ok(()),
            result = capture.read() => {
                if let Err(error) = result { break Err(error); }
            }
        }
    };
    capture.close().await;
    track.mute();
    unpublish(participant, &published.sid()).await?;
    result
}

async fn unpublish(participant: &LocalParticipant, sid: &TrackSid) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(3), participant.unpublish_track(sid))
        .await
        .map_err(|_| Error::Unpublish)?
        .map_err(|_| Error::Unpublish)?;
    Ok(())
}

// Hold through process reaping so a new preview or publication cannot race the
// old process for exclusive access to a hardware camera.
static CAMERA: Mutex<()> = Mutex::const_new(());

pub(super) struct Capture {
    child: Child,
    bytes: Vec<u8>,
    exclusive: Option<MutexGuard<'static, ()>>,
    width: u32,
    height: u32,
}

impl Capture {
    pub(super) async fn open(options: &CameraOptions) -> Result<Self, Error> {
        let exclusive = CAMERA.lock().await;
        let graph = devices().await?;
        let device = super::resolve_device(&graph, &options.device)?;
        let (width, height, fps, _) = options.quality.profile();
        let mode = format!("width={width},height={height},framerate={fps}/1");
        let mut command = Command::new("gst-launch-1.0");
        command.arg("-q");
        #[cfg(target_os = "linux")]
        command.args(device.source_args());
        #[cfg(windows)]
        command.args(["mfvideosrc", &format!("device-path={}", device.id)]);
        let child = command
            .args([
                "!",
                "capsfilter",
                &format!("caps=image/jpeg,{mode};video/x-raw,{mode}"),
                "!",
                "decodebin",
                "!",
                "videoconvert",
                "!",
                &format!("video/x-raw,format=I420,{mode},pixel-aspect-ratio=1/1"),
                "!",
                "fdsink",
                "fd=1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error::CaptureUnavailable)?;
        Ok(Self {
            child,
            bytes: vec![0; (width * height * 3 / 2) as usize],
            exclusive: Some(exclusive),
            width,
            height,
        })
    }

    pub(super) async fn read(&mut self) -> Result<(), Error> {
        let stdout = self.child.stdout.as_mut().ok_or(Error::Capture)?;
        tokio::time::timeout(Duration::from_secs(6), stdout.read_exact(&mut self.bytes))
            .await
            .map_err(|_| Error::Capture)?
            .map_err(|_| Error::Capture)?;
        Ok(())
    }

    pub(super) fn i420(&self) -> I420Buffer {
        let (width, height) = (self.width, self.height);
        let mut buffer = I420Buffer::new(width, height);
        let (y, u, v) = buffer.data_mut();
        let luma = (width * height) as usize;
        let (luma_bytes, chroma) = self.bytes.split_at(luma);
        let (u_bytes, v_bytes) = chroma.split_at(luma / 4);
        y.copy_from_slice(luma_bytes);
        u.copy_from_slice(u_bytes);
        v.copy_from_slice(v_bytes);
        buffer
    }

    pub(super) async fn close(&mut self) {
        if self.exclusive.is_some() {
            let _ = self.child.kill().await;
            self.exclusive.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn replaced_preview_can_cancel_while_previous_capture_owns_camera() {
        let _previous_capture = CAMERA.lock().await;
        let (stop, cancelled) = oneshot::channel();
        let (frames, latest) = watch::channel(None);
        let preview = super::super::run_preview(
            CameraOptions {
                device: "default".into(),
                quality: Default::default(),
            },
            cancelled,
            frames,
        );
        tokio::pin!(preview);
        poll_fn(|cx| {
            assert!(preview.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(stop);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), preview)
                .await
                .unwrap(),
            Ok(())
        );
        assert!(latest.borrow().is_none());
    }
}
