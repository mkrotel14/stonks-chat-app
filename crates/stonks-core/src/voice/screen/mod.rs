//! Screen sharing for the native voice room.
#[cfg(target_os = "linux")]
mod audio;
#[cfg(target_os = "windows")]
#[path = "windows_audio.rs"]
mod audio;
#[cfg(target_os = "linux")]
mod capture;
#[cfg(target_os = "linux")]
mod routing;
#[cfg(windows)]
mod windows_capture;
pub use super::video_view::VideoFrame as ScreenFrame;
#[cfg(windows)]
pub use windows_capture::{sources, ScreenSelection, ScreenSource, ScreenSourceKind};

use livekit::prelude::Room;
use std::{fmt, sync::Arc};
use tokio::{
    sync::{oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenShareError {
    Disconnected,
    Busy,
    PublishDenied,
    AudioPublishDenied,
    AudioUnavailable,
    AudioCapture,
    AudioPublish,
    Cancelled,
    Portal,
    PortalClosed,
    CaptureUnavailable,
    #[cfg(windows)]
    SourceRequired,
    #[cfg(windows)]
    SourceUnavailable,
    Capture,
    Publish,
    Unpublish,
}

impl fmt::Display for ScreenShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disconnected => "Join a connected voice room first.",
            Self::Busy => "Screen sharing is already active or stopping.",
            Self::PublishDenied => "This room does not allow you to share your screen.",
            Self::AudioPublishDenied => "This room does not allow screen-share audio.",
            Self::AudioUnavailable => {
                #[cfg(target_os = "linux")]
                {
                    "Could not establish a private PipeWire application-audio route."
                }
                #[cfg(windows)]
                {
                    "Could not start isolated screen-share audio. It requires Windows build 20348 or newer and an available Windows audio service."
                }
            }
            Self::AudioCapture => "The private application-audio route failed.",
            Self::AudioPublish => "Could not publish screen-share audio.",
            Self::Cancelled => "Screen selection was cancelled.",
            Self::Portal => "The desktop ScreenCast portal could not start screen sharing.",
            Self::PortalClosed => "The desktop portal ended screen sharing.",
            #[cfg(target_os = "windows")]
            Self::CaptureUnavailable => "Windows desktop capture is unavailable.",
            #[cfg(target_os = "linux")]
            Self::CaptureUnavailable => "Screen sharing requires the GStreamer PipeWire plugin.",
            #[cfg(windows)]
            Self::SourceRequired => "Choose a monitor or window to share.",
            #[cfg(windows)]
            Self::SourceUnavailable => "The selected monitor or window is no longer available.",
            #[cfg(windows)]
            Self::Capture => "Windows screen video capture failed.",
            #[cfg(target_os = "linux")]
            Self::Capture => "The portal's PipeWire video stream failed.",
            Self::Publish => "Could not publish the screen share.",
            Self::Unpublish => "Could not remove the screen share from the room.",
        })
    }
}
impl std::error::Error for ScreenShareError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenShareState {
    Idle,
    Starting,
    Sharing,
    Stopping,
    Failed(ScreenShareError),
}

type Reply = oneshot::Sender<Result<(), ScreenShareError>>;
pub(super) enum Command {
    Start(ScreenShareOptions, Reply),
    Stop(Reply),
    View(Option<String>, Reply),
}

pub(super) struct Sharing {
    pub jobs: JoinSet<Result<(), ScreenShareError>>,
    stop: Option<CancellationToken>,
    stop_reply: Option<Reply>,
    with_audio: bool,
    state: watch::Sender<ScreenShareState>,
}

