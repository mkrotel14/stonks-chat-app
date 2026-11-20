use super::*;
use gstreamer as gst;
use gstreamer_video::prelude::*;
use std::time::{Duration, Instant};
#[derive(Clone, Copy)]
pub(super) enum AudioOutput {
    System,
    Silent,
}
struct Engine {
    pipeline: gst::Pipeline,
    sink: gstreamer_app::AppSink,
    source: Source,
    playing: bool,
    buffering: bool,
    ended: bool,
    ready: bool,
    seek: Option<f64>,
    frame_serial: u64,
    last_query: Instant,
    snapshot: Snapshot,
    fault: Arc<Mutex<bool>>,
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
impl Engine {
    fn new(source: Source, audio: AudioOutput, volume: f64) -> Result<Self, PlaybackError> {
        gst::init().map_err(|_| PlaybackError::Engine)?;
        let pipeline = gst::ElementFactory::make("playbin3")
            .build()
            .map_err(|_| PlaybackError::Engine)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| PlaybackError::Engine)?;
        let sink = gstreamer_app::AppSink::builder()
            .sync(true)
            .max_buffers(1)
            .max_bytes(MAX_FRAME_BYTES as u64)
            .wait_on_eos(false)
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "BGRA")
                    .field(
                        "width",
                        gst::IntRange::<i32>::new(1, MAX_VIDEO_WIDTH as i32),
                    )
                    .field(
                        "height",
                        gst::IntRange::<i32>::new(1, MAX_VIDEO_HEIGHT as i32),
                    )
                    .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                    .build(),
            )
            .build();
        sink.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);
        let audio_sink = match audio {
            AudioOutput::System => gst::ElementFactory::make("autoaudiosink").build(),
            AudioOutput::Silent => gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build(),
        }
        .map_err(|_| PlaybackError::Engine)?;
        pipeline.set_property("audio-sink", &audio_sink);
        pipeline.set_property("video-sink", &sink);
        pipeline.set_property_from_str(
            "flags",
            if source.kind == MediaKind::Video {
                "audio+video+soft-volume+buffering"
            } else {
                "audio+soft-volume+buffering"
            },
        );
        pipeline.set_property("buffer-size", 2_097_152_i32);
        pipeline.set_property("buffer-duration", 2_000_000_000_i64);
        pipeline.set_property("volume", volume);
        let fault = Arc::new(Mutex::new(false));
        let element_fault = fault.clone();
        pipeline.connect_deep_element_added(move |_, _, element| {
            let Some(factory) = element.factory() else {
                return;
            };
            // Attachments and the resolver use direct combined media. Do not
            // let an adaptive manifest spawn additional, unvalidated URIs.
            let adaptive = matches!(
                factory.name().as_str(),
                "hlsdemux" | "hlsdemux2" | "dashdemux" | "dashdemux2" | "mssdemux" | "mssdemux2"
            );
            let foreign_source = !element.is::<gst::Bin>()
                && factory
                    .metadata("klass")
                    .is_some_and(|klass| klass.contains("Source"))
                && factory.name() != "souphttpsrc";
            if adaptive || foreign_source {
                element.set_locked_state(true);
                if let Ok(mut fault) = element_fault.lock() {
                    *fault = true;
                }
            }
        });
        let source_fault = fault.clone();
        let headers = source.headers.clone();
        pipeline.connect("source-setup", false, move |values| {
            let Some(source) = values
                .get(1)
                .and_then(|value| value.get::<gst::Element>().ok())
            else {
                return None;
            };
            if source
                .factory()
                .is_none_or(|factory| factory.name() != "souphttpsrc")
            {
                source.set_locked_state(true);
                if let Ok(mut fault) = source_fault.lock() {
                    *fault = true;
                }
                return None;
            }
            source.set_property("automatic-redirect", false);
            source.set_property("ssl-strict", true);
            source.set_property("location-trusted", false);
            source.set_property("timeout", 10_u32);
            source.set_property("user-id", None::<String>);
            source.set_property("user-pw", None::<String>);
            source.set_property("cookies", gst::glib::StrV::new());
            for (name, value) in &headers {
                if name.eq_ignore_ascii_case("user-agent") {
                    source.set_property("user-agent", value);
                } else if name.eq_ignore_ascii_case("referer") {
                    source.set_property(
                        "extra-headers",
                        gst::Structure::builder("headers")
                            .field("Referer", value)
                            .build(),
                    );
                }
            }
            None
        });
        pipeline.set_property("uri", &source.url);
        let playing = source.autoplay;
        if pipeline
            .set_state(if playing {
                gst::State::Playing
            } else {
                gst::State::Paused
            })
            .is_err()
        {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(PlaybackError::Unsupported);
        }
        Ok(Self {
            pipeline,
            sink,
            source: source.clone(),
            playing,
            buffering: false,
            ended: false,
            ready: false,
            seek: None,
            frame_serial: 0,
            last_query: Instant::now() - Duration::from_secs(1),
            snapshot: Snapshot {
                key: Some(source.key),
                status: PlaybackStatus::Loading,
                volume,
                ..Default::default()
            },
            fault,
        })
    }
    fn controls(&mut self, playing: bool, seek: Option<f64>, volume: f64) {
        if volume != self.snapshot.volume {
            self.pipeline.set_property("volume", volume);
            self.snapshot.volume = volume;
        }
        if let Some(seek) = seek {
            self.seek = Some(seek);
            self.ended = false;
        }
        if playing != self.playing {
            self.playing = playing;
            if playing && self.ended {
                self.seek = Some(0.);
                self.ended = false;
            }
            let _ = self.pipeline.set_state(if playing && !self.buffering {
                gst::State::Playing
            } else {
                gst::State::Paused
            });
        }
    }
    fn poll(&mut self) -> Result<(), PlaybackError> {
        if self.fault.lock().map(|fault| *fault).unwrap_or(true) {
            return Err(PlaybackError::Unavailable);
        }
        let bus = self.pipeline.bus().ok_or(PlaybackError::Engine)?;
        for message in bus.iter_timed(gst::ClockTime::ZERO).take(64) {
            match message.view() {
                gst::MessageView::Error(_) => return Err(PlaybackError::Unsupported),
                gst::MessageView::Eos(_) => {
                    // A replay/seek may be queued before the old EOS reaches
                    // this bus drain. The pending flush invalidates that EOS.
                    if self.seek.is_some() {
                        continue;
                    }
                    self.ended = true;
                    self.playing = false;
                    // GStreamer retains PLAYING at EOS. Explicitly pause so
                    // seeking after the end does not restart audio by itself.
                    let _ = self.pipeline.set_state(gst::State::Paused);
                    self.snapshot.status = PlaybackStatus::Ended;
                    if self.snapshot.duration > 0. {
                        self.snapshot.position = self.snapshot.duration;
                    }
                }
                gst::MessageView::AsyncDone(_) => self.ready = true,
                gst::MessageView::Buffering(buffering) => {
                    self.buffering = buffering.percent() < 100;
                    let _ = self.pipeline.set_state(if self.playing && !self.buffering {
                        gst::State::Playing
                    } else {
                        gst::State::Paused
                    });
                }
                gst::MessageView::ClockLost(_) => {
                    if self.playing {
                        let _ = self.pipeline.set_state(gst::State::Paused);
                        let _ = self.pipeline.set_state(gst::State::Playing);
                    }
                }
                gst::MessageView::Latency(_) => {
                    let _ = self.pipeline.recalculate_latency();
                }
                _ => {}
            }
        }
        if self.pipeline.current_state() == gst::State::Playing
            || self.pipeline.current_state() == gst::State::Paused
        {
            self.ready = true;
        }
        if self.ready {
            if let Some(position) = self.seek.take() {
                let end = if self.snapshot.duration > 0. {
                    position.min(self.snapshot.duration)
                } else {
                    position
                };
                let target =
                    gst::ClockTime::try_from_seconds_f64(end).map_err(|_| PlaybackError::Seek)?;
                self.pipeline
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, target)
                    .map_err(|_| PlaybackError::Seek)?;
                self.snapshot.position = end;
            }
        }
        if self.last_query.elapsed() >= Duration::from_millis(80) {
            if let Some(duration) = self.pipeline.query_duration::<gst::ClockTime>() {
                self.snapshot.duration = duration.seconds_f64();
            }
            if !self.ended {
                if let Some(position) = self.pipeline.query_position::<gst::ClockTime>() {
                    self.snapshot.position = position.seconds_f64();
                }
            }
            self.last_query = Instant::now();
        }
        if self.source.kind == MediaKind::Video {
            // pull_sample clears the preroll reference even when a paused
            // pipeline cannot return a playing sample. Read the paused frame
            // directly so Preview displays its first frame without autoplay.
            let sample = if self.pipeline.current_state() == gst::State::Playing {
                self.sink.try_pull_sample(gst::ClockTime::ZERO)
            } else {
                self.sink.try_pull_preroll(gst::ClockTime::ZERO)
            };
            if let Some(sample) = sample {
                let info = gstreamer_video::VideoInfo::from_caps(
                    sample.caps().ok_or(PlaybackError::Unsupported)?,
                )
                .map_err(|_| PlaybackError::Unsupported)?;
                if info.width() > MAX_VIDEO_WIDTH
                    || info.height() > MAX_VIDEO_HEIGHT
                    || info.format() != gstreamer_video::VideoFormat::Bgra
                {
                    return Err(PlaybackError::Unsupported);
                }
                let frame = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(
                    sample.buffer().ok_or(PlaybackError::Unsupported)?,
                    &info,
                )
                .map_err(|_| PlaybackError::Unsupported)?;
                let stride = usize::try_from(frame.plane_stride()[0])
                    .map_err(|_| PlaybackError::Unsupported)?;
                let data = frame
                    .plane_data(0)
                    .map_err(|_| PlaybackError::Unsupported)?;
                let row = info.width() as usize * 4;
                let mut pixels = Vec::with_capacity(row * info.height() as usize);
                for y in 0..info.height() as usize {
                    let start = y.checked_mul(stride).ok_or(PlaybackError::Unsupported)?;
                    pixels.extend_from_slice(
                        data.get(start..start + row)
                            .ok_or(PlaybackError::Unsupported)?,
                    );
                }
                self.frame_serial = self.frame_serial.wrapping_add(1);
                self.snapshot.frame = Some(VideoFrame {
                    width: info.width(),
                    height: info.height(),
                    bgra: pixels.into(),
                    serial: self.frame_serial,
                });
            }
        }
        if !self.ended {
            self.snapshot.status = if !self.ready || self.buffering {
                PlaybackStatus::Loading
            } else if self.playing {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Paused
            };
        }
        Ok(())
    }
}
/// Metadata-only audio preroll has its own fakesink and never enters PLAYING.
/// The caller's owned cancellation flag is checked while awaiting metadata;
/// Engine's guard always sets NULL before the bounded probe slot is released.
pub(super) fn probe_audio(source: Source, cancelled: &AtomicBool) -> Result<f64, PlaybackError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(PlaybackError::Unavailable);
    }
    let mut engine = Engine::new(source, AudioOutput::Silent, 0.)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
        engine.poll()?;
        let duration = engine.snapshot.duration;
        if duration.is_finite() && duration > 0. {
            return Ok(duration);
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    Err(PlaybackError::Unavailable)
}

