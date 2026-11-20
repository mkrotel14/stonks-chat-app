use std::{
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use livekit::{
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStdout, Command},
};

use super::{routing, ScreenShareError as Error};

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const FRAME: usize = (RATE / 100 * CHANNELS) as usize;

pub(super) struct Audio {
    pub track: LocalAudioTrack,
    child: Child,
    stdout: ChildStdout,
    source: NativeAudioSource,
    name: String,
    pid: u32,
    output: String,
}

impl Audio {
    pub async fn open(output: String) -> Result<Self, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "stonks-native-share-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let props = format!("node.name={name} application.name=Stonks application.id=com.rdxlabs.stonks node.autoconnect=false node.always-process=true adapter.auto-port-config={{ mode=dsp }}");
        let mut child = Command::new("pw-cat")
            .args([
                "--record",
                "--raw",
                "--rate",
                "48000",
                "--channels",
                "2",
                "--format",
                "s16",
                "--latency",
                "10ms",
                "--properties",
                &props,
                "--target",
                "0",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error::AudioUnavailable)?;
        let stdout = child.stdout.take().ok_or(Error::AudioUnavailable)?;
        let pid = child.id().ok_or(Error::AudioUnavailable)?;
        routing::start(&name, pid, &output)
            .await
            .map_err(|_| Error::AudioUnavailable)?;
        let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, CHANNELS, 20);
        let track = LocalAudioTrack::create_audio_track(
            "screen-share-audio",
            RtcAudioSource::Native(source.clone()),
        );
        Ok(Self {
            track,
            child,
            stdout,
            source,
            name,
            pid,
            output,
        })
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut bytes = [0; FRAME * 2];
        let mut pcm = [0; FRAME];
        let mut filled = 0;
        loop {
            tokio::select! {
                biased;
                _ = self.child.wait() => return Err(Error::AudioCapture),
                _ = interval.tick() => {
                    // Don't pump PCM while inspecting or changing the route.
                    routing::refresh(&self.name, self.pid, &self.output).await.map_err(|_| Error::AudioCapture)?;
                }
                read = self.stdout.read(&mut bytes[filled..]) => {
                    let read = read.map_err(|_| Error::AudioCapture)?;
                    if read == 0 { return Err(Error::AudioCapture); }
                    filled += read;
                    if filled != bytes.len() { continue; }
                    filled = 0;
                    for (sample, bytes) in pcm.iter_mut().zip(bytes.chunks_exact(2)) {
                        *sample = i16::from_le_bytes([bytes[0], bytes[1]]);
                    }
                    tokio::time::timeout(Duration::from_millis(250), self.source.capture_frame(&AudioFrame {
                        data: pcm.as_slice().into(), sample_rate: RATE, num_channels: CHANNELS,
                        samples_per_channel: RATE / 100,
                    })).await.map_err(|_| Error::AudioCapture)?.map_err(|_| Error::AudioCapture)?;
                }
            }
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.track.mute();
        self.source.clear_buffer();
        let _ = self.child.start_kill();
    }
}

pub(super) fn publish_options() -> livekit::options::TrackPublishOptions {
    livekit::options::TrackPublishOptions {
        source: TrackSource::ScreenshareAudio,
        dtx: false,
        ..Default::default()
    }
}
