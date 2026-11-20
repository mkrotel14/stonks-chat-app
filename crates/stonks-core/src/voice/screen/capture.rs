use std::{
    os::fd::{AsRawFd, OwnedFd},
    time::Duration,
};

use ashpd::{
    desktop::{
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
        CreateSessionOptions, PersistMode, ResponseError, Session,
    },
    zbus::Connection,
};
use futures_util::StreamExt;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSink;
use gstreamer_video::{VideoFormat, VideoInfo, VideoMeta};
use livekit::{
    prelude::*,
    webrtc::{
        prelude::{I420Buffer, RtcVideoSource, VideoFrame, VideoResolution, VideoRotation},
        video_source::native::NativeVideoSource,
    },
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{
    audio, convert_frame, grant_allows_screen, grant_allows_screen_audio, publish_options, Profile,
    ScreenQuality, ScreenShareError as Error, ScreenShareState as State,
};

pub(super) async fn run(
    room: &Room,
    stop: CancellationToken,
    state: watch::Sender<State>,
    quality: ScreenQuality,
    output: Option<String>,
) -> Result<(), Error> {
    // A private bus connection owns all requests. Disconnecting it also cleans up
    // requests cancelled before ashpd can return a Request or Session handle.
    let connection = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(5), Connection::session()) => {
            result.map_err(|_| Error::Portal)?.map_err(|_| Error::Portal)?
        }
    };
    let mut session = None;
    let result = capture(
        room,
        &stop,
        &state,
        &connection,
        &mut session,
        quality.profile(),
        output,
    )
    .await;
    if let Some(session) = session {
        let _ = tokio::time::timeout(Duration::from_secs(1), session.close()).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), connection.close()).await;
    result
}

