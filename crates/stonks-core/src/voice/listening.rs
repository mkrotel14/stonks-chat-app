use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, RwLock,
    },
};

use livekit::prelude::TrackSource;

pub(super) const SOUNDBOARD_TRACK: &str = "soundboard";

/// Local-only listening preferences. Clones share live settings; use one handle
/// per app session and reuse it across room joins if preferences should persist.
/// All settings default to 100% and setters clamp to 0–100%.
#[derive(Clone)]
pub struct VoiceListeningGains(Arc<RwLock<Gains>>, Arc<AtomicU8>);

impl Default for VoiceListeningGains {
    fn default() -> Self {
        Self(Arc::default(), Arc::new(AtomicU8::new(100)))
    }
}

struct Gains {
    screens: HashMap<String, u8>,
    microphones: HashMap<String, u8>,
    soundboard: u8,
}

impl Default for Gains {
    fn default() -> Self {
        Self {
            screens: HashMap::new(),
            microphones: HashMap::new(),
            soundboard: 100,
        }
    }
}

impl VoiceListeningGains {
    pub fn set_music_listening_gain(&self, percent: u8) {
        self.1.store(percent.min(100), Ordering::Relaxed);
    }

    pub fn music_listening_gain(&self) -> u8 {
        self.1.load(Ordering::Relaxed)
    }

    pub(super) fn music_gain_handle(&self) -> Arc<AtomicU8> {
        self.1.clone()
    }

    /// Set the microphone gain for the exact LiveKit participant identity.
    /// Setting 100 restores the default and removes the override.
    pub fn set_participant_microphone_gain(&self, identity: &str, percent: u8) {
        let mut gains = self.0.write().unwrap_or_else(|error| error.into_inner());
        if percent >= 100 {
            gains.microphones.remove(identity);
        } else {
            gains.microphones.insert(identity.to_owned(), percent);
        }
    }

    pub fn participant_microphone_gain(&self, identity: &str) -> u8 {
        let gains = self.0.read().unwrap_or_else(|error| error.into_inner());
        gains.microphones.get(identity).copied().unwrap_or(100)
    }

    /// Set listener-only gain for one remote screen-audio publisher.
    pub fn set_screen_listening_gain(&self, identity: &str, percent: u8) {
        let mut gains = self.0.write().unwrap_or_else(|error| error.into_inner());
        if percent >= 100 {
            gains.screens.remove(identity);
        } else {
            gains.screens.insert(identity.to_owned(), percent);
        }
    }

    pub fn screen_listening_gain(&self, identity: &str) -> u8 {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .screens
            .get(identity)
            .copied()
            .unwrap_or(100)
    }

    /// Set local soundboard playback gain, without changing published audio.
    pub fn set_soundboard_listening_gain(&self, percent: u8) {
        self.0
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .soundboard = percent.min(100);
    }

    pub fn soundboard_listening_gain(&self) -> u8 {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .soundboard
    }
}

pub(super) enum RemoteGain {
    Microphone(String),
    Screen(String),
    Soundboard,
    Music,
    Unchanged,
}

impl RemoteGain {
    pub(super) fn for_track(name: &str, source: TrackSource, identity: &str) -> Self {
        match (name, source) {
            (_, TrackSource::ScreenshareAudio) => Self::Screen(identity.to_owned()),
            ("music", _) => Self::Music,
            (SOUNDBOARD_TRACK, _) => Self::Soundboard,
            (_, TrackSource::Microphone) => Self::Microphone(identity.to_owned()),
            _ => Self::Unchanged,
        }
    }

    pub(super) fn percent(&self, gains: &VoiceListeningGains) -> u8 {
        match self {
            Self::Microphone(identity) => gains.participant_microphone_gain(identity),
            Self::Screen(identity) => gains.screen_listening_gain(identity),
            Self::Soundboard => gains.soundboard_listening_gain(),
            Self::Music => gains.music_listening_gain(),
            Self::Unchanged => 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_gains_are_isolated_by_identity_and_track_kind() {
        let gains = VoiceListeningGains::default();
        let worker = gains.clone();
        let alice = RemoteGain::for_track("microphone", TrackSource::Microphone, "alice");
        let bob = RemoteGain::for_track("microphone", TrackSource::Microphone, "bob");
        let sound = RemoteGain::for_track(SOUNDBOARD_TRACK, TrackSource::Microphone, "alice");
        let music = RemoteGain::for_track("music", TrackSource::Microphone, "alice");
        let screen = RemoteGain::for_track("music", TrackSource::ScreenshareAudio, "alice");
        let other = [RemoteGain::for_track(
            "other",
            TrackSource::Unknown,
            "alice",
        )];
        assert_eq!(alice.percent(&worker), 100);
        assert_eq!(sound.percent(&worker), 100);
        gains.set_participant_microphone_gain("alice", 0);
        gains.set_soundboard_listening_gain(35);
        gains.set_music_listening_gain(22);
        assert_eq!(screen.percent(&worker), 100);
        gains.set_screen_listening_gain("alice", 0);
        assert_eq!(screen.percent(&worker), 0);
        assert_eq!(gains.screen_listening_gain("bob"), 100);
        assert_eq!(music.percent(&worker), 22);
        assert_eq!(alice.percent(&worker), 0);
        assert_eq!(bob.percent(&worker), 100);
        assert_eq!(sound.percent(&worker), 35);
        assert!(other.iter().all(|track| track.percent(&worker) == 100));
        gains.set_participant_microphone_gain("alice", 100);
        gains.set_soundboard_listening_gain(255);
        assert_eq!(alice.percent(&worker), 100);
        assert_eq!(sound.percent(&worker), 100);
    }
}
