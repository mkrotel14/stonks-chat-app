use std::time::Duration;

use livekit::{
    options::TrackPublishOptions,
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit};

use crate::soundboard::{
    validate_volume, SoundboardError as Error, SoundboardStore, VOICE_SAMPLE_RATE as SAMPLE_RATE,
};

use super::{
    listening::SOUNDBOARD_TRACK, room::grant_allows_microphone, VoiceListeningGains, VoicePhase,
    VoiceSnapshot,
};

const FRAME: usize = SAMPLE_RATE as usize / 100;
const FRAME_PERIOD: Duration = Duration::from_millis(10);
const SUBSCRIBE_DELAY: Duration = Duration::from_millis(500);
const SOURCE_QUEUE_MS: u32 = 100;
const DRAIN_FRAMES: usize = SOURCE_QUEUE_MS as usize / 10;
const FADE: usize = FRAME / 2; // 5 ms, including a zero at each clip boundary.

fn fade(index: usize) -> f32 {
    index.min(FADE - 1) as f32 / (FADE - 1) as f32
}

fn limit(sample: f32) -> i16 {
    // A smooth, symmetric knee leaves ordinary mixes alone and avoids flat
    // tops on loud overlaps without changing gain at 10 ms frame boundaries.
    const KNEE: f32 = 30_000.0;
    const HEADROOM: f32 = i16::MAX as f32 - KNEE;
    let over = (sample.abs() - KNEE).max(0.0);
    let compressed = sample.abs().min(KNEE) + HEADROOM * over / (HEADROOM + over);
    compressed.copysign(sample).round() as i16
}

/// Bounds decoded clips across loading, queued playback, and the mixer.
pub const SOUNDBOARD_CAPACITY: usize = 8;

#[derive(Clone, Debug)]
pub struct JoinSoundOptions {
    pub store: SoundboardStore,
    pub suppress_join_sound: bool,
}

pub(super) struct Play {
    pub id: String,
    pub pcm: Vec<i16>,
    pub reply: Option<oneshot::Sender<Result<(), Error>>>,
    pub _permit: OwnedSemaphorePermit,
}

impl Play {
    fn cancelled(&self) -> bool {
        self.reply.as_ref().is_some_and(|reply| reply.is_closed())
    }

    fn finish(self, result: Result<(), Error>) {
        if let Some(reply) = self.reply {
            let _ = reply.send(result);
        }
    }
}

struct Clip {
    play: Play,
    offset: usize,
    drain: usize,
    result: Result<(), Error>,
}

impl Clip {
    fn mix(&self, out: &mut [f32], release: bool) {
        let len = self.play.pcm.len();
        // Short clips retain a peak rather than being attenuated twice over a
        // full 5 ms window. A single sample has no click-free audible form.
        let edge = FADE.min(len.div_ceil(2)).max(2) - 1;
        for (index, (out, &sample)) in out
            .iter_mut()
            .zip(&self.play.pcm[self.offset..])
            .enumerate()
        {
            let position = self.offset + index;
            let envelope = position.min(len - 1 - position).min(edge) as f32 / edge as f32;
            let release = if release { fade(FADE - 1 - index) } else { 1.0 };
            *out += f32::from(sample) * envelope * release;
        }
    }
}

struct Mixer(Vec<Clip>, [f32; FRAME]);

impl Default for Mixer {
    fn default() -> Self {
        Self(Vec::new(), [0.0; FRAME])
    }
}

impl Mixer {
    fn insert(&mut self, play: Play) {
        if let Some(index) = self.0.iter().position(|clip| clip.play.id == play.id) {
            let old = self.0.swap_remove(index);
            if old.offset != 0 {
                old.mix(&mut self.1[..FADE], true);
            }
            old.play.finish(old.result);
        }
        self.0.push(Clip {
            play,
            offset: 0,
            drain: DRAIN_FRAMES,
            result: Ok(()),
        });
    }

