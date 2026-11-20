//! Linux PipeWire PCM transport, using the same format as the Tauri voice path.
use std::{process::Stdio, time::Duration};

use futures_util::StreamExt;
use livekit::{
    prelude::RemoteAudioTrack,
    webrtc::{
        audio_source::native::NativeAudioSource, audio_stream::native::NativeAudioStream,
        native::apm::AudioProcessingModule, prelude::AudioFrame,
    },
};
use nnnoiseless::DenoiseState;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout, Command},
};

use super::{devices, listening::RemoteGain, VoiceAudioOptions, VoiceError, VoiceListeningGains};

const SAMPLES: usize = DenoiseState::FRAME_SIZE;

fn pipewire(mode: &str, device: &str, latency: &str, channels: &str) -> Command {
    let mut command = Command::new("pw-cat");
    command
        .args([
            mode,
            "--raw",
            "--rate",
            "48000",
            "--channels",
            channels,
            "--format",
            "s16",
            "--latency",
            latency,
        ])
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if !device.is_empty() && device != "default" {
        command.args([
            "--target",
            device.strip_prefix("pipewire:").unwrap_or(device),
        ]);
    }
    if mode == "--playback" {
        command.args([
            "--properties",
            "application.name=Stonks application.id=com.rdxlabs.stonks",
        ]);
    }
    command.arg("-");
    command
}

// A single sink monitor supplies the mixed signal actually sent to the selected
// output, not independently timed copies of each LiveKit playback stream.
// These samples are only passed to WebRTC's reverse processor, never published.
struct RenderReference {
    _child: Child,
    pcm: PcmReader,
    last_frame: tokio::time::Instant,
}

