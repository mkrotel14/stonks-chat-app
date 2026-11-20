//! WASAPI process loopback, adapted from Tauri's `windows_screen_audio.rs`.
use std::{collections::VecDeque, time::Duration};

use livekit::{
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use livekit_protocol::ParticipantPermission;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};
use windows::Win32::Media::Audio::{IAudioCaptureClient, AUDCLNT_BUFFERFLAGS_SILENT};

use super::{grant_allows_screen_audio, ScreenShareError as Error};

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const FRAME: usize = (RATE / 100 * CHANNELS) as usize;
type Pcm = [i16; FRAME];

pub(super) struct Audio {
    pub track: LocalAudioTrack,
    source: NativeAudioSource,
    frames: mpsc::Receiver<Pcm>,
    worker: JoinHandle<Result<(), String>>,
}

impl Audio {
    /// Call only after screen selection succeeds, using the user's audio opt-in
    /// and the current room grant. Process loopback spans output endpoints.
    pub async fn open_if_requested(
        share_audio: bool,
        permission: Option<&ParticipantPermission>,
    ) -> Result<Option<Self>, Error> {
        if !share_audio {
            return Ok(None);
        }
        if !grant_allows_screen_audio(permission) {
            return Err(Error::AudioPublishDenied);
        }
        let (tx, frames) = mpsc::channel(20);
        let (ready_tx, ready_rx) = oneshot::channel();
        // Dropping the receiver (including on startup cancellation or timeout)
        // signals the worker without blocking the voice room on WASAPI.
        let worker = tokio::task::spawn_blocking(move || {
            let result = capture(&tx, ready_tx);
            if let Err(error) = &result {
                eprintln!("screen audio: isolated WASAPI capture failed: {error}");
            }
            result
        });
        tokio::time::timeout(Duration::from_secs(10), ready_rx)
            .await
            .map_err(|_| Error::AudioUnavailable)?
            .map_err(|_| Error::AudioUnavailable)?;
        let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, CHANNELS, 20);
        let track = LocalAudioTrack::create_audio_track(
            "screen-share-audio",
            RtcAudioSource::Native(source.clone()),
        );
        Ok(Some(Self {
            track,
            source,
            frames,
            worker,
        }))
    }

    /// Drive alongside the screen share's stop/permission lifecycle. Drop the
    /// owner before waiting for unpublish so capture and queued audio stop first.
    pub async fn run(&mut self) -> Result<(), Error> {
        loop {
            let samples = tokio::select! {
                biased;
                _ = &mut self.worker => return Err(Error::AudioCapture),
                samples = self.frames.recv() => samples.ok_or(Error::AudioCapture)?,
            };
            let frame = AudioFrame {
                data: samples.as_slice().into(),
                sample_rate: RATE,
                num_channels: CHANNELS,
                samples_per_channel: RATE / 100,
            };
            tokio::time::timeout(
                Duration::from_millis(250),
                self.source.capture_frame(&frame),
            )
            .await
            .map_err(|_| Error::AudioCapture)?
            .map_err(|_| Error::AudioCapture)?;
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.frames.close();
        self.track.mute();
        self.source.clear_buffer();
    }
}

pub(super) fn publish_options() -> livekit::options::TrackPublishOptions {
    livekit::options::TrackPublishOptions {
        source: TrackSource::ScreenshareAudio,
        dtx: false,
        ..Default::default()
    }
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}

fn capture(frames: &mpsc::Sender<Pcm>, ready: oneshot::Sender<()>) -> Result<(), String> {
    if frames.is_closed() {
        return Ok(());
    }
    wasapi::initialize_mta().ok().map_err(|e| e.to_string())?;
    let _apartment = ComApartment;
    // `false` selects EXCLUDE_TARGET_PROCESS_TREE. Never use endpoint loopback
    // as a fallback: it would include Stonks playback and remote call audio.
    let mut client = AudioClient::new_application_loopback_client(std::process::id(), false)
        .map_err(|e| format!("process loopback activation: {e}"))?;
    let format = WaveFormat::new(
        16,
        16,
        &SampleType::Int,
        RATE as usize,
        CHANNELS as usize,
        None,
    );
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 200_000,
            },
        )
        .map_err(|e| e.to_string())?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    // SAFETY: the client is initialized for capture in this COM apartment.
    let capture = unsafe { client.as_iaudioclient().GetService::<IAudioCaptureClient>() }
        .map_err(|e| e.to_string())?;
    if frames.is_closed() || ready.is_closed() {
        return Ok(());
    }
    client.start_stream().map_err(|e| e.to_string())?;
    let result = (|| {
        if ready.send(()).is_err() {
            return Ok(());
        }
        let mut pending = VecDeque::with_capacity(FRAME * 4);
        while !frames.is_closed() {
            while !frames.is_closed() && capture_packet(&capture, &mut pending)? {
                while pending.len() >= FRAME {
                    let mut pcm = [0; FRAME];
                    for (sample, value) in pcm.iter_mut().zip(pending.drain(..FRAME)) {
                        *sample = value;
                    }
                    match frames.try_send(pcm) {
                        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                    }
                }
            }
            // No packets is normal when every other application is quiet.
            let _ = event.wait_for_event(20);
        }
        Ok(())
    })();
    let _ = client.stop_stream();
    result
}

fn capture_packet(
    capture: &IAudioCaptureClient,
    pending: &mut VecDeque<i16>,
) -> Result<bool, String> {
    // SAFETY: all calls for this capture client happen on the worker thread.
    if unsafe { capture.GetNextPacketSize() }.map_err(|e| e.to_string())? == 0 {
        return Ok(false);
    }
    let (mut data, mut frame_count, mut flags) = (std::ptr::null_mut(), 0, 0);
    // SAFETY: outputs are writable and remain valid for the call. WASAPI owns
    // the returned buffer until ReleaseBuffer on this same thread.
    unsafe { capture.GetBuffer(&mut data, &mut frame_count, &mut flags, None, None) }
        .map_err(|e| e.to_string())?;
    if frame_count == 0 {
        return Ok(false);
    }
    let samples = frame_count as usize * CHANNELS as usize;
    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
    if silent {
        // Silent packets need not have a valid data pointer.
        pending.resize(pending.len() + samples, 0);
    } else if !data.is_null() {
        // SAFETY: non-silent packets contain frame_count frames in the
        // initialized 16-bit stereo format and have not yet been released.
        let bytes = unsafe { std::slice::from_raw_parts(data, samples * 2) };
        pending.extend(
            bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]])),
        );
    }
    // SAFETY: releases the packet acquired above after copying its samples.
    unsafe { capture.ReleaseBuffer(frame_count) }.map_err(|e| e.to_string())?;
    if !silent && data.is_null() {
        return Err("WASAPI returned a missing audio buffer".to_owned());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_capture_without_opt_in_and_publish_grant() {
        let permission = ParticipantPermission {
            can_publish: true,
            ..Default::default()
        };
        assert!(matches!(
            Audio::open_if_requested(false, Some(&permission)).await,
            Ok(None)
        ));
        assert!(matches!(
            Audio::open_if_requested(true, None).await,
            Err(Error::AudioPublishDenied)
        ));
    }
}