async fn capture(
    room: &Room,
    stop: &CancellationToken,
    state: &watch::Sender<State>,
    connection: &Connection,
    session: &mut Option<Session<Screencast>>,
    profile: Profile,
    output: Option<String>,
) -> Result<(), Error> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let create = async {
        let portal = Screencast::with_connection(connection.clone())
            .await
            .map_err(portal_error)?;
        let created = portal
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(portal_error)?;
        Ok((portal, created))
    };
    let (portal, created) = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        result = tokio::time::timeout_at(deadline, create) => result.map_err(|_| Error::Portal)??,
    };
    let created = session.insert(created);
    let mut closed = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        result = tokio::time::timeout_at(deadline, created.receive_closed()) => result.map_err(|_| Error::Portal)?.map_err(portal_error)?,
    };
    let setup = async {
        let sources = portal
            .available_source_types()
            .await
            .map_err(portal_error)?
            & (SourceType::Monitor | SourceType::Window);
        if sources.is_empty() {
            return Err(Error::Portal);
        }
        // Like WebRTC, prefer compositor-embedded cursor pixels. This pipeline
        // does not consume SPA cursor metadata; do not request Metadata mode.
        let cursor = portal
            .available_cursor_modes()
            .await
            .ok()
            .and_then(|modes| {
                [CursorMode::Embedded, CursorMode::Hidden]
                    .into_iter()
                    .find(|mode| modes.contains(*mode))
            });
        portal
            .select_sources(
                created,
                SelectSourcesOptions::default()
                    .set_sources(sources)
                    .set_multiple(false)
                    .set_cursor_mode(cursor)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;
        let streams = portal
            .start(created, None, Default::default())
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;
        let [stream] = streams.streams() else {
            return Err(Error::Portal);
        };
        let fd = portal
            .open_pipe_wire_remote(created, Default::default())
            .await
            .map_err(|error| {
                eprintln!("screen capture: portal remote open failed");
                portal_error(error)
            })?;
        let mut pipe = Pipe::open(fd, stream.pipe_wire_node_id(), profile)?;
        let first = tokio::time::timeout(Duration::from_secs(8), pipe.frame())
            .await
            .map_err(|_| {
                eprintln!("screen capture: timed out waiting for first frame");
                Error::Capture
            })??;
        Ok((pipe, first))
    };
    let (pipe, first) = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        _ = closed.next() => {
            eprintln!("screen capture: portal closed during startup");
            return Err(Error::PortalClosed);
        }
        result = tokio::time::timeout_at(deadline, setup) => result.map_err(|_| Error::Portal)??,
    };
    let participant = room.local_participant();
    if stop.is_cancelled() {
        return Ok(());
    }
    if !grant_allows_screen(participant.permission().as_ref()) {
        return Err(Error::PublishDenied);
    }
    let source = NativeVideoSource::new(
        VideoResolution {
            width: profile.width,
            height: profile.height,
        },
        true,
    );
    let track =
        LocalVideoTrack::create_video_track("screen-share", RtcVideoSource::Native(source.clone()));
    let publish = tokio::time::timeout(
        Duration::from_secs(10),
        participant.publish_track(LocalTrack::Video(track.clone()), publish_options(profile)),
    );
    tokio::pin!(publish);
    // Once publishing starts, collect its result so a racing stop can unpublish
    // the exact track. Stop the capture pipeline immediately, not after signaling.
    let (published, shutdown) = tokio::select! {
        biased;
        _ = stop.cancelled() => (None, Ok(())),
        _ = closed.next() => (None, Err(Error::PortalClosed)),
        result = &mut publish => (Some(result), Ok(())),
    };
    let Some(published) = published else {
        drop(pipe);
        if let Ok(Ok(publication)) = publish.await {
            unpublish(&participant, &track, &publication.sid()).await?;
        }
        return shutdown;
    };
    let publication = published
        .map_err(|_| Error::Publish)?
        .map_err(|_| Error::Publish)?;
    let mut pipe = Some(pipe);
    let mut audio = None;
    let mut audio_publication = None;
    let result = async {
        if let Some(output) = output {
            if !grant_allows_screen_audio(participant.permission().as_ref()) {
                return Err(Error::AudioPublishDenied);
            }
            audio = Some(tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                _ = closed.next() => return Err(Error::PortalClosed),
                result = audio::Audio::open(output) => result?,
            });
            if !grant_allows_screen_audio(participant.permission().as_ref()) {
                return Err(Error::AudioPublishDenied);
            }
            let Some(capture) = audio.as_ref() else { return Err(Error::AudioCapture); };
            let publish = tokio::time::timeout(Duration::from_secs(6), participant.publish_track(
                LocalTrack::Audio(capture.track.clone()), audio::publish_options(),
            ));
            tokio::pin!(publish);
            let interrupted = tokio::select! {
                biased;
                _ = stop.cancelled() => Ok(()),
                _ = closed.next() => Err(Error::PortalClosed),
                result = &mut publish => {
                    audio_publication = Some(result.map_err(|_| Error::AudioPublish)?.map_err(|_| Error::AudioPublish)?.sid());
                    Ok(())
                }
            };
            if audio_publication.is_none() {
                drop(audio.take());
                drop(pipe.take());
                if let Ok(Ok(publication)) = publish.await {
                    audio_publication = Some(publication.sid());
                }
                return interrupted;
            }
        }
        if stop.is_cancelled() {
            return Ok(());
        }
        if !grant_allows_screen(participant.permission().as_ref()) {
            return Err(Error::PublishDenied);
        }
        if audio.is_some() && !grant_allows_screen_audio(participant.permission().as_ref()) {
            return Err(Error::AudioPublishDenied);
        }
        if !state.send_if_modified(|state| {
            if *state != State::Starting {
                return false;
            }
            *state = State::Sharing;
            true
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
        let pipe = pipe.as_mut().ok_or(Error::Capture)?;
        push(&source, first);
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break Ok(()),
                _ = closed.next() => break Err(Error::PortalClosed),
                result = &mut pump => break result,
                frame = pipe.frame() => push(&source, frame?),
            }
        }
    }
    .await;
    drop(audio);
    drop(pipe);
    track.mute();
    let remove_audio = async {
        match audio_publication {
            Some(sid) => unpublish_sid(&participant, &sid).await,
            None => Ok(()),
        }
    };
    let video_sid = publication.sid();
    let (video, audio) = tokio::join!(unpublish_sid(&participant, &video_sid), remove_audio);
    result.and(video).and(audio)
}

async fn unpublish(
    participant: &LocalParticipant,
    track: &LocalVideoTrack,
    sid: &TrackSid,
) -> Result<(), Error> {
    track.mute();
    unpublish_sid(participant, sid).await
}

async fn unpublish_sid(participant: &LocalParticipant, sid: &TrackSid) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(3), participant.unpublish_track(sid))
        .await
        .map_err(|_| Error::Unpublish)?
        .map_err(|_| Error::Unpublish)?;
    Ok(())
}

fn push(source: &NativeVideoSource, frame: I420Buffer) {
    // The WebRTC adapter may drop a frame (including with no sinks yet or
    // for rate adaptation). This is not a failure of the capture pipeline.
    let _ = source.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, frame));
}

fn portal_error(error: ashpd::Error) -> Error {
    match error {
        ashpd::Error::Response(ResponseError::Cancelled) => Error::Cancelled,
        _ => Error::Portal,
    }
}

struct Pipe {
    pipeline: gst::Pipeline,
    bus: gst::Bus,
    sink: AppSink,
    ticks: tokio::time::Interval,
    profile: Profile,
    _fd: OwnedFd,
}

