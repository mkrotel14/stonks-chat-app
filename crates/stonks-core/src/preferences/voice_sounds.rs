//! Local overrides in Tauri's app-data layout (not in preferences.json).
use rodio::Source;
use std::{
    collections::HashMap,
    fmt, fs,
    io::{self, BufWriter, Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_BYTES: u64 = 15 * 1024 * 1024;
const CUSTOM_AUDIO: &str = "Custom audio";

macro_rules! cues {
    ($($variant:ident => ($id:literal, $file:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum Cue { $($variant),+ }

        impl Cue {
            pub const ALL: [Self; 11] = [$(Self::$variant),+];
            pub fn id(self) -> &'static str { match self { $(Self::$variant => $id),+ } }
            pub fn default_file(self) -> &'static str { match self { $(Self::$variant => $file),+ } }
            fn builtin(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("../../assets/cues/", $file))),+
                }
            }
        }
    }
}

cues! {
    Mute => ("microphoneMute", "microphone-mute.wav"),
    Unmute => ("microphoneUnmute", "microphone-unmute.wav"),
    Deafen => ("headphonesDeafen", "headphones-deafen.wav"),
    Undeafen => ("headphonesUndeafen", "headphones-undeafen.wav"),
    Join => ("voiceConnect", "voice-connect.wav"),
    Leave => ("voiceDisconnect", "voice-disconnect.wav"),
    Message => ("messageNotification", "message-notification.wav"),
    StreamStart => ("screenShareStart", "screen-share-start.wav"),
    StreamStop => ("screenShareStop", "screen-share-stop.wav"),
    ViewerJoin => ("screenViewerJoin", "screen-viewer-join.wav"),
    ViewerLeave => ("screenViewerLeave", "screen-viewer-leave.wav"),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Unavailable,
    Size,
    Format,
    Decode,
    Playback,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Custom UI sounds are unavailable. Try again.",
            Self::Size => "Choose an audio file between 1 byte and 15 MB.",
            Self::Format => "Choose an MP3, WAV, OGG, or FLAC file.",
            Self::Decode => "This audio format could not be decoded.",
            Self::Playback => "The native sound could not be played.",
        })
    }
}
impl std::error::Error for Error {}

pub struct Store(PathBuf);

impl Store {
    pub fn local() -> Result<Self, Error> {
        let root = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .ok_or(Error::Unavailable)?;
        Ok(Self(root.join("com.rdxlabs.stonks/custom-ui-sounds")))
    }

    fn paths(&self, cue: Cue) -> (PathBuf, PathBuf) {
        let id = cue.id();
        (
            self.0.join(format!("{id}.audio")),
            self.0.join(format!("{id}.name")),
        )
    }

    pub fn list(&self) -> Result<HashMap<Cue, String>, Error> {
        let mut sounds = HashMap::new();
        for cue in Cue::ALL {
            let (audio, name) = self.paths(cue);
            match fs::symlink_metadata(audio) {
                Ok(meta) if meta.is_file() => {
                    let name = fs::read_to_string(name).unwrap_or_default();
                    let name = name.trim();
                    sounds.insert(
                        cue,
                        if name.is_empty() { CUSTOM_AUDIO } else { name }.into(),
                    );
                }
                Ok(_) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(_) => return Err(Error::Unavailable),
            }
        }
        Ok(sounds)
    }

    pub fn import(&self, cue: Cue, source: &Path) -> Result<String, Error> {
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if !["mp3", "wav", "ogg", "flac"]
            .iter()
            .any(|ext| extension.eq_ignore_ascii_case(ext))
        {
            return Err(Error::Format);
        }
        let bytes = std::sync::Arc::<[u8]>::from(read_audio(source)?);
        let mut decoder =
            rodio::Decoder::try_from(Cursor::new(bytes.clone())).map_err(|_| Error::Decode)?;
        if decoder.next().is_none() {
            return Err(Error::Decode);
        }
        let name: String = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim()
            .chars()
            .take(160)
            .collect();
        let name = if name.is_empty() {
            CUSTOM_AUDIO.into()
        } else {
            name
        };
        fs::create_dir_all(&self.0).map_err(|_| Error::Unavailable)?;
        let (audio_path, name_path) = self.paths(cue);
        // Stage both writes before publishing either; playback never sees a partial audio file.
        let mut audio = tempfile::NamedTempFile::new_in(&self.0).map_err(|_| Error::Unavailable)?;
        let mut display =
            tempfile::NamedTempFile::new_in(&self.0).map_err(|_| Error::Unavailable)?;
        audio.write_all(&bytes).map_err(|_| Error::Unavailable)?;
        display
            .write_all(name.as_bytes())
            .map_err(|_| Error::Unavailable)?;
        audio.persist(audio_path).map_err(|_| Error::Unavailable)?;
        display.persist(name_path).map_err(|_| Error::Unavailable)?;
        Ok(name)
    }

