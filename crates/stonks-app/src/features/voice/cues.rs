//! Best-effort local cues; playback never blocks room controls or teardown.
use std::{
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Deserialize;
use stonks_core::voice::VoicePresence;
use tokio::{
    process::{Child, Command},
    sync::{mpsc, watch},
    time::timeout,
};

use super::{Control, VoiceSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cue {
    Join,
    Leave,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
    ShareStart,
    ShareStop,
}

impl From<VoicePresence> for Cue {
    fn from(event: VoicePresence) -> Self {
        match event {
            VoicePresence::Joined => Self::Join,
            VoicePresence::Left => Self::Leave,
        }
    }
}

impl Cue {
    pub(super) fn acknowledged(
        command: Control,
        before: &VoiceSnapshot,
        after: &VoiceSnapshot,
    ) -> Option<Self> {
        match command {
            Control::Mute(_) if before.muted != after.muted => Some(if after.muted {
                Self::Mute
            } else {
                Self::Unmute
            }),
            Control::Deafen(_) if before.deafened != after.deafened => Some(if after.deafened {
                Self::Deafen
            } else {
                Self::Undeafen
            }),
            _ => None,
        }
    }

    fn asset(self) -> (&'static str, &'static [u8]) {
        macro_rules! asset {
            ($name:literal, $file:literal) => {
                (
                    $name,
                    include_bytes!(concat!("../../../../stonks-core/assets/cues/", $file)),
                )
            };
        }
        match self {
            Self::ShareStart => asset!("screenShareStart", "screen-share-start.wav"),
            Self::ShareStop => asset!("screenShareStop", "screen-share-stop.wav"),
            Self::Join => asset!("voiceConnect", "voice-connect.wav"),
            Self::Leave => asset!("voiceDisconnect", "voice-disconnect.wav"),
            Self::Mute => asset!("microphoneMute", "microphone-mute.wav"),
            Self::Unmute => asset!("microphoneUnmute", "microphone-unmute.wav"),
            Self::Deafen => asset!("headphonesDeafen", "headphones-deafen.wav"),
            Self::Undeafen => asset!("headphonesUndeafen", "headphones-undeafen.wav"),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoundPreferences {
    notification_sounds: bool,
    call_sounds: bool,
}

impl Default for SoundPreferences {
    fn default() -> Self {
        Self {
            notification_sounds: true,
            call_sounds: true,
        }
    }
}

fn allowed(path: &Path) -> bool {
    let preferences = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(SoundPreferences::default()),
        Err(_) => None,
    };
    preferences.is_some_and(|prefs| prefs.notification_sounds && prefs.call_sounds)
}

pub(super) fn app_data_directory() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data.join("com.rdxlabs.stonks"))
}

fn custom_directory() -> Option<PathBuf> {
    Some(app_data_directory()?.join("custom-ui-sounds"))
}

fn audio_input(cue: Cue, custom: Option<&Path>) -> io::Result<Stdio> {
    let (name, builtin) = cue.asset();
    if let Some(path) = custom.map(|dir| dir.join(format!("{name}.audio"))) {
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_file() && (1..=15 * 1024 * 1024).contains(&meta.len()))
        {
            if let Ok(file) = std::fs::File::open(path) {
                return Ok(file.into());
            }
        }
    }
    let mut file = tempfile::tempfile()?;
    file.write_all(builtin)?;
    std::io::Seek::rewind(&mut file)?;
    Ok(file.into())
}

async fn prepare(cue: Cue, preferences: PathBuf, custom: Option<PathBuf>) -> Option<Stdio> {
    tokio::task::spawn_blocking(move || {
        allowed(&preferences).then(|| audio_input(cue, custom.as_deref()))
    })
    .await
    .ok()?
    .and_then(Result::ok)
}

fn spawn(cue: Cue, input: Stdio) -> Option<Child> {
    // Feed local bytes to GStreamer without embedding paths in its pipeline.
    match Command::new("gst-launch-1.0")
        .args([
            "-q",
            "fdsrc",
            "!",
            "decodebin",
            "!",
            "audioconvert",
            "!",
            "audioresample",
            "!",
            "autoaudiosink",
        ])
        .stdin(input)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            eprintln!("voice cue {cue:?}: could not start local playback: {error}");
            None
        }
    }
}

// Returns whether playback was interrupted. Reap before starting a replacement;
// kill_on_drop is a fallback if the runtime drops this task during shutdown.
async fn finish(cue: Cue, mut child: Child, interrupt: impl Future<Output = ()>) -> bool {
    let interrupted = tokio::select! {
        biased;
        _ = interrupt => true,
        result = timeout(Duration::from_secs(5), child.wait()) => {
            match result {
                Ok(Ok(status)) if status.success() => return false,
                _ => eprintln!("voice cue {cue:?}: local playback failed"),
            }
            false
        }
    };
    if let Err(error) = child.kill().await {
        eprintln!("voice cue {cue:?}: could not stop local playback: {error}");
    }
    interrupted
}

pub(super) fn player(preferences: PathBuf) -> mpsc::Sender<Cue> {
    let (tx, mut rx) = mpsc::channel(4);
    tokio::spawn(async move {
        let custom = custom_directory();
        while let Some(cue) = rx.recv().await {
            if let Some(child) = prepare(cue, preferences.clone(), custom.clone())
                .await
                .and_then(|input| spawn(cue, input))
            {
                finish(cue, child, std::future::pending()).await;
            }
        }
    });
    tx
}