    fn frame(&mut self) -> [i16; FRAME] {
        let mut sum = std::mem::replace(&mut self.1, [0.0; FRAME]);
        for clip in &mut self.0 {
            let end = (clip.offset + FRAME).min(clip.play.pcm.len());
            clip.mix(&mut sum, false);
            if clip.offset == end {
                clip.drain = clip.drain.saturating_sub(1);
            }
            clip.offset = end;
        }
        sum.map(limit)
    }

    fn complete_frame(&mut self, output: Result<(), Error>) {
        let mut index = 0;
        while index < self.0.len() {
            let clip = &mut self.0[index];
            clip.result = clip.result.and(output);
            if clip.drain == 0 {
                let clip = self.0.swap_remove(index);
                clip.play.finish(clip.result);
            } else {
                index += 1;
            }
        }
    }

    fn remove_cancelled(&mut self) {
        self.0.retain(|clip| {
            if !clip.play.cancelled() {
                return true;
            }
            if clip.offset != 0 {
                clip.mix(&mut self.1[..FADE], true);
            }
            false
        });
    }

    fn fail(&mut self, error: Error) {
        self.1.fill(0.0);
        for clip in self.0.drain(..) {
            clip.play.finish(Err(error));
        }
    }
}

pub(super) async fn run(
    room: std::sync::Arc<Room>,
    mut state: watch::Receiver<VoiceSnapshot>,
    mut inbox: mpsc::Receiver<Play>,
    join_sound: Option<(JoinSoundOptions, OwnedSemaphorePermit)>,
    output_device: String,
    listening_gains: VoiceListeningGains,
) {
    let mut source: Option<Published> = None;
    // The local sink lives with the published track; dropping it kills pw-cat
    // and discards its buffered audio on deafen or room teardown.
    let mut local = Ok(None);
    let mut mixer = Mixer::default();
    if let Some((options, permit)) = join_sound.filter(|_| allowed(&room, &state).is_ok()) {
        let selected = tokio::task::spawn_blocking(move || {
            options
                .store
                .join_pcm()
                .map(|selection| (selection, permit))
        })
        .await;
        if let Ok(Ok((Some((id, pcm)), permit))) = selected {
            if allowed(&room, &state).is_ok() {
                mixer.insert(Play {
                    id,
                    pcm,
                    reply: None,
                    _permit: permit,
                });
            }
        }
    }
    loop {
        if let Err(error) = allowed(&room, &state) {
            mixer.fail(error);
            local = Ok(None);
            if let Some(published) = &source {
                published.source.clear_buffer();
            }
        }
        mixer.remove_cancelled();
        if mixer.0.is_empty() && local.is_err() {
            // Retry a failed sink on the next playback burst, not every frame.
            local = Ok(None);
        }
        if source.is_none() && !mixer.0.is_empty() {
            // Finish publication even if a request goes away; otherwise we could
            // leave a server-side track without its local pump.
            match publish(&room).await {
                Ok(published) => source = Some(published),
                Err(error) => mixer.fail(error),
            }
            continue;
        }
        tokio::select! {
            biased;
            changed = state.changed() => if changed.is_err() { break; },
            _ = async {
                if let Some(published) = &mut source {
                    published.clock.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                let Some(published) = &source else { continue; };
                // Drain before mixing so pending clips affect this frame, not the
                // end of the previous clip. The semaphore also bounds this batch.
                for _ in 0..SOUNDBOARD_CAPACITY {
                    let Ok(play) = inbox.try_recv() else { break; };
                    admit(&room, &state, &mut mixer, play);
                }
                let playing = !mixer.0.is_empty()
                    && tokio::time::Instant::now() >= published.ready_at;
                let frame = if tokio::time::Instant::now() >= published.ready_at {
                    mixer.frame()
                } else {
                    [0; FRAME]
                };
                let outputs = async {
                    tokio::join!(capture(&published.source, &frame), async {
                        // Once opened, keep feeding silence between bursts, as
                        // with the LiveKit source. Do not open a sink for silence.
                        if playing || matches!(local, Ok(Some(_))) {
                            let result = write_local(&mut local, &output_device, &frame, listening_gains.soundboard_listening_gain()).await;
                            if let Err(error) = result {
                                local = Err(error);
                            }
                            result
                        } else {
                            Ok(())
                        }
                    })
                };
                tokio::select! {
                    biased;
                    error = wait_until_denied(&room, &mut state) => {
                        published.source.clear_buffer();
                        local = Ok(None);
                        mixer.fail(error);
                    }
                    (remote, local_result) = outputs => {
                        if remote.is_err() {
                            published.source.clear_buffer();
                        }
                        mixer.complete_frame(remote.and(local_result));
                    }
                }
            }
            play = inbox.recv() => {
                let Some(play) = play else { break; };
                admit(&room, &state, &mut mixer, play);
            }
        }
    }
}

async fn write_local(
    local: &mut Result<Option<super::audio::SoundboardOutput>, Error>,
    device: &str,
    frame: &[i16; FRAME],
    listening: u8,
) -> Result<(), Error> {
    let sink = local.as_mut().map_err(|error| *error)?;
    if sink.is_none() {
        *sink = Some(super::audio::SoundboardOutput::open(device).await?);
    }
    sink.as_mut()
        .ok_or(Error::Playback)?
        .write(frame, listening)
        .await
}

async fn wait_until_denied(room: &Room, state: &mut watch::Receiver<VoiceSnapshot>) -> Error {
    loop {
        if let Err(error) = allowed(room, state) {
            return error;
        }
        if state.changed().await.is_err() {
            return Error::Disconnected;
        }
    }
}

fn admit(room: &Room, state: &watch::Receiver<VoiceSnapshot>, mixer: &mut Mixer, play: Play) {
    if !play.cancelled() {
        match allowed(room, state) {
            Ok(()) => mixer.insert(play),
            Err(error) => play.finish(Err(error)),
        }
    }
}

fn allowed(room: &Room, state: &watch::Receiver<VoiceSnapshot>) -> Result<(), Error> {
    let state = state.borrow();
    check_state(&state)?;
    if !grant_allows_microphone(room.local_participant().permission().as_ref()) {
        return Err(Error::SpeakDenied);
    }
    Ok(())
}

pub(super) fn check_state(state: &VoiceSnapshot) -> Result<(), Error> {
    if state.phase != VoicePhase::Connected {
        return Err(Error::Disconnected);
    }
    if !state.can_speak || state.deafened {
        return Err(Error::SpeakDenied);
    }
    Ok(())
}

struct Published {
    source: NativeAudioSource,
    clock: tokio::time::Interval,
    ready_at: tokio::time::Instant,
}

impl Drop for Published {
    fn drop(&mut self) {
        self.source.clear_buffer();
    }
}

async fn publish(room: &Room) -> Result<Published, Error> {
    let source = NativeAudioSource::new(
        AudioSourceOptions::default(),
        SAMPLE_RATE,
        1,
        SOURCE_QUEUE_MS,
    );
    let track = LocalAudioTrack::create_audio_track(
        SOUNDBOARD_TRACK,
        RtcAudioSource::Native(source.clone()),
    );
    tokio::time::timeout(
        Duration::from_secs(8),
        room.local_participant().publish_track(
            LocalTrack::Audio(track),
            TrackPublishOptions {
                // Match Tauri's routing name and the server's Speak grant.
                source: TrackSource::Microphone,
                ..Default::default()
            },
        ),
    )
    .await
    .map_err(|_| Error::Playback)?
    .map_err(|_| Error::Playback)?;
    let mut clock = tokio::time::interval(FRAME_PERIOD);
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    Ok(Published {
        source,
        clock,
        // Publication completes before remote subscribers are necessarily ready.
        ready_at: tokio::time::Instant::now() + SUBSCRIBE_DELAY,
    })
}

async fn capture(source: &NativeAudioSource, frame: &[i16; FRAME]) -> Result<(), Error> {
    tokio::time::timeout(
        Duration::from_millis(250),
        source.capture_frame(&AudioFrame {
            data: frame.as_slice().into(),
            sample_rate: SAMPLE_RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        }),
    )
    .await
    .map_err(|_| Error::Playback)?
    .map_err(|_| Error::Playback)
}

/// Play on the local audio output only. Await completion or drop the future to stop it.
pub async fn preview_soundboard_clip(
    store: &SoundboardStore,
    id: &str,
    volume: u8,
    output_device: &str,
) -> Result<(), Error> {
    VoiceListeningGains::default()
        .preview_soundboard_clip(store, id, volume, output_device)
        .await
}

impl VoiceListeningGains {
    /// Preview locally using this handle's listening gain, even outside a room.
    pub async fn preview_soundboard_clip(
        &self,
        store: &SoundboardStore,
        id: &str,
        volume: u8,
        output_device: &str,
    ) -> Result<(), Error> {
        validate_volume(volume)?;
        let (store, id) = (store.clone(), id.to_owned());
        let pcm = tokio::task::spawn_blocking(move || store.pcm(&id))
            .await
            .map_err(|_| Error::Playback)??;
        super::audio::preview(&pcm, volume, output_device, self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soundboard_is_independent_of_microphone_mute_but_gated_by_room_state() {
        let mut state = VoiceSnapshot {
            channel_id: "voice".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: true,
            deafened: false,
            error: None,
            active_speakers: Default::default(),
        };
        assert_eq!(check_state(&state), Ok(()));
        state.deafened = true;
        assert_eq!(check_state(&state), Err(Error::SpeakDenied));
        state.deafened = false;
        state.can_speak = false;
        assert_eq!(check_state(&state), Err(Error::SpeakDenied));
        state.phase = VoicePhase::Reconnecting;
        assert_eq!(check_state(&state), Err(Error::Disconnected));
    }

    fn request(
        slots: &std::sync::Arc<tokio::sync::Semaphore>,
        id: &str,
        pcm: Vec<i16>,
    ) -> (Play, oneshot::Receiver<Result<(), Error>>) {
        let (reply, done) = oneshot::channel();
        (
            Play {
                id: id.into(),
                pcm,
                reply: Some(reply),
                _permit: slots.clone().try_acquire_owned().unwrap(),
            },
            done,
        )
    }

    #[tokio::test]
    async fn second_clip_reaches_the_same_webrtc_track_before_the_first_finishes() {
        use futures_util::StreamExt;
        use livekit::webrtc::audio_stream::native::NativeAudioStream;

        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(SOUNDBOARD_CAPACITY));
        let source = NativeAudioSource::new(
            AudioSourceOptions::default(),
            SAMPLE_RATE,
            1,
            SOURCE_QUEUE_MS,
        );
        let track = LocalAudioTrack::create_audio_track(
            "soundboard",
            RtcAudioSource::Native(source.clone()),
        );
        let mut stream = NativeAudioStream::new(track.rtc_track(), SAMPLE_RATE as i32, 1);
        let mut clock = tokio::time::interval(FRAME_PERIOD);
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut mixer = Mixer::default();
        let (first, _first_done) = request(&slots, "first", vec![2_000; FRAME * 5]);
        let (second, _second_done) = request(&slots, "second", vec![3_000; FRAME * 2]);
        let mut second_for_test = Some(second);
        mixer.insert(first);
        let sent = async {
            for tick in 0..(5 + DRAIN_FRAMES) {
                clock.tick().await;
                if tick == 1 {
                    mixer.insert(second_for_test.take().unwrap());
                }
                capture(&source, &mixer.frame()).await.unwrap();
                mixer.complete_frame(Ok(()));
            }
        };
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            let mut frames = Vec::new();
            while let Some(frame) = stream.next().await {
                assert_eq!(frame.data.len(), FRAME);
                let sample = frame.data[FADE];
                if sample != 0 {
                    frames.push(sample);
                    if frames.len() == 5 {
                        break;
                    }
                }
            }
            frames
        });
        let (_, observed) = tokio::join!(sent, observed);
        assert_eq!(observed.unwrap(), [2_000, 5_000, 5_000, 2_000, 2_000]);
    }

    #[test]
    fn restart_replaces_the_same_id_and_capacity_is_released_on_denial() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(SOUNDBOARD_CAPACITY));
        let mut mixer = Mixer::default();
        let (original, mut original_done) = request(&slots, "repeat", vec![20_000; FRAME * 3]);
        mixer.insert(original);
        let first = mixer.frame();
        assert_eq!(first[0], 0);
        assert_eq!(first[FRAME - 1], 20_000);
        let (restart, mut restart_done) = request(&slots, "repeat", vec![-20_000; FRAME * 2]);
        mixer.insert(restart);
        assert_eq!(original_done.try_recv(), Ok(Ok(())));
        let crossfade = mixer.frame();
        assert_eq!(crossfade[0], first[FRAME - 1]);
        assert_eq!(crossfade[FADE - 1], -20_000);
        assert!(crossfade
            .windows(2)
            .all(|pair| (i32::from(pair[1]) - i32::from(pair[0])).abs() <= 168));
        let other: Vec<_> = (1..SOUNDBOARD_CAPACITY)
            .map(|index| {
                let (play, done) = request(&slots, &index.to_string(), vec![0; FRAME]);
                mixer.insert(play);
                done
            })
            .collect();
        assert!(slots.try_acquire().is_err());
        mixer.fail(Error::SpeakDenied);
        assert_eq!(restart_done.try_recv(), Ok(Err(Error::SpeakDenied)));
        assert_eq!(slots.available_permits(), SOUNDBOARD_CAPACITY);
        drop(other);
    }

    #[test]
    fn cancelling_one_request_leaves_the_other_playing() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(SOUNDBOARD_CAPACITY));
        let mut mixer = Mixer::default();
        let (cancelled, cancel) = request(&slots, "cancelled", vec![1_000; FRAME * 2]);
        let (survivor, mut done) = request(&slots, "survivor", vec![2_000; FRAME * 2]);
        mixer.insert(cancelled);
        mixer.insert(survivor);
        assert_eq!(mixer.frame()[FADE], 3_000);
        mixer.complete_frame(Ok(()));
        drop(cancel);
        mixer.remove_cancelled();
        assert_eq!(slots.available_permits(), SOUNDBOARD_CAPACITY - 1);
        let tail = mixer.frame();
        assert_eq!(tail[0], 3_000);
        assert_eq!(tail[FADE - 1], 2_000);
        assert_eq!(tail[FRAME - 1], 0);
        mixer.complete_frame(Ok(()));
        assert!(done.try_recv().is_err());
        for _ in 0..DRAIN_FRAMES {
            assert_eq!(mixer.frame(), [0; FRAME]);
            mixer.complete_frame(Ok(()));
        }
        assert_eq!(done.try_recv(), Ok(Ok(())));
        assert_eq!(slots.available_permits(), SOUNDBOARD_CAPACITY);
    }

    #[test]
    fn loud_overlap_preserves_peak_shape_and_partial_frame_fades_to_silence() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(SOUNDBOARD_CAPACITY));
        let mut mixer = Mixer::default();
        for (id, pcm) in [
            (
                "a",
                [
                    vec![0; FADE],
                    vec![32_000, 30_000, -32_000, 1000],
                    vec![1000; FRAME],
                ]
                .concat(),
            ),
            (
                "b",
                [
                    vec![0; FADE],
                    vec![16_000, 15_000, -16_000, 500],
                    vec![500; FRAME],
                ]
                .concat(),
            ),
        ] {
            mixer.insert(request(&slots, id, pcm).0);
        }
        let frame = mixer.frame();
        let peak = frame[FADE];
        assert!(30_000 < frame[FADE + 1] && frame[FADE + 1] < peak && peak < i16::MAX);
        assert_eq!(frame[FADE + 2], -peak);
        assert_eq!(frame[FADE + 3], 1500);
        let tail = mixer.frame();
        assert_eq!(tail[FADE + 2], 6);
        assert!(tail[FADE + 3..].iter().all(|&sample| sample == 0));
    }
}