    pub fn reset(&self, cue: Cue) -> Result<(), Error> {
        let (audio, name) = self.paths(cue);
        for path in [audio, name] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(_) => return Err(Error::Unavailable),
            }
        }
        Ok(())
    }

    fn preview_bytes(&self, cue: Cue) -> Vec<u8> {
        let (path, _) = self.paths(cue);
        read_audio(&path).unwrap_or_else(|_| cue.builtin().to_vec())
    }

    /// Explicit previews bypass automatic notification and call-sound switches.
    /// Run on a blocking worker; this waits for PipeWire playback to finish.
    pub fn preview(&self, cue: Cue) -> Result<(), Error> {
        let decoder = rodio::Decoder::try_from(Cursor::new(self.preview_bytes(cue)))
            .map_err(|_| Error::Playback)?;
        let mut child = Command::new("pw-play")
            .args([
                "--properties",
                "application.name=Stonks application.id=com.rdxlabs.stonks",
                "--raw",
                "--format",
                "f32",
                "--rate",
                &decoder.sample_rate().to_string(),
                "--channels",
                &decoder.channels().to_string(),
                "--volume",
                "1.0",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::Playback)?;
        let written = child.stdin.take().ok_or(Error::Playback).and_then(|input| {
            let mut input = BufWriter::new(input);
            for sample in decoder {
                input
                    .write_all(&sample.to_le_bytes())
                    .map_err(|_| Error::Playback)?;
            }
            input.flush().map_err(|_| Error::Playback)
        });
        let status = child.wait().map_err(|_| Error::Playback)?;
        if status.success() {
            written
        } else {
            Err(Error::Playback)
        }
    }
}

fn read_audio(path: &Path) -> Result<Vec<u8>, Error> {
    let meta = fs::metadata(path).map_err(|_| Error::Unavailable)?;
    if !meta.is_file() {
        return Err(Error::Unavailable);
    }
    if !(1..=MAX_BYTES).contains(&meta.len()) {
        return Err(Error::Size);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| Error::Unavailable)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    if !(1..=MAX_BYTES).contains(&(bytes.len() as u64)) {
        return Err(Error::Size);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_defaults_decode_and_use_distinct_tauri_ids() {
        let mut ids = std::collections::HashSet::new();
        for cue in Cue::ALL {
            assert!(ids.insert(cue.id()));
            assert!(
                rodio::Decoder::try_from(Cursor::new(cue.builtin()))
                    .unwrap()
                    .next()
                    .is_some(),
                "{cue:?}"
            );
        }
    }

    #[test]
    fn import_copies_override_and_reset_restores_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store(dir.path().join("custom-ui-sounds"));
        let source = dir.path().join("My cue.WAV");
        fs::write(&source, Cue::Leave.builtin()).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.import(Cue::Join, &source).unwrap(), "My cue.WAV");
        fs::remove_file(source).unwrap();
        let store = Store(store.0);
        assert_eq!(store.list().unwrap()[&Cue::Join], "My cue.WAV");
        assert_eq!(store.preview_bytes(Cue::Join), Cue::Leave.builtin());
        store.reset(Cue::Join).unwrap();
        store.reset(Cue::Join).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.preview_bytes(Cue::Join), Cue::Join.builtin());
    }

    #[test]
    fn rejects_unsupported_empty_corrupt_and_oversized_without_replacing_override() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store(dir.path().join("custom-ui-sounds"));
        let cue = Cue::Mute;
        let good = dir.path().join("good.wav");
        fs::write(&good, cue.builtin()).unwrap();
        store.import(cue, &good).unwrap();
        let bad = dir.path().join("invalid.wav");
        assert_eq!(
            store.import(cue, &good.with_extension("aac")),
            Err(Error::Format)
        );
        fs::write(&bad, []).unwrap();
        assert_eq!(store.import(cue, &bad), Err(Error::Size));
        fs::write(&bad, b"not audio").unwrap();
        assert_eq!(store.import(cue, &bad), Err(Error::Decode));
        let file = fs::File::create(&bad).unwrap();
        file.set_len(MAX_BYTES).unwrap();
        assert_eq!(read_audio(&bad).unwrap().len() as u64, MAX_BYTES);
        file.set_len(MAX_BYTES + 1).unwrap();
        assert_eq!(store.import(cue, &bad), Err(Error::Size));
        assert_eq!(store.list().unwrap()[&cue], "good.wav");
    }
}