impl Pipe {
    // Keep construction in NULL so the installed ABI can be checked without capture.
    fn build(fd: OwnedFd, node: u32, profile: Profile) -> Result<Self, Error> {
        gst::init().map_err(|_| unavailable("GStreamer initialization"))?;
        let pipeline = gst::parse::launch("pipewiresrc name=portal ! videoconvert ! appsink name=frames max-buffers=1 leaky-type=downstream wait-on-eos=false sync=false enable-last-sample=false")
            .map_err(|_| unavailable("pipeline parse"))?
            .downcast::<gst::Pipeline>().map_err(|_| unavailable("pipeline type"))?;
        let src = pipeline
            .by_name("portal")
            .ok_or_else(|| unavailable("PipeWire source lookup"))?;
        if !src.has_property_with_type("fd", i32::static_type())
            || !src.has_property_with_type("path", String::static_type())
        {
            return Err(unavailable("PipeWire source property ABI"));
        }
        // PipeWire 1.6.11 uses path for the portal's node ID; a numeric
        // target-object means an object serial, not a node ID. The plugin
        // duplicates fd on connect; we keep the original through teardown.
        src.set_property("fd", fd.as_raw_fd());
        src.set_property("path", node.to_string());
        let sink = pipeline
            .by_name("frames")
            .and_then(|sink| sink.downcast::<AppSink>().ok())
            .ok_or_else(|| unavailable("appsink lookup"))?;
        sink.set_caps(Some(
            &gst::Caps::builder("video/x-raw")
                .field("format", "BGRA")
                .build(),
        ));
        let bus = pipeline
            .bus()
            .ok_or_else(|| unavailable("pipeline bus lookup"))?;
        let mut ticks = tokio::time::interval(Duration::from_nanos(1_000_000_000 / profile.fps));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Ok(Self {
            pipeline,
            bus,
            sink,
            ticks,
            profile,
            _fd: fd,
        })
    }

    fn open(fd: OwnedFd, node: u32, profile: Profile) -> Result<Self, Error> {
        let pipe = Self::build(fd, node, profile)?;
        if pipe.pipeline.set_state(gst::State::Playing).is_err() {
            eprintln!("screen capture: pipeline failed to enter Playing");
            let _ = pipe.check_bus();
            return Err(Error::Capture);
        }
        Ok(pipe)
    }

    fn check_bus(&self) -> Result<(), Error> {
        let mut failed = false;
        for message in self.bus.iter() {
            match message.view() {
                gst::MessageView::Error(message) => {
                    let error = message.error();
                    // Only log typed codes, never backend messages, debug strings,
                    // or object paths (which can carry portal or device details).
                    eprintln!(
                        "screen capture: pipeline error: core={:?} resource={:?} stream={:?} library={:?}",
                        error.kind::<gst::CoreError>(),
                        error.kind::<gst::ResourceError>(),
                        error.kind::<gst::StreamError>(),
                        error.kind::<gst::LibraryError>(),
                    );
                    failed = true;
                }

                _ => {}
            }
        }
        if failed {
            Err(Error::Capture)
        } else {
            Ok(())
        }
    }

    async fn frame(&mut self) -> Result<I420Buffer, Error> {
        loop {
            self.ticks.tick().await;
            self.check_bus()?;
            if let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) {
                return convert_sample(&sample, self.profile);
            }
            // With wait-on-eos=false, the bus can report EOS while the last
            // (possibly first) sample is still queued. Drain that sample first.
            if self.sink.is_eos() {
                return Err(frame_error("pipeline reached EOS"));
            }
        }
    }
}

fn convert_sample(sample: &gst::Sample, profile: Profile) -> Result<I420Buffer, Error> {
    let info = VideoInfo::from_caps(sample.caps().ok_or_else(|| frame_error("missing caps"))?)
        .map_err(|_| frame_error("invalid video caps"))?;
    if info.format() != VideoFormat::Bgra {
        return Err(frame_error("expected BGRA caps"));
    }
    let buffer = sample
        .buffer()
        .ok_or_else(|| frame_error("missing buffer"))?;
    let (offset, stride) = if let Some(meta) = buffer.meta::<VideoMeta>() {
        if meta.format() != VideoFormat::Bgra
            || meta.n_planes() != 1
            || meta.width() != info.width()
            || meta.height() != info.height()
        {
            return Err(frame_error("inconsistent BGRA metadata"));
        }
        (meta.offset()[0], meta.stride()[0])
    } else {
        (info.offset()[0], info.stride()[0])
    };
    let stride = u32::try_from(stride).map_err(|_| frame_error("invalid stride"))?;
    // Use the actual mapped extent. VideoFrameRef::plane_data constructs a
    // stride*height slice even when the final row has no trailing padding.
    let map = buffer
        .map_readable()
        .map_err(|_| frame_error("buffer map"))?;
    let data = map
        .as_slice()
        .get(offset..)
        .ok_or_else(|| frame_error("invalid plane offset"))?;
    convert_frame(profile, data, stride, info.width(), info.height()).map_err(|error| {
        eprintln!("screen capture: frame conversion failed: {error:?}");
        Error::Capture
    })
}

