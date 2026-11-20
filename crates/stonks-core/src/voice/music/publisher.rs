use super::*;
use std::{borrow::Cow, collections::VecDeque, time::Duration};

pub(super) const MUSIC_FRAME_SAMPLES: usize = 480;

pub(in crate::voice) enum MusicPcmCommand {
    Samples(Vec<i16>),
    Flush,
}

pub(in crate::voice) fn native_music_track() -> (
    LocalAudioTrack,
    NativeAudioSource,
    mpsc::Sender<MusicPcmCommand>,
    mpsc::Receiver<MusicPcmCommand>,
) {
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
    let track =
        LocalAudioTrack::create_audio_track("music", RtcAudioSource::Native(source.clone()));
    let (sender, incoming) = mpsc::channel::<MusicPcmCommand>(10);
    (track, source, sender, incoming)
}

pub(super) async fn capture(
    source: NativeAudioSource,
    mut incoming: mpsc::Receiver<MusicPcmCommand>,
) {
    let mut queued = VecDeque::<i16>::new();
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        while let Ok(command) = incoming.try_recv() {
            match command {
                MusicPcmCommand::Samples(frame) => {
                    // Limit latency if the decoder briefly outruns WebRTC.
                    let excess =
                        (queued.len() + frame.len()).saturating_sub(10 * MUSIC_FRAME_SAMPLES);
                    queued.drain(..excess.min(queued.len()));
                    queued.extend(frame);
                }
                MusicPcmCommand::Flush => {
                    queued.clear();
                    source.clear_buffer();
                }
            }
        }
        let mut pcm = [0_i16; MUSIC_FRAME_SAMPLES];
        for sample in &mut pcm {
            *sample = queued.pop_front().unwrap_or(0);
        }
        let frame = AudioFrame {
            data: Cow::Borrowed(&pcm),
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: MUSIC_FRAME_SAMPLES as u32,
        };
        if source.capture_frame(&frame).await.is_err() {
            break;
        }
    }
}
