//! Local playback preferences, isolated from Tauri settings and room publication.
use super::Voice;
use crate::{preferences::NativePreferences, runtime::spawn_core};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
use stonks_core::voice::VoiceListeningGains;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Saved {
    microphones: BTreeMap<String, u8>,
    screens: BTreeMap<String, u8>,
    soundboard: u8,
    music: u8,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            microphones: BTreeMap::new(),
            screens: BTreeMap::new(),
            soundboard: 100,
            music: 70,
        }
    }
}

impl Saved {
    fn read(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut saved: Self = serde_json::from_slice(&bytes)?;
                saved
                    .microphones
                    .retain(|id, gain| !id.is_empty() && *gain < 100);
                saved.soundboard = saved.soundboard.min(100);
                saved.music = saved.music.min(100);
                Ok(saved)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    fn write(&self, path: &Path) -> std::io::Result<()> {
        // Do not silently replace an unreadable or malformed settings file.
        Self::read(path)?;
        let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temp, self)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    fn gains(&self) -> VoiceListeningGains {
        let gains = VoiceListeningGains::default();
        for (id, gain) in &self.microphones {
            gains.set_participant_microphone_gain(id, *gain);
        }
        for (id, gain) in &self.screens {
            gains.set_screen_listening_gain(id, *gain);
        }
        gains.set_soundboard_listening_gain(self.soundboard);
        gains.set_music_listening_gain(self.music);
        gains
    }
}

#[derive(Default)]
pub(super) struct Listening {
    saved: Saved,
    pub(super) gains: VoiceListeningGains,
    path: PathBuf,
    pending: bool,
    worker: Option<Task<()>>,
    pub(super) storage_error: bool,
}

impl Listening {
    pub(super) fn load(cx: &App) -> Self {
        let path = cx
            .global::<NativePreferences>()
            .path
            .with_file_name("voice-listening.json");
        let result = Saved::read(&path);
        let storage_error = result.is_err();
        let saved = result.unwrap_or_default();
        Self {
            gains: saved.gains(),
            saved,
            path,
            storage_error,
            ..Self::default()
        }
    }
}

impl Voice {
    pub(super) fn set_microphone_gain(&mut self, id: &str, gain: u8, cx: &mut Context<Self>) {
        let gain = gain.min(100);
        self.listening
            .gains
            .set_participant_microphone_gain(id, gain);
        if gain == 100 {
            self.listening.saved.microphones.remove(id);
        } else {
            self.listening.saved.microphones.insert(id.to_owned(), gain);
        }
        self.save_listening(cx);
    }

    pub(super) fn set_screen_gain(&mut self, id: &str, gain: u8, cx: &mut Context<Self>) {
        self.listening.gains.set_screen_listening_gain(id, gain);
        if gain >= 100 {
            self.listening.saved.screens.remove(id);
        } else {
            self.listening.saved.screens.insert(id.to_owned(), gain);
        }
        self.save_listening(cx);
    }

    pub(super) fn set_soundboard_gain(&mut self, gain: u8, cx: &mut Context<Self>) {
        let gain = gain.min(100);
        self.listening.gains.set_soundboard_listening_gain(gain);
        self.listening.saved.soundboard = gain;
        self.save_listening(cx);
    }

    pub(super) fn set_music_gain(&mut self, gain: u8, cx: &mut Context<Self>) {
        let gain = gain.min(100);
        self.listening.gains.set_music_listening_gain(gain);
        self.listening.saved.music = gain;
        self.save_listening(cx);
    }

    fn save_listening(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.listening.pending = true;
        if self.listening.worker.is_some() {
            return;
        }
        let path = self.listening.path.clone();
        self.listening.worker = Some(cx.spawn(async move |this, cx| loop {
            let Ok(saved) = this.update(cx, |this, _| {
                this.listening.pending = false;
                this.listening.saved.clone()
            }) else {
                break;
            };
            let path = path.clone();
            let saved_ok = spawn_core(async move {
                tokio::task::spawn_blocking(move || saved.write(&path))
                    .await
                    .is_ok_and(|result| result.is_ok())
            })
            .await;
            let Ok(again) = this.update(cx, |this, cx| {
                this.listening.storage_error = !saved_ok;
                cx.notify();
                if this.listening.pending {
                    true
                } else {
                    this.listening.worker = None;
                    false
                }
            }) else {
                break;
            };
            if !again {
                break;
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::Saved;

    #[test]
    fn round_trip_seeds_independent_gains_and_preserves_outgoing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice-listening.json");
        let store = stonks_core::soundboard::SoundboardStore::new(dir.path(), dir.path());
        store.set_volume(65).unwrap();
        let mut saved = Saved::read(&path).unwrap();
        assert_eq!(saved.gains().soundboard_listening_gain(), 100);
        saved.microphones.insert("alice".into(), 0);
        saved.screens.insert("alice".into(), 40);
        assert_eq!(saved.gains().music_listening_gain(), 70);
        saved.soundboard = 0;
        saved.music = 28;
        saved.write(&path).unwrap();
        let gains = Saved::read(&path).unwrap().gains();
        assert_eq!(gains.participant_microphone_gain("alice"), 0);
        assert_eq!(gains.screen_listening_gain("alice"), 40);
        assert_eq!(gains.participant_microphone_gain("bob"), 100);
        assert_eq!(gains.soundboard_listening_gain(), 0);
        assert_eq!(gains.music_listening_gain(), 28);
        assert_eq!(store.settings().unwrap().soundboard_volume, 65);
    }

    #[test]
    fn malformed_preferences_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice-listening.json");
        std::fs::write(&path, b"bad json").unwrap();
        assert!(Saved::default().write(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"bad json");
    }
}