impl Sharing {
    pub fn new(state: watch::Sender<ScreenShareState>) -> Self {
        Self {
            jobs: JoinSet::new(),
            stop: None,
            stop_reply: None,
            with_audio: false,
            state,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn start(
        &mut self,
        room: Arc<Room>,
        connected: bool,
        options: ScreenShareOptions,
        output: &str,
    ) -> Result<(), ScreenShareError> {
        if !connected {
            return Err(ScreenShareError::Disconnected);
        }
        if self.stop.is_some() {
            return Err(ScreenShareError::Busy);
        }
        if !grant_allows_screen(room.local_participant().permission().as_ref()) {
            return Err(ScreenShareError::PublishDenied);
        }
        if options.share_audio
            && !grant_allows_screen_audio(room.local_participant().permission().as_ref())
        {
            return Err(ScreenShareError::AudioPublishDenied);
        }
        self.with_audio = options.share_audio;
        let output = options.share_audio.then(|| output.to_owned());
        let stop = CancellationToken::new();
        self.stop = Some(stop.clone());
        self.state.send_replace(ScreenShareState::Starting);
        let state = self.state.clone();
        self.jobs
            .spawn(async move { capture::run(&room, stop, state, options.quality, output).await });
        Ok(())
    }

    pub fn check_grant(&mut self, permission: Option<&ParticipantPermission>) {
        if !grant_allows_screen(permission)
            || (self.with_audio && !grant_allows_screen_audio(permission))
        {
            self.stop(None);
        }
    }

    #[cfg(target_os = "windows")]
    pub fn start(
        &mut self,
        room: Arc<Room>,
        connected: bool,
        options: ScreenShareOptions,
        _output: &str,
    ) -> Result<(), ScreenShareError> {
        if !connected {
            return Err(ScreenShareError::Disconnected);
        }
        if self.stop.is_some() {
            return Err(ScreenShareError::Busy);
        }
        if !grant_allows_screen(room.local_participant().permission().as_ref()) {
            return Err(ScreenShareError::PublishDenied);
        }
        if options.share_audio
            && !grant_allows_screen_audio(room.local_participant().permission().as_ref())
        {
            return Err(ScreenShareError::AudioPublishDenied);
        }
        let selection = options.selection.ok_or(ScreenShareError::SourceRequired)?;
        self.with_audio = options.share_audio;
        let stop = CancellationToken::new();
        self.stop = Some(stop.clone());
        self.state.send_replace(ScreenShareState::Starting);
        self.jobs.spawn(windows_capture::run(
            room,
            stop,
            self.state.clone(),
            options.quality,
            selection,
            options.share_audio,
        ));
        Ok(())
    }

    pub fn stop(&mut self, reply: Option<Reply>) {
        if let Some(stop) = &self.stop {
            self.state.send_replace(ScreenShareState::Stopping);
            stop.cancel();
            if let Some(reply) = reply {
                if self.stop_reply.is_some() {
                    let _ = reply.send(Err(ScreenShareError::Busy));
                } else {
                    self.stop_reply = Some(reply);
                }
            }
        } else if let Some(reply) = reply {
            self.state.send_replace(ScreenShareState::Idle);
            let _ = reply.send(Ok(()));
        }
    }

    pub fn finished(&mut self, result: Result<(), ScreenShareError>) {
        self.stop = None;
        self.with_audio = false;
        self.state.send_replace(match result {
            Ok(()) => ScreenShareState::Idle,
            Err(error) => ScreenShareState::Failed(error),
        });
        if let Some(reply) = self.stop_reply.take() {
            let _ = reply.send(result);
        }
    }
}

use super::video_view::Viewing;

pub(super) struct ScreenRoom {
    pub inbox: tokio::sync::mpsc::Receiver<Command>,
    pub sharing: Sharing,
    pub viewing: Viewing,
    pub viewed_audio: tokio::sync::watch::Sender<Option<String>>,
}

use livekit::{
    options::{DegradationPreference, TrackPublishOptions, VideoEncoding},
    prelude::TrackSource,
    webrtc::{native::yuv_helper, prelude::I420Buffer},
};
use livekit_protocol::{ParticipantPermission, TrackSource as GrantSource};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ScreenQuality {
    #[default]
    Hd30,
    FullHd30,
    FullHd60,
}

/// Audio is opt-in per start. Linux uses the configured output; Windows
/// captures playback across endpoints while excluding the Stonks process tree.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScreenShareOptions {
    pub quality: ScreenQuality,
    pub share_audio: bool,
    #[cfg(windows)]
    pub selection: Option<ScreenSelection>,
}

impl ScreenQuality {
    pub const ALL: [Self; 3] = [Self::Hd30, Self::FullHd30, Self::FullHd60];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hd30 => "720p · 30 FPS",
            Self::FullHd30 => "1080p · 30 FPS",
            Self::FullHd60 => "1080p · 60 FPS",
        }
    }

    // Same canvas, capture cadence and bitrate ceiling as Tauri.
    fn profile(self) -> Profile {
        let (width, height, fps, bitrate) = match self {
            Self::Hd30 => (1280, 720, 30, 4_000_000),
            Self::FullHd30 => (1920, 1080, 30, 8_000_000),
            Self::FullHd60 => (1920, 1080, 60, 12_000_000),
        };
        Profile {
            width,
            height,
            fps,
            bitrate,
        }
    }
}

