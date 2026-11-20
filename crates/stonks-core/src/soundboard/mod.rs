//! Local soundboard library and join selection, compatible with Tauri's on-disk layout.
mod pcm;
#[cfg(all(
    feature = "native-voice",
    any(target_os = "linux", target_os = "windows")
))]
pub(crate) use pcm::SAMPLE_RATE as VOICE_SAMPLE_RATE;
use pcm::{decode, SAMPLE_RATE};

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_BYTES: u64 = 25 * 1024 * 1024;
const MAX_SECONDS: usize = 10;
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundboardClip {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SoundboardSettings {
    pub soundboard_volume: u8,
    pub join_soundboard_clip_id: Option<String>,
}

impl Default for SoundboardSettings {
    fn default() -> Self {
        Self {
            soundboard_volume: 20,
            join_soundboard_clip_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundboardError {
    InvalidId,
    InvalidName,
    InvalidVolume,
    TooLarge,
    TooLong,
    InvalidAudio,
    NotFound,
    InvalidCatalog,
    InvalidPreferences,
    Storage,
    Playback,
    Busy,
    Disconnected,
    SpeakDenied,
}

impl std::fmt::Display for SoundboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "The sound ID is invalid.",
            Self::InvalidName => "Sound names must contain 1 to 64 characters.",
            Self::InvalidVolume => "The soundboard volume must be between 0 and 100.",
            Self::TooLarge => "The audio file is larger than 25 MB.",
            Self::TooLong => "Soundboard clips must be 10 seconds or shorter.",
            Self::InvalidAudio => "The sound could not be decoded.",
            Self::NotFound => "Sound not found.",
            Self::InvalidCatalog => "The soundboard library data is invalid.",
            Self::InvalidPreferences => "The saved soundboard settings are invalid.",
            Self::Storage => "The soundboard storage is unavailable.",
            Self::Playback => "The sound could not be played.",
            Self::Busy => "Another sound is already playing.",
            Self::Disconnected => "Join a voice channel first.",
            Self::SpeakDenied => "Soundboard audio is unavailable in this channel.",
        })
    }
}
impl std::error::Error for SoundboardError {}
use SoundboardError as Error;

/// Both paths are already application-specific (Tauri's app data and config dirs).
/// File operations are synchronous; call them off the GUI thread.
#[derive(Clone, Debug)]
pub struct SoundboardStore {
    root: PathBuf,
    preferences: PathBuf,
}

impl SoundboardStore {
    pub fn new(app_data_dir: &Path, app_config_dir: &Path) -> Self {
        Self {
            root: app_data_dir.join("soundboard"),
            preferences: crate::preferences::path_in(app_config_dir),
        }
    }

    pub fn list(&self) -> Result<Vec<SoundboardClip>, Error> {
        let Some(bytes) = read_optional(&self.root.join("clips.json"))? else {
            return Ok(Vec::new());
        };
        let clips: Vec<SoundboardClip> =
            serde_json::from_slice(&bytes).map_err(|_| Error::InvalidCatalog)?;
        let mut ids = std::collections::HashSet::new();
        let mut files = std::collections::HashSet::new();
        if clips.iter().any(|clip| {
            !valid_id(&clip.id)
                || !valid_file_name(&clip.file_name)
                || !ids.insert(&clip.id)
                || !files.insert(&clip.file_name)
        }) {
            return Err(Error::InvalidCatalog);
        }
        Ok(clips)
    }

    pub fn import(&self, source: &Path) -> Result<SoundboardClip, Error> {
        let name = name(
            source
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Sound"),
        )?;
        let parent = self.root.join("clips");
        fs::create_dir_all(&parent).map_err(|_| Error::Storage)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 10
                    && value.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .unwrap_or("audio")
            .to_ascii_lowercase();
        let mut staged = tempfile::Builder::new()
            .prefix("clip-")
            .rand_bytes(16)
            .suffix(&format!(".{extension}"))
            .tempfile_in(&parent)
            .map_err(|_| Error::Storage)?;
        use io::Read;
        let input = fs::File::open(source).map_err(|_| Error::Storage)?;
        if !input.metadata().map_err(|_| Error::Storage)?.is_file() {
            return Err(Error::InvalidAudio);
        }
        let size = io::copy(&mut input.take(MAX_BYTES + 1), staged.as_file_mut())
            .map_err(|_| Error::Storage)?;
        if size > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        let pcm = decode(staged.path())?;
        let clip = SoundboardClip {
            id: staged
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or(Error::Storage)?
                .to_owned(),
            file_name: staged
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(Error::Storage)?
                .to_owned(),
            name,
            duration_ms: pcm.len() as u64 * 1000 / u64::from(SAMPLE_RATE),
        };
        let _guard = WRITES.lock().map_err(|_| Error::Storage)?;
        let mut clips = self.list()?;
        clips.push(clip.clone());
        let (_, path) = staged.keep().map_err(|_| Error::Storage)?;
        if let Err(error) = self.write_catalog(&clips) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(clip)
    }

