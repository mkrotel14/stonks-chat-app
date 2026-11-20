//! WASAPI streams are owned by the call; callbacks never wait on the RTC worker.
use std::{
    num::{NonZeroU16, NonZeroU32},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use super::{devices, listening::RemoteGain, VoiceAudioOptions, VoiceError, VoiceListeningGains};
use crate::soundboard::SoundboardError;
use futures_util::StreamExt;
use livekit::{
    prelude::RemoteAudioTrack,
    webrtc::{
        audio_source::native::NativeAudioSource, audio_stream::native::NativeAudioStream,
        native::apm::AudioProcessingModule, prelude::AudioFrame,
    },
};
use nnnoiseless::DenoiseState;
use rodio::{
    cpal::{
        self,
        traits::{DeviceTrait, StreamTrait},
    },
    DeviceSinkBuilder, MixerDeviceSink, Source,
};
use tokio::sync::mpsc;

const RATE: u32 = 48_000;
const SAMPLES: usize = DenoiseState::FRAME_SIZE;
const STARTUP: Duration = Duration::from_secs(3);
type Pcm = [i16; SAMPLES];

pub(super) struct Capture {
    _stream: cpal::Stream,
    frames: mpsc::Receiver<Pcm>,
    frame: Option<Pcm>,
    source: NativeAudioSource,
    denoiser: Option<Box<DenoiseState<'static>>>,
    processing: Option<AudioProcessingModule>,
    gain: u8,
}

impl Capture {
    pub(super) async fn open(
        audio: &VoiceAudioOptions,
        source: NativeAudioSource,
    ) -> Result<Self, VoiceError> {
        let device = audio.input_device.clone();
        let (stream, mut frames) = tokio::time::timeout(
            STARTUP,
            tokio::task::spawn_blocking(move || open_input(&device)),
        )
        .await
        .map_err(|_| VoiceError::Capture)?
        .map_err(|_| VoiceError::Capture)??;
        tokio::time::timeout(STARTUP, frames.recv())
            .await
            .map_err(|_| VoiceError::Capture)?
            .ok_or(VoiceError::Capture)?;
        if audio.echo_cancellation {
            eprintln!(
                "voice room: Windows echo cancellation unavailable without a render reference"
            );
        }
        Ok(Self {
            _stream: stream,
            frames,
            frame: None,
            source,
            denoiser: audio.rnnoise.then(DenoiseState::new),
            processing: audio
                .automatic_gain_control
                .then(|| AudioProcessingModule::new(false, true, true, false)),
            gain: audio.microphone_volume,
        })
    }

    pub(super) fn clear(&self) {
        self.source.clear_buffer();
    }

    pub(super) async fn read(&mut self) -> Result<(), VoiceError> {
        self.frame = Some(
            tokio::time::timeout(STARTUP, self.frames.recv())
                .await
                .map_err(|_| VoiceError::Capture)?
                .ok_or(VoiceError::Capture)?,
        );
        Ok(())
    }

    pub(super) async fn send_frame(&mut self, muted: bool) -> Result<(), VoiceError> {
        let Some(mut pcm) = self.frame.take() else {
            return Ok(());
        };
        if let Some(processing) = &mut self.processing {
            processing
                .process_stream(&mut pcm, RATE as i32, 1)
                .map_err(|_| VoiceError::Capture)?;
        }
        let input = pcm.map(f32::from);
        let mut filtered = [0.0; SAMPLES];
        let output = if let Some(denoiser) = &mut self.denoiser {
            denoiser.process_frame(&mut filtered, &input);
            filtered
        } else {
            input
        };
        if muted {
            return Ok(());
        }
        let pcm = output.map(|sample| (sample * f32::from(self.gain) / 100.0) as i16);
        self.source
            .capture_frame(&AudioFrame {
                data: pcm.as_slice().into(),
                sample_rate: RATE,
                num_channels: 1,
                samples_per_channel: SAMPLES as u32,
            })
            .await
            .map_err(|_| VoiceError::Capture)
    }
}

fn open_input(id: &str) -> Result<(cpal::Stream, mpsc::Receiver<Pcm>), VoiceError> {
    let device = devices::select(id, true).ok_or(VoiceError::Capture)?;
    let supported = device
        .default_input_config()
        .map_err(|_| VoiceError::Capture)?;
    let config = supported.config();
    let channels = usize::from(config.channels);
    if channels == 0 || config.sample_rate == 0 {
        return Err(VoiceError::Capture);
    }
    let (tx, rx) = mpsc::channel(8);
    let (mut frame, mut offset, mut phase, mut previous) = ([0; SAMPLES], 0, 0_u64, 0.0);
    let rate = u64::from(config.sample_rate);
    let failed = Arc::new(AtomicBool::new(false));
    let on_error = failed.clone();
    let mut push = move |sample: f32| {
        // Linear interpolation with an integer phase keeps 10 ms RTC frames
        // correctly paced even when the endpoint runs at 44.1 kHz.
        phase += u64::from(RATE);
        while phase >= rate {
            phase -= rate;
            let weight = phase as f32 / RATE as f32;
            frame[offset] = ((sample + (previous - sample) * weight) * 32768.0) as i16;
            offset += 1;
            if offset == SAMPLES {
                let _ = tx.try_send(frame);
                offset = 0;
            }
        }
        previous = sample;
    };
    let stream = device
        .build_input_stream_raw(
            config,
            supported.sample_format(),
            move |data, _| {
                if failed.load(Ordering::Relaxed) {
                    return;
                }
                fn downmix<T: cpal::SizedSample>(
                    data: &cpal::Data,
                    channels: usize,
                    mut normalize: impl FnMut(T) -> f32,
                    push: &mut impl FnMut(f32),
                ) {
                    if let Some(samples) = data.as_slice::<T>() {
                        for frame in samples.chunks_exact(channels) {
                            push(
                                frame.iter().copied().map(&mut normalize).sum::<f32>()
                                    / channels as f32,
                            );
                        }
                    }
                }
                match data.sample_format() {
                    cpal::SampleFormat::F32 => downmix(data, channels, |s: f32| s, &mut push),
                    cpal::SampleFormat::I16 => {
                        downmix(data, channels, |s: i16| f32::from(s) / 32768.0, &mut push)
                    }
                    cpal::SampleFormat::I32 => {
                        downmix(data, channels, |s: i32| s as f32 / 2147483648.0, &mut push)
                    }
                    cpal::SampleFormat::U8 => downmix(
                        data,
                        channels,
                        |s: u8| (f32::from(s) - 128.0) / 128.0,
                        &mut push,
                    ),
                    cpal::SampleFormat::I24 => downmix(
                        data,
                        channels,
                        |s: cpal::I24| s.inner() as f32 / 8388608.0,
                        &mut push,
                    ),
                    cpal::SampleFormat::I64 => downmix(
                        data,
                        channels,
                        |s: i64| s as f32 / 9223372036854775808.0,
                        &mut push,
                    ),
                    cpal::SampleFormat::F64 => {
                        downmix(data, channels, |s: f64| s as f32, &mut push)
                    }
                    _ => {}
                }
            },
            move |_| on_error.store(true, Ordering::Relaxed),
            None,
        )
        .map_err(|_| VoiceError::Capture)?;
    stream.play().map_err(|_| VoiceError::Capture)?;
    Ok((stream, rx))
}

struct Output {
    sink: MixerDeviceSink,
    failed: Arc<AtomicBool>,
}

impl Output {
    async fn open(id: String) -> Result<Self, VoiceError> {
        tokio::time::timeout(
            STARTUP,
            tokio::task::spawn_blocking(move || {
                let device = devices::select(&id, false).ok_or(VoiceError::Playback)?;
                let failed = Arc::new(AtomicBool::new(false));
                let on_error = failed.clone();
                let mut sink = DeviceSinkBuilder::from_device(device)
                    .map_err(|_| VoiceError::Playback)?
                    .with_error_callback(move |_| on_error.store(true, Ordering::Relaxed))
                    .open_stream()
                    .map_err(|_| VoiceError::Playback)?;
                sink.log_on_drop(false);
                Ok(Self { sink, failed })
            }),
        )
        .await
        .map_err(|_| VoiceError::Playback)?
        .map_err(|_| VoiceError::Playback)?
    }

    fn stream(&self, channels: u16) -> mpsc::Sender<Vec<f32>> {
        let (tx, frames) = mpsc::channel(8);
        self.sink.mixer().add(PlaybackSource {
            frames,
            samples: Vec::new().into_iter(),
            channels,
            buffering: true,
            silence: 0,
        });
        tx
    }
}

struct PlaybackSource {
    frames: mpsc::Receiver<Vec<f32>>,
    samples: std::vec::IntoIter<f32>,
    channels: u16,
    buffering: bool,
    silence: u16,
}
impl Iterator for PlaybackSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.silence > 0 {
            self.silence -= 1;
            return Some(0.0);
        }
        if let Some(sample) = self.samples.next() {
            return Some(sample);
        }
        if !(self.buffering && self.frames.len() < 3 && !self.frames.is_closed()) {
            match self.frames.try_recv() {
                Ok(frame) => {
                    self.samples = frame.into_iter();
                    self.buffering = false;
                    return self.samples.next();
                }
                Err(mpsc::error::TryRecvError::Disconnected) => return None,
                Err(mpsc::error::TryRecvError::Empty) => self.buffering = true,
            }
        }
        self.silence = self.channels - 1;
        Some(0.0)
    }
}
impl Source for PlaybackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZeroU16 {
        NonZeroU16::new(self.channels).unwrap()
    }
    fn sample_rate(&self) -> NonZeroU32 {
        NonZeroU32::new(RATE).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn scaled(sample: i16, gain: u8, listening: u8) -> f32 {
    (f32::from(sample) * f32::from(gain) * f32::from(listening) / (32768.0 * 10_000.0))
        .clamp(-1.0, 1.0)
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
    let output = Output::open(device).await?;
    let tx = output.stream(channels);
    let mut stream = NativeAudioStream::new(track.rtc_track(), RATE as i32, i32::from(channels));
    let mut health = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = health.tick() => if output.failed.load(Ordering::Relaxed) { return Err(VoiceError::Playback); },
            frame = stream.next() => {
                let Some(frame) = frame else { return Ok(()); };
                if !frame.data.is_empty() {
                    let listening = gain_kind.percent(&listening_gains);
                    let _ = tx.try_send(frame.data.iter().map(|&s| scaled(s, gain, listening)).collect());
                }
            }
        }
    }
}