fn unavailable(stage: &'static str) -> Error {
    eprintln!("screen capture: {stage} failed");
    Error::CaptureUnavailable
}

fn frame_error(stage: &'static str) -> Error {
    eprintln!("screen capture: frame rejected: {stage}");
    Error::Capture
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bgra_sample(tail: usize) -> gst::Sample {
        gst::init().unwrap();
        let mut pixels = vec![0; tail];
        for row in [4, 16] {
            pixels[row..row + 7].copy_from_slice(&[0, 0, 255, 255, 0, 0, 255]);
        }
        let mut buffer = gst::Buffer::from_mut_slice(pixels);
        VideoMeta::add_full(
            buffer.get_mut().unwrap(),
            gstreamer_video::VideoFrameFlags::empty(),
            VideoFormat::Bgra,
            2,
            2,
            &[4],
            &[12],
        )
        .unwrap();
        let caps = VideoInfo::builder(VideoFormat::Bgra, 2, 2)
            .build()
            .unwrap()
            .to_caps()
            .unwrap();
        gst::Sample::builder().buffer(&buffer).caps(&caps).build()
    }

    #[test]
    fn padded_bgra_uses_metadata_and_reads_only_visible_pixels() {
        let frame = convert_sample(&bgra_sample(24), ScreenQuality::Hd30.profile()).unwrap();
        let (y, u, v) = frame.data();
        let (sy, su, sv) = frame.strides();
        for row in [358, 359] {
            assert_eq!(&y[row * sy as usize + 638..][..2], &[82, 82]);
        }
        assert_eq!(u[179 * su as usize + 319], 90);
        assert!(v[179 * sv as usize + 319] >= 239);
        assert!(matches!(
            convert_sample(&bgra_sample(23), ScreenQuality::Hd30.profile()),
            Err(Error::Capture)
        ));
    }

    #[tokio::test]
    async fn first_frame_drains_even_when_pipeline_has_already_posted_eos() {
        gst::init().unwrap();
        let pipeline = gst::parse::launch("videotestsrc num-buffers=1 ! video/x-raw,format=BGRx,width=2,height=2 ! videoconvert ! video/x-raw,format=BGRA ! appsink name=frames max-buffers=1 wait-on-eos=false sync=false")
            .unwrap().downcast::<gst::Pipeline>().unwrap();
        let (local, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut pipe = Pipe {
            sink: pipeline.by_name("frames").unwrap().downcast().unwrap(),
            bus: pipeline.bus().unwrap(),
            pipeline,
            ticks: tokio::time::interval(Duration::from_millis(1)),
            profile: ScreenQuality::Hd30.profile(),
            _fd: local.into(),
        };
        pipe.pipeline.set_state(gst::State::Playing).unwrap();
        let eos = pipe
            .bus
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(3),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .unwrap();
        assert_eq!(eos.type_(), gst::MessageType::Eos);
        pipe.bus.post(eos).unwrap();
        tokio::time::timeout(Duration::from_secs(1), pipe.frame())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), pipe.frame())
                .await
                .unwrap(),
            Err(Error::Capture)
        ));
    }

    #[tokio::test]
    async fn adapter_drop_is_not_a_capture_failure() {
        let source = NativeVideoSource::new(
            VideoResolution {
                width: 1280,
                height: 720,
            },
            true,
        );
        // An unattached source has no consumers; exercise the SDK's actual drop path.
        assert!(!source.capture_frame(&VideoFrame::new(
            VideoRotation::VideoRotation0,
            I420Buffer::new_black(1280, 720)
        )));
        let (): () = push(&source, I420Buffer::new_black(1280, 720));
    }

    #[tokio::test]
    #[ignore = "requires the installed GStreamer PipeWire, videoconvert and appsink plugins"]
    async fn installed_portal_pipeline_in_null() {
        let (local, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let pipe = Pipe::build(local.into(), 42, ScreenQuality::Hd30.profile()).unwrap();
        assert_eq!(pipe.pipeline.current_state(), gst::State::Null);
        let src = pipe.pipeline.by_name("portal").unwrap();
        assert_eq!(src.property::<String>("path"), "42");
        assert_eq!(src.property::<Option<String>>("target-object"), None);
        assert_eq!(src.property::<i32>("fd"), pipe._fd.as_raw_fd());

        let offered = src.static_pad("src").unwrap().peer_query_caps(None);
        let raw = gst::Caps::builder("video/x-raw")
            .field("format", "BGRx")
            .build();
        assert!(offered.can_intersect(&raw));
        let output = pipe.sink.caps().unwrap();
        assert_eq!(
            output.structure(0).unwrap().get::<&str>("format").unwrap(),
            "BGRA"
        );
    }
}