#[derive(Clone, Copy)]
struct Profile {
    width: u32,
    height: u32,
    fps: u64,
    bitrate: u64,
}

/// Fail closed until a room supplies a publish grant. Microphone and screen audio
/// grants are not substitutes for permission to publish screen video.
pub fn grant_allows_screen(permission: Option<&ParticipantPermission>) -> bool {
    grant_allows(permission, GrantSource::ScreenShare)
}

pub fn grant_allows_screen_audio(permission: Option<&ParticipantPermission>) -> bool {
    grant_allows(permission, GrantSource::ScreenShareAudio)
}

fn grant_allows(permission: Option<&ParticipantPermission>, source: GrantSource) -> bool {
    permission.is_some_and(|permission| {
        permission.can_publish
            && (permission.can_publish_sources.is_empty()
                || permission.can_publish_sources.contains(&(source as i32)))
    })
}

fn publish_options(profile: Profile) -> TrackPublishOptions {
    TrackPublishOptions {
        source: TrackSource::Screenshare,
        video_encoding: Some(VideoEncoding {
            max_bitrate: profile.bitrate,
            max_framerate: profile.fps as f64,
        }),
        degradation_preference: Some(DegradationPreference::MaintainFramerateAndResolution),
        simulcast: false,
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    InvalidDimensions,
    InvalidStride,
    Truncated,
}

/// Convert packed BGRA bytes (libyuv's ARGB on little-endian targets) to a
/// fixed-size I420 frame without upscaling. Final-row padding is optional.
fn convert_frame(
    profile: Profile,
    data: &[u8],
    stride: u32,
    width: u32,
    height: u32,
) -> Result<I420Buffer, FrameError> {
    // Bound the temporary full-size conversion buffer before passing dimensions to libyuv.
    if !(2..=8192).contains(&width) || !(2..=8192).contains(&height) {
        return Err(FrameError::InvalidDimensions);
    }
    let row_bytes = width * 4;
    if stride < row_bytes || stride > i32::MAX as u32 {
        return Err(FrameError::InvalidStride);
    }
    // The binding checks the full padded extent using u32 arithmetic.
    let required = stride
        .checked_mul(height)
        .ok_or(FrameError::InvalidStride)?;
    let visible_end = required - (stride - row_bytes);
    if (visible_end as usize) > data.len() {
        return Err(FrameError::Truncated);
    }
    // The binding requires stride * height bytes. Repack only if the buffer
    // ends before that boundary; never read or fabricate a slice past it.
    let packed;
    let (data, stride) = if (required as usize) > data.len() {
        packed = data[..visible_end as usize]
            .chunks(stride as usize)
            .flat_map(|row| row[..row_bytes as usize].iter().copied())
            .collect::<Vec<_>>();
        (packed.as_slice(), row_bytes)
    } else {
        (data, stride)
    };

    let mut input = I420Buffer::new(width, height);
    let (sy, su, sv) = input.strides();
    let (y, u, v) = input.data_mut();
    yuv_helper::argb_to_i420(
        data,
        stride,
        y,
        sy,
        u,
        su,
        v,
        sv,
        width as i32,
        height as i32,
    );

    let (w, h) = fitted_dimensions(profile, width, height);
    let input = if (w, h) == (width, height) {
        input
    } else {
        input.scale(w as i32, h as i32)
    };
    if (w, h) == (profile.width, profile.height) {
        return Ok(input);
    }
    let mut output = I420Buffer::new_black(profile.width, profile.height);
    // Even offsets keep the luma and subsampled chroma planes aligned.
    let x = (((profile.width - w) / 2) & !1) as usize;
    let y = (((profile.height - h) / 2) & !1) as usize;
    let (src_y, src_u, src_v) = input.data();
    let (sy, su, sv) = input.strides();
    let (dy, du, dv) = output.strides();
    let (dst_y, dst_u, dst_v) = output.data_mut();
    for (src, src_stride, dst, dst_stride, divisor) in [
        (src_y, sy, dst_y, dy, 1),
        (src_u, su, dst_u, du, 2),
        (src_v, sv, dst_v, dv, 2),
    ] {
        let cols = w as usize / divisor;
        for row in 0..h as usize / divisor {
            let src_start = row * src_stride as usize;
            let dst_start = (row + y / divisor) * dst_stride as usize + x / divisor;
            dst[dst_start..dst_start + cols].copy_from_slice(&src[src_start..src_start + cols]);
        }
    }
    Ok(output)
}

fn fitted_dimensions(profile: Profile, width: u32, height: u32) -> (u32, u32) {
    let (w, h) = if width <= profile.width && height <= profile.height {
        (width, height)
    } else if profile.width * height <= profile.height * width {
        (profile.width, height * profile.width / width)
    } else {
        (width * profile.height / height, profile.height)
    };
    (w.max(2) & !1, h.max(2) & !1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use livekit::webrtc::prelude::VideoBuffer;

    #[tokio::test]
    async fn stop_cancels_startup_and_acknowledges_only_after_cleanup() {
        let (state, observer) = watch::channel(ScreenShareState::Starting);
        let mut sharing = Sharing::new(state);
        let token = CancellationToken::new();
        sharing.stop = Some(token.clone());
        let (reply, mut ack) = oneshot::channel();

        sharing.stop(Some(reply));
        assert!(token.is_cancelled());
        assert_eq!(*observer.borrow(), ScreenShareState::Stopping);
        assert!(matches!(
            ack.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        sharing.finished(Ok(()));
        assert_eq!(ack.await.unwrap(), Ok(()));
        assert_eq!(*observer.borrow(), ScreenShareState::Idle);
    }

    #[test]
    fn losing_audio_grant_stops_only_shares_that_requested_audio() {
        let permission = ParticipantPermission {
            can_publish: true,
            can_publish_sources: vec![GrantSource::ScreenShare as i32],
            ..Default::default()
        };
        for with_audio in [false, true] {
            let (state, observer) = watch::channel(ScreenShareState::Sharing);
            let mut sharing = Sharing::new(state);
            let stop = CancellationToken::new();
            sharing.stop = Some(stop.clone());
            sharing.with_audio = with_audio;

            sharing.check_grant(Some(&permission));
            assert_eq!(stop.is_cancelled(), with_audio);
            assert_eq!(
                *observer.borrow(),
                if with_audio {
                    ScreenShareState::Stopping
                } else {
                    ScreenShareState::Sharing
                },
            );
        }
    }

    #[test]
    fn only_screen_grants_allow_screen_publication() {
        let mut grant = ParticipantPermission::default();
        assert!(!grant_allows_screen(None));
        assert!(!grant_allows_screen(Some(&grant)));
        grant.can_publish = true;
        assert!(grant_allows_screen(Some(&grant)));
        grant.can_publish_sources = vec![
            GrantSource::Microphone as i32,
            GrantSource::ScreenShareAudio as i32,
        ];
        assert!(!grant_allows_screen(Some(&grant)));
        grant
            .can_publish_sources
            .push(GrantSource::ScreenShare as i32);
        assert!(grant_allows_screen(Some(&grant)));
        assert_eq!(
            publish_options(ScreenQuality::Hd30.profile()).source,
            TrackSource::Screenshare
        );
    }

    #[test]
    fn audio_opt_in_requires_its_own_grant_and_publishes_a_distinct_source() {
        assert!(!ScreenShareOptions::default().share_audio);
        assert!(!grant_allows_screen_audio(None));
        let mut permission = ParticipantPermission {
            can_publish: true,
            can_publish_sources: vec![GrantSource::ScreenShare as i32],
            ..Default::default()
        };
        assert!(!grant_allows_screen_audio(Some(&permission)));
        permission
            .can_publish_sources
            .push(GrantSource::ScreenShareAudio as i32);
        assert!(grant_allows_screen_audio(Some(&permission)));
        permission.can_publish = false;
        assert!(!grant_allows_screen_audio(Some(&permission)));
        assert_eq!(
            audio::publish_options().source,
            TrackSource::ScreenshareAudio
        );
        assert!(!audio::publish_options().dtx);
    }

    #[test]
    fn profiles_set_canvas_and_single_layer_encoding_limits() {
        for (quality, (width, height, fps, bitrate)) in ScreenQuality::ALL.into_iter().zip([
            (1280, 720, 30, 4_000_000),
            (1920, 1080, 30, 8_000_000),
            (1920, 1080, 60, 12_000_000),
        ]) {
            let profile = quality.profile();
            assert_eq!(
                (profile.width, profile.height, profile.fps),
                (width, height, fps)
            );
            let options = publish_options(profile);
            assert!(!options.simulcast);
            let encoding = options.video_encoding.unwrap();
            assert_eq!(encoding.max_framerate, fps as f64);
            assert_eq!(encoding.max_bitrate, bitrate);
            let frame = convert_frame(profile, &[255; 16], 8, 2, 2).unwrap();
            assert_eq!((frame.width(), frame.height()), (width, height));
        }
    }

    #[test]
    fn rejects_malformed_frames_before_conversion() {
        assert!(matches!(
            convert_frame(ScreenQuality::Hd30.profile(), &[], 4, 1, 1),
            Err(FrameError::InvalidDimensions)
        ));
        assert!(matches!(
            convert_frame(ScreenQuality::Hd30.profile(), &[], 7, 2, 2),
            Err(FrameError::InvalidStride)
        ));
        assert!(matches!(
            convert_frame(ScreenQuality::Hd30.profile(), &[0; 19], 12, 2, 2),
            Err(FrameError::Truncated)
        ));
    }

    #[test]
    fn fits_with_even_dimensions_without_upscaling() {
        assert_eq!(
            fitted_dimensions(ScreenQuality::Hd30.profile(), 1920, 1080),
            (1280, 720)
        );
        assert_eq!(
            fitted_dimensions(ScreenQuality::Hd30.profile(), 1080, 1920),
            (404, 720)
        );
        assert_eq!(
            fitted_dimensions(ScreenQuality::Hd30.profile(), 641, 479),
            (640, 478)
        );
    }

    #[test]
    fn padded_white_frame_is_centered_on_black() {
        // Two white pixels per row, with padding between rows.
        let mut pixels = [255; 32];
        pixels[8..16].fill(0);
        let frame = convert_frame(ScreenQuality::Hd30.profile(), &pixels, 16, 2, 2).unwrap();
        assert_eq!((frame.width(), frame.height()), (1280, 720));
        let (y, _, _) = frame.data();
        let stride = frame.strides().0 as usize;
        assert!(y[358 * stride + 638] > 200);
        assert!(y[359 * stride + 639] > 200);
        assert!(y[358 * stride + 637] < 32);
        assert!(y[360 * stride + 639] < 32);
    }
}