pub(super) struct SoundboardOutput {
    output: Output,
    frames: mpsc::Sender<Vec<f32>>,
}
impl SoundboardOutput {
    pub(super) async fn open(device: &str) -> Result<Self, SoundboardError> {
        let output = Output::open(device.to_owned())
            .await
            .map_err(|_| SoundboardError::Playback)?;
        let frames = output.stream(1);
        Ok(Self { output, frames })
    }
    pub(super) async fn write(
        &mut self,
        frame: &Pcm,
        listening: u8,
    ) -> Result<(), SoundboardError> {
        if self.output.failed.load(Ordering::Relaxed) {
            return Err(SoundboardError::Playback);
        }
        let _ = self
            .frames
            .try_send(frame.iter().map(|&s| scaled(s, 100, listening)).collect());
        Ok(())
    }
}

pub(super) async fn preview(
    pcm: &[i16],
    volume: u8,
    device: &str,
    gains: &VoiceListeningGains,
) -> Result<(), SoundboardError> {
    let output = Output::open(device.to_owned())
        .await
        .map_err(|_| SoundboardError::Playback)?;
    let player = rodio::Player::connect_new(output.sink.mixer());
    player.append(rodio::buffer::SamplesBuffer::new(
        NonZeroU16::MIN,
        NonZeroU32::new(RATE).unwrap(),
        pcm.iter()
            .map(|&s| scaled(s, volume, gains.soundboard_listening_gain()))
            .collect::<Vec<_>>(),
    ));
    tokio::time::timeout(Duration::from_secs(15), async {
        while !player.empty() {
            if output.failed.load(Ordering::Relaxed) {
                return Err(SoundboardError::Playback);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    })
    .await
    .map_err(|_| SoundboardError::Playback)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_buffers_and_drains_without_blocking() {
        let (tx, frames) = mpsc::channel(8);
        let mut source = PlaybackSource {
            frames,
            samples: Vec::new().into_iter(),
            channels: 2,
            buffering: true,
            silence: 0,
        };
        tx.try_send(vec![0.25, -0.25]).unwrap();
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(source.next(), Some(0.0));
        tx.try_send(vec![0.5, -0.5]).unwrap();
        tx.try_send(vec![1.0, -1.0]).unwrap();
        drop(tx);
        assert_eq!(
            source.collect::<Vec<_>>(),
            vec![0.25, -0.25, 0.5, -0.5, 1.0, -1.0]
        );
    }
}