pub(super) fn run(shared: Arc<Shared>, audio: AudioOutput) {
    let mut generation = 0;
    let mut engine: Option<Engine> = None;
    while !shared.shutdown.load(Ordering::Acquire) {
        let Ok(mut desired) = shared.desired.lock() else {
            break;
        };
        let current = desired.generation;
        let source = if current != generation {
            desired.source.take()
        } else {
            None
        };
        let playing = desired.playing;
        let volume = desired.volume;
        let seek = desired.seek.take();
        drop(desired);
        if current != generation {
            engine = None;
            generation = current;
            if let Some(source) = source {
                match Engine::new(source, audio, volume) {
                    Ok(value) => engine = Some(value),
                    Err(error) => {
                        if let Ok(desired) = shared.desired.lock() {
                            if desired.generation == generation {
                                if let Ok(mut snapshot) = shared.snapshot.lock() {
                                    snapshot.status = PlaybackStatus::Failed;
                                    snapshot.error = Some(error);
                                    snapshot.revision = snapshot.revision.wrapping_add(1);
                                }
                            }
                        }
                    }
                }
            }
        }
        if shared.shutdown.load(Ordering::Acquire)
            || shared
                .desired
                .lock()
                .is_ok_and(|desired| desired.generation != generation)
        {
            engine = None;
            continue;
        }
        if let Some(current_engine) = engine.as_mut() {
            current_engine.controls(playing, seek, volume);
            let result = current_engine.poll();
            if result.is_err() {
                current_engine.snapshot.status = PlaybackStatus::Failed;
                current_engine.snapshot.error = result.err();
            }
            if let Ok(mut desired) = shared.desired.lock() {
                if desired.generation == generation {
                    if let Ok(mut snapshot) = shared.snapshot.lock() {
                        let next = &current_engine.snapshot;
                        if snapshot.key != next.key
                            || snapshot.status != next.status
                            || snapshot.position != next.position
                            || snapshot.duration != next.duration
                            || snapshot.volume != next.volume
                            || snapshot.error != next.error
                            || snapshot.frame.as_ref().map(|frame| frame.serial)
                                != next.frame.as_ref().map(|frame| frame.serial)
                        {
                            let revision = snapshot.revision.wrapping_add(1);
                            *snapshot = next.clone();
                            snapshot.revision = revision;
                        }
                    }
                    if current_engine.ended {
                        desired.playing = false;
                    }
                }
            }
            if result.is_err() {
                engine = None;
            }
        }
        std::thread::park_timeout(
            if engine
                .as_ref()
                .is_some_and(|engine| engine.playing || !engine.ready || engine.buffering)
            {
                Duration::from_millis(16)
            } else {
                Duration::from_millis(100)
            },
        );
    }
    drop(engine);
    shared.finished.store(true, Ordering::Release);
}
#[cfg(test)]
mod tests {
    use super::{AudioOutput, Engine};
    use crate::media_playback::{MediaKind, Source};
    use gstreamer::prelude::*;
    #[test]
    fn initial_volume_is_applied_to_the_pipeline_before_autoplay() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let source = Source::new(
            "initial-volume".into(),
            format!("http://{}/pending.mp3", listener.local_addr().unwrap()),
            MediaKind::Audio,
            true,
            vec![],
        )
        .unwrap();
        let engine = Engine::new(source, AudioOutput::Silent, 0.).unwrap();
        assert_eq!(engine.pipeline.property::<f64>("volume"), 0.);
        drop(engine);
    }
}