pub(super) fn control_player(preferences: PathBuf) -> watch::Sender<Option<Cue>> {
    let (tx, rx) = watch::channel(None);
    let custom = custom_directory();
    tokio::spawn(play_controls(
        rx,
        move |cue| prepare(cue, preferences.clone(), custom.clone()),
        spawn,
    ));
    tx
}

async fn play_controls<F: Future<Output = Option<Stdio>>>(
    mut rx: watch::Receiver<Option<Cue>>,
    prepare: impl Fn(Cue) -> F,
    spawn: impl Fn(Cue, Stdio) -> Option<Child>,
) {
    if rx.changed().await.is_err() {
        return;
    }
    while rx.has_changed().is_ok() {
        let Some(cue) = *rx.borrow_and_update() else {
            break;
        };
        // Only one blocking preparation at a time. If superseded while reading
        // preferences or input, discard it before starting a process.
        let input = prepare(cue).await;
        match rx.has_changed() {
            Ok(true) => continue,
            Err(_) => break,
            Ok(false) => (),
        }
        if let Some(child) = input.and_then(|input| spawn(cue, input)) {
            if finish(cue, child, async {
                let _ = rx.changed().await;
            })
            .await
            {
                continue;
            }
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stonks_core::voice::VoicePhase;

    #[tokio::test]
    async fn latest_control_skips_stale_preparation_and_reaps_active_children() {
        use tokio::sync::{oneshot, Notify};

        timeout(Duration::from_secs(5), async {
            let (tx, rx) = watch::channel(None);
            let (preparing, started) = oneshot::channel();
            let (spawned, mut children) = mpsc::unbounded_channel();
            let release_preparation = Notify::new();
            let worker = play_controls(
                rx,
                |cue| {
                    let release = &release_preparation;
                    async move {
                        if cue == Cue::Mute {
                            release.notified().await;
                        }
                        Some(Stdio::null())
                    }
                },
                |cue, input| {
                    // No audio, and no shell or descendants to clean up.
                    let child = Command::new("sleep")
                        .arg("60")
                        .stdin(input)
                        .kill_on_drop(true)
                        .spawn()
                        .unwrap();
                    spawned.send((cue, child.id().unwrap())).unwrap();
                    Some(child)
                },
            );
            tx.send(Some(Cue::Mute)).unwrap();
            // Signal once the worker has polled the initial cue's preparation.
            let worker = async {
                tokio::pin!(worker);
                assert!(futures_util::poll!(&mut worker).is_pending());
                preparing.send(()).unwrap();
                worker.await;
            };
            let drive = async {
                started.await.unwrap();
                for cue in [Cue::Unmute, Cue::Deafen, Cue::Undeafen] {
                    tx.send(Some(cue)).unwrap();
                }
                release_preparation.notify_one();
                let (cue, first_pid) = children.recv().await.unwrap();
                assert_eq!(cue, Cue::Undeafen);

                tx.send(Some(Cue::Deafen)).unwrap();
                let (cue, second_pid) = children.recv().await.unwrap();
                assert_eq!(cue, Cue::Deafen);
                assert!(!Path::new(&format!("/proc/{first_pid}")).exists());

                // Closing the lane cancels active playback and discards any pending cue.
                tx.send(Some(Cue::Unmute)).unwrap();
                drop(tx);
                second_pid
            };
            let ((), second_pid) = tokio::join!(worker, drive);
            assert!(!Path::new(&format!("/proc/{second_pid}")).exists());
            assert!(children.try_recv().is_err());
        })
        .await
        .expect("cue worker should replace and reap children promptly");
    }

    #[test]
    fn remote_presence_reuses_configurable_call_assets() {
        assert_eq!(Cue::from(VoicePresence::Joined).asset().0, "voiceConnect");
        assert_eq!(Cue::from(VoicePresence::Left).asset().0, "voiceDisconnect");
    }

    #[test]
    fn automatic_cues_require_both_tauri_switches_and_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        assert!(allowed(&path));
        for (json, expected) in [
            ("{}", true),
            (r#"{"notificationSounds":true,"callSounds":true}"#, true),
            (r#"{"notificationSounds":false}"#, false),
            (r#"{"callSounds":false}"#, false),
            (r#"{"callSounds":"false"}"#, false),
            ("not JSON", false),
        ] {
            std::fs::write(&path, json).unwrap();
            assert_eq!(allowed(&path), expected, "{json}");
        }
    }

    #[test]
    fn acknowledged_controls_use_only_the_changed_preference() {
        let initial = VoiceSnapshot {
            channel_id: "channel".into(),
            phase: VoicePhase::Connected,
            can_speak: true,
            muted: false,
            deafened: false,
            error: None,
            active_speakers: Default::default(),
        };
        let muted = VoiceSnapshot {
            muted: true,
            ..initial.clone()
        };
        let deafened = VoiceSnapshot {
            deafened: true,
            ..initial.clone()
        };
        assert_eq!(
            Cue::acknowledged(Control::Mute(true), &initial, &muted),
            Some(Cue::Mute)
        );
        assert_eq!(
            Cue::acknowledged(Control::Mute(false), &muted, &initial),
            Some(Cue::Unmute)
        );
        assert_eq!(
            Cue::acknowledged(Control::Deafen(true), &initial, &deafened),
            Some(Cue::Deafen)
        );
        assert_eq!(
            Cue::acknowledged(Control::Deafen(false), &deafened, &initial),
            Some(Cue::Undeafen)
        );
        assert_eq!(
            Cue::acknowledged(Control::Mute(false), &initial, &deafened),
            None
        );
        assert_eq!(Cue::acknowledged(Control::Mute(true), &muted, &muted), None);
    }
}