    pub fn rename(&self, id: &str, new_name: &str) -> Result<Vec<SoundboardClip>, Error> {
        validate_id(id)?;
        let _guard = WRITES.lock().map_err(|_| Error::Storage)?;
        let mut clips = self.list()?;
        clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .ok_or(Error::NotFound)?
            .name = name(new_name)?;
        self.write_catalog(&clips)?;
        Ok(clips)
    }

    /// An existing selection may become stale. `settings` preserves it; auto-play skips it.
    pub fn delete(&self, id: &str) -> Result<Vec<SoundboardClip>, Error> {
        validate_id(id)?;
        let _guard = WRITES.lock().map_err(|_| Error::Storage)?;
        let mut clips = self.list()?;
        let index = clips
            .iter()
            .position(|clip| clip.id == id)
            .ok_or(Error::NotFound)?;
        let path = self.clip_path(&clips.remove(index));
        self.write_catalog(&clips)?;
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Storage),
        }
        Ok(clips)
    }

    pub fn settings(&self) -> Result<SoundboardSettings, Error> {
        let settings: SoundboardSettings = serde_json::from_value(self.preferences_json()?)
            .map_err(|_| Error::InvalidPreferences)?;
        if settings.soundboard_volume > 100
            || settings
                .join_soundboard_clip_id
                .as_deref()
                .is_some_and(|id| !valid_id(id))
        {
            return Err(Error::InvalidPreferences);
        }
        Ok(settings)
    }

    /// Read the join clip without interpreting the legacy publication volume.
    pub fn join_clip_id(&self) -> Result<Option<String>, Error> {
        let mut preferences = self.preferences_json()?;
        let id: Option<String> = serde_json::from_value(preferences["joinSoundboardClipId"].take())
            .map_err(|_| Error::InvalidPreferences)?;
        if id.as_deref().is_some_and(|id| !valid_id(id)) {
            return Err(Error::InvalidPreferences);
        }
        Ok(id)
    }

    /// `None` clears the selection. A new selection must name a current catalog entry.
    pub fn set_join_clip(&self, id: Option<&str>) -> Result<(), Error> {
        let _guard = WRITES.lock().map_err(|_| Error::Storage)?;
        if let Some(id) = id {
            self.find(id)?;
        }
        self.write_preference("joinSoundboardClipId", serde_json::json!(id))
    }

    pub fn set_volume(&self, volume: u8) -> Result<(), Error> {
        validate_volume(volume)?;
        let _guard = WRITES.lock().map_err(|_| Error::Storage)?;
        self.write_preference("soundboardVolume", serde_json::json!(volume))
    }

    #[cfg(all(
        feature = "native-voice",
        any(target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn pcm(&self, id: &str) -> Result<Vec<i16>, Error> {
        decode(&self.clip_path(&self.find(id)?))
    }

    #[cfg(all(
        feature = "native-voice",
        any(target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn join_pcm(&self) -> Result<Option<(String, Vec<i16>)>, Error> {
        let Some(id) = self.join_clip_id()? else {
            return Ok(None);
        };
        match self.pcm(&id) {
            Ok(pcm) => Ok(Some((id, pcm))),
            Err(Error::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn find(&self, id: &str) -> Result<SoundboardClip, Error> {
        validate_id(id)?;
        self.list()?
            .into_iter()
            .find(|clip| clip.id == id)
            .ok_or(Error::NotFound)
    }

    fn clip_path(&self, clip: &SoundboardClip) -> PathBuf {
        self.root.join("clips").join(&clip.file_name)
    }

    fn preferences_json(&self) -> Result<serde_json::Value, Error> {
        match read_optional(&self.preferences)? {
            None => Ok(serde_json::json!({})),
            Some(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .filter(serde_json::Value::is_object)
                .ok_or(Error::InvalidPreferences),
        }
    }

    fn write_preference(&self, key: &str, value: serde_json::Value) -> Result<(), Error> {
        let mut json = self.preferences_json()?;
        json[key] = value;
        write_json(&self.preferences, &json)
    }

    fn write_catalog(&self, clips: &[SoundboardClip]) -> Result<(), Error> {
        write_json(&self.root.join("clips.json"), clips)
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Storage),
    }
}

fn write_json(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<(), Error> {
    let parent = path.parent().ok_or(Error::Storage)?;
    fs::create_dir_all(parent).map_err(|_| Error::Storage)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|_| Error::Storage)?;
    serde_json::to_writer_pretty(&mut staged, value).map_err(|_| Error::Storage)?;
    staged.as_file().sync_all().map_err(|_| Error::Storage)?;
    staged.persist(path).map_err(|_| Error::Storage)?;
    Ok(())
}

pub(crate) fn validate_id(id: &str) -> Result<(), Error> {
    valid_id(id).then_some(()).ok_or(Error::InvalidId)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn valid_file_name(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 160
        && !file.contains(['/', '\\', ':'])
        && !file.chars().any(char::is_control)
        && Path::new(file).file_name().and_then(|s| s.to_str()) == Some(file)
}
fn name(value: &str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(Error::InvalidName);
    }
    Ok(value.to_owned())
}
pub(crate) fn validate_volume(volume: u8) -> Result<(), Error> {
    (volume <= 100).then_some(()).ok_or(Error::InvalidVolume)
}

#[cfg(test)]
mod tests;