impl RenderReference {
    async fn open(device: &str) -> Option<Self> {
        let use_default = device.is_empty() || device == "default";
        let output = devices::discover().await.ok()?.into_iter().find(|node| {
            node.kind == devices::Kind::Output
                && if use_default {
                    node.is_default
                } else {
                    node.id == device
                }
        })?;
        let mut child = pipewire("--record", &output.id, "10ms", "1")
            .args([
                "--properties",
                "stream.capture.sink=true node.dont-fallback=true node.dont-reconnect=true",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        Some(Self {
            pcm: PcmReader::new(child.stdout.take()?),
            _child: child,
            last_frame: tokio::time::Instant::now(),
        })
    }
}

struct PcmReader {
    stdout: ChildStdout,
    bytes: [u8; SAMPLES * 2],
    filled: usize,
}

impl PcmReader {
    fn new(stdout: ChildStdout) -> Self {
        Self {
            stdout,
            bytes: [0; SAMPLES * 2],
            filled: 0,
        }
    }

    // A room event can cancel a read without discarding partial PCM frames.
    async fn read(&mut self) -> Result<(), VoiceError> {
        let n = self
            .stdout
            .read(&mut self.bytes[self.filled..])
            .await
            .map_err(|_| VoiceError::Capture)?;
        if n == 0 {
            return Err(VoiceError::Capture);
        }
        self.filled += n;
        Ok(())
    }

    fn take_frame(&mut self) -> Option<[i16; SAMPLES]> {
        if self.filled != self.bytes.len() {
            return None;
        }
        self.filled = 0;
        Some(std::array::from_fn(|i| {
            i16::from_le_bytes([self.bytes[i * 2], self.bytes[i * 2 + 1]])
        }))
    }
}

pub(super) struct Capture {
    _child: Child,
    pcm: PcmReader,
    source: NativeAudioSource,
    denoiser: Option<Box<DenoiseState<'static>>>,
    processing: Option<AudioProcessingModule>,
    reference: Option<RenderReference>,
    agc: bool,
    gain: u8,
}

fn processing(aec: bool, agc: bool) -> Option<AudioProcessingModule> {
    // RNNoise remains the only noise suppressor. Avoid modifying PCM at all
    // through WebRTC when both processing preferences are off.
    (aec || agc).then(|| AudioProcessingModule::new(aec, agc, true, false))
}

impl Capture {
    pub(super) async fn open(
        audio: &VoiceAudioOptions,
        source: NativeAudioSource,
    ) -> Result<Self, VoiceError> {
        let reference = if audio.echo_cancellation {
            let reference = RenderReference::open(&audio.output_device).await;
            if reference.is_none() {
                eprintln!("voice room: no PipeWire output monitor; echo cancellation unavailable");
            }
            reference
        } else {
            None
        };
        let mut child = pipewire("--record", &audio.input_device, "20ms", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| VoiceError::Capture)?;
        let mut pcm = PcmReader::new(child.stdout.take().ok_or(VoiceError::Capture)?);
        tokio::time::timeout(Duration::from_secs(3), async {
            while pcm.filled < pcm.bytes.len() {
                pcm.read().await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| VoiceError::Capture)??;
        pcm.take_frame();
        Ok(Self {
            _child: child,
            pcm,
            source,
            denoiser: audio.rnnoise.then(DenoiseState::new),
            processing: processing(reference.is_some(), audio.automatic_gain_control),
            reference,
            agc: audio.automatic_gain_control,
            gain: audio.microphone_volume,
        })
    }

    fn disable_echo_cancellation(&mut self) {
        eprintln!(
            "voice room: PipeWire output reference lost; continuing without echo cancellation"
        );
        self.reference = None;
        self.processing = processing(false, self.agc);
    }

    pub(super) fn clear(&self) {
        self.source.clear_buffer();
    }

    pub(super) async fn read(&mut self) -> Result<(), VoiceError> {
        let Some(reference) = &mut self.reference else {
            return self.pcm.read().await;
        };
        tokio::select! {
            result = self.pcm.read() => result,
            result = reference.pcm.read() => {
                if result.is_err() {
                    self.disable_echo_cancellation();
                } else if let Some(mut frame) = reference.pcm.take_frame() {
                    if let Some(processing) = &mut self.processing {
                        if processing.process_reverse_stream(&mut frame, 48_000, 1).is_err() {
                            self.disable_echo_cancellation();
                        } else {
                            reference.last_frame = tokio::time::Instant::now();
                        }
                    }
                }
                Ok(())
            }
        }
    }

    pub(super) async fn send_frame(&mut self, muted: bool) -> Result<(), VoiceError> {
        let Some(mut pcm) = self.pcm.take_frame() else {
            return Ok(());
        };
        if self
            .reference
            .as_ref()
            .is_some_and(|r| r.last_frame.elapsed() > Duration::from_secs(2))
        {
            self.disable_echo_cancellation();
        }
        if let Some(processing) = &mut self.processing {
            processing
                .process_stream(&mut pcm, 48_000, 1)
                .map_err(|_| VoiceError::Capture)?;
        }
        let input = pcm.map(f32::from);
        let mut filtered = [0.0; SAMPLES];
        let output = match &mut self.denoiser {
            Some(denoiser) => {
                denoiser.process_frame(&mut filtered, &input);
                &filtered
            }
            None => &input,
        };
        if muted {
            return Ok(());
        }
        let pcm = output.map(|sample| scale(sample, self.gain));
        self.source
            .capture_frame(&AudioFrame {
                data: pcm.as_slice().into(),
                sample_rate: 48_000,
                num_channels: 1,
                samples_per_channel: SAMPLES as u32,
            })
            .await
            .map_err(|_| VoiceError::Capture)
    }
}

pub(super) async fn play(
    track: RemoteAudioTrack,
    device: String,
    gain: u8,
    stereo: bool,
    gain_kind: RemoteGain,
    listening_gains: VoiceListeningGains,
) -> Result<(), VoiceError> {
    let channels = if stereo { 2 } else { 1 };
    let mut child = pipewire(
        "--playback",
        &device,
        "60ms",
        if stereo { "2" } else { "1" },
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .spawn()
    .map_err(|_| VoiceError::Playback)?;
    let mut stdin = child.stdin.take().ok_or(VoiceError::Playback)?;
    let mut stream = NativeAudioStream::new(track.rtc_track(), 48_000, channels);
    let mut bytes = Vec::new();
    loop {
        tokio::select! {
            _ = child.wait() => return Err(VoiceError::Playback),
            frame = stream.next() => {
                let Some(frame) = frame else { return Ok(()); };
                let listening = gain_kind.percent(&listening_gains);
                bytes.clear();
                for &sample in frame.data.iter() {
                    bytes.extend_from_slice(&scale_playback(sample, gain, listening).to_le_bytes());
                }
                stdin.write_all(&bytes).await.map_err(|_| VoiceError::Playback)?;
            }
        }
    }
}

pub(super) struct SoundboardOutput {
    child: Child,
    stdin: tokio::process::ChildStdin,
}

impl SoundboardOutput {
    pub(super) async fn open(device: &str) -> Result<Self, crate::soundboard::SoundboardError> {
        use crate::soundboard::SoundboardError::Playback;
        let mut child = pipewire("--playback", device, "20ms", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|_| Playback)?;
        let stdin = child.stdin.take().ok_or(Playback)?;
        Ok(Self { child, stdin })
    }

    pub(super) async fn write(
        &mut self,
        frame: &[i16; SAMPLES],
        listening: u8,
    ) -> Result<(), crate::soundboard::SoundboardError> {
        use crate::soundboard::SoundboardError::Playback;
        if self.child.try_wait().map_err(|_| Playback)?.is_some() {
            return Err(Playback);
        }
        let mut bytes = [0; SAMPLES * 2];
        for (out, sample) in bytes.chunks_exact_mut(2).zip(frame) {
            out.copy_from_slice(&scale_playback(*sample, 100, listening).to_le_bytes());
        }
        // A stalled local device must not hold up the LiveKit pump.
        tokio::time::timeout(Duration::from_millis(10), self.stdin.write_all(&bytes))
            .await
            .map_err(|_| Playback)?
            .map_err(|_| Playback)
    }
}

pub(super) async fn preview(
    pcm: &[i16],
    volume: u8,
    device: &str,
    gains: &VoiceListeningGains,
) -> Result<(), crate::soundboard::SoundboardError> {
    use crate::soundboard::SoundboardError::Playback;
    let mut child = pipewire("--playback", device, "20ms", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|_| Playback)?;
    let mut stdin = child.stdin.take().ok_or(Playback)?;
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut bytes = Vec::with_capacity(SAMPLES * 2);
        for frame in pcm.chunks(SAMPLES) {
            let listening = gains.soundboard_listening_gain();
            bytes.clear();
            bytes.extend(
                frame
                    .iter()
                    .flat_map(|&sample| scale_playback(sample, volume, listening).to_le_bytes()),
            );
            stdin.write_all(&bytes).await.map_err(|_| Playback)?;
        }
        drop(stdin);
        if !child.wait().await.map_err(|_| Playback)?.success() {
            return Err(Playback);
        }
        Ok(())
    })
    .await
    .map_err(|_| Playback)?
}

fn scale_playback(sample: i16, base: u8, listening: u8) -> i16 {
    if listening == 100 {
        return scale(f32::from(sample), base);
    }
    // Compose gains before saturation so attenuation can offset a base boost.
    (f32::from(sample) * f32::from(base) * f32::from(listening) / 10_000.0) as i16
}

fn scale(sample: f32, gain: u8) -> i16 {
    (sample * f32::from(gain) / 100.0) as i16
}

#[cfg(test)]
mod tests {
    use super::{processing, scale, scale_playback, VoiceListeningGains, SAMPLES};

    #[test]
    fn webrtc_processing_accepts_pipewire_frames() {
        assert!(processing(false, false).is_none());
        for (aec, agc) in [(false, true), (true, false), (true, true)] {
            let mut apm = processing(aec, agc).unwrap();
            for _ in 0..10 {
                if aec {
                    apm.process_reverse_stream(&mut [200; SAMPLES], 48_000, 1)
                        .unwrap();
                }
                apm.process_stream(&mut [100; SAMPLES], 48_000, 1).unwrap();
            }
        }
    }

    #[test]
    fn local_soundboard_attenuation_does_not_modify_publish_pcm() {
        let gains = VoiceListeningGains::default();
        let published = [12_000, -12_000];
        for (gain, expected) in [(100, published), (25, [3_000, -3_000]), (0, [0, 0])] {
            gains.set_soundboard_listening_gain(gain);
            let local = published
                .map(|sample| scale_playback(sample, 100, gains.soundboard_listening_gain()));
            assert_eq!(local, expected);
            assert_eq!(published, [12_000, -12_000]);
        }
        assert_eq!(scale_playback(20_000, 200, 50), 20_000);
    }

    #[test]
    fn gain_saturates_without_wrapping() {
        assert_eq!(scale(20_000.0, 200), i16::MAX);
        assert_eq!(scale(-20_000.0, 200), i16::MIN);
        assert_eq!(scale(-500.0, 0), 0);
    }
}
