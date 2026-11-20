//! Native projection of Tauri's `app_config_dir()/preferences.json`.
//! Defaults and supported-value validation mirror settings/preferences.rs.
//! Individual edits preserve fields owned by Tauri or newer clients.
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub mod keybinds;
pub mod voice_sounds;

pub const PREFERENCES_FILE: &str = "preferences.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    Light,
    #[default]
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accent(String);
impl Default for Accent {
    fn default() -> Self {
        Self("#777cc1".into())
    }
}
impl Accent {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for Accent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() == 7
            && value.starts_with('#')
            && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            Ok(Self(value))
        } else {
            Err(serde::de::Error::custom("invalid accent"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DateFormat {
    #[default]
    DayFirst,
    MonthFirst,
    YearFirst,
}
impl DateFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DayFirst => "day-first",
            Self::MonthFirst => "month-first",
            Self::YearFirst => "year-first",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum TimeFormat {
    #[serde(rename = "12-hour")]
    TwelveHour,
    #[default]
    #[serde(rename = "24-hour")]
    TwentyFourHour,
}
impl TimeFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TwelveHour => "12-hour",
            Self::TwentyFourHour => "24-hour",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub input_device: String,
    pub output_device: String,
    pub video_device: String,
    pub camera_quality: VideoQuality,
    pub screen_share_quality: VideoQuality,
    pub screen_share_audio: bool,
    pub microphone_volume: u8,
    pub output_volume: u8,
    pub noise_suppression_mode: String,
    pub echo_cancellation: bool,
    pub automatic_gain_control: bool,
    pub theme: ThemePreference,
    pub accent: Accent,
    pub compact_mode: bool,
    pub reduced_motion: bool,
    pub minimize_to_tray: bool,
    pub spellcheck: bool,
    pub hardware_acceleration: bool,
    pub native_start_minimized: bool,
    pub developer_mode: bool,
    pub language: String,
    pub date_format: DateFormat,
    pub time_format: TimeFormat,
    pub show_notification_content: bool,
    pub send_typing_indicators: bool,
    pub display_embeds: bool,
    pub warn_external_links: bool,
    // Native trust is scoped to exact origins, not imported from hostname lists.
    native_trusted_link_origins: std::collections::BTreeSet<String>,
    pub desktop_notifications: bool,
    pub mentions_only: bool,
    pub direct_message_sounds: bool,
    pub server_message_sounds: bool,
    pub mention_sounds: bool,
    pub notification_sounds: bool,
    pub call_sounds: bool,
    pub keybinds: keybinds::Keybinds,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            input_device: "default".into(),
            output_device: "default".into(),
            video_device: "default".into(),
            camera_quality: VideoQuality::default(),
            screen_share_quality: VideoQuality::default(),
            screen_share_audio: false,
            microphone_volume: 35,
            output_volume: 100,
            noise_suppression_mode: "rnnoise".into(),
            echo_cancellation: true,
            automatic_gain_control: true,
            theme: ThemePreference::Dark,
            accent: Accent::default(),
            compact_mode: false,
            reduced_motion: false,
            minimize_to_tray: true,
            spellcheck: true,
            hardware_acceleration: true,
            native_start_minimized: false,
            developer_mode: false,
            language: "en".into(),
            date_format: DateFormat::DayFirst,
            time_format: TimeFormat::TwentyFourHour,
            show_notification_content: true,
            send_typing_indicators: true,
            display_embeds: true,
            warn_external_links: true,
            native_trusted_link_origins: Default::default(),
            desktop_notifications: true,
            mentions_only: false,
            direct_message_sounds: true,
            server_message_sounds: true,
            mention_sounds: true,
            notification_sounds: true,
            call_sounds: true,
            keybinds: Default::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum VideoQuality {
    #[default]
    #[serde(rename = "720p30")]
    Hd30,
    #[serde(rename = "1080p30")]
    FullHd30,
    #[serde(rename = "1080p60")]
    FullHd60,
}

impl VideoQuality {
    pub const ALL: [Self; 3] = [Self::Hd30, Self::FullHd30, Self::FullHd60];

    #[cfg(all(feature = "native-voice", any(target_os = "linux", windows)))]
    pub fn camera(self) -> crate::voice::camera::CameraQuality {
        use crate::voice::camera::CameraQuality;
        match self {
            Self::Hd30 => CameraQuality::Hd30,
            Self::FullHd30 => CameraQuality::FullHd30,
            Self::FullHd60 => CameraQuality::FullHd60,
        }
    }

    #[cfg(all(feature = "native-voice", any(target_os = "linux", windows)))]
    pub fn screen(self) -> crate::voice::screen::ScreenQuality {
        use crate::voice::screen::ScreenQuality;
        match self {
            Self::Hd30 => ScreenQuality::Hd30,
            Self::FullHd30 => ScreenQuality::FullHd30,
            Self::FullHd60 => ScreenQuality::FullHd60,
        }
    }
}

impl Preferences {
    #[cfg(all(feature = "native-voice", target_os = "linux"))]
    pub fn voice_audio(&self) -> crate::voice::VoiceAudioOptions {
        crate::voice::VoiceAudioOptions {
            input_device: self.input_device.clone(),
            output_device: self.output_device.clone(),
            // Match Tauri's normalization of legacy processing modes.
            rnnoise: self.noise_suppression_mode != "disabled",
            echo_cancellation: self.echo_cancellation,
            automatic_gain_control: self.automatic_gain_control,
            microphone_volume: self.microphone_volume.min(200),
            output_volume: self.output_volume.min(200),
        }
    }

    #[cfg(all(feature = "native-voice", any(target_os = "linux", windows)))]
    pub fn camera_options(&self) -> crate::voice::camera::CameraOptions {
        crate::voice::camera::CameraOptions {
            device: self.video_device.clone(),
            quality: self.camera_quality.camera(),
        }
    }

    #[cfg(all(feature = "native-voice", target_os = "linux"))]
    pub fn screen_options(&self) -> crate::voice::screen::ScreenShareOptions {
        crate::voice::screen::ScreenShareOptions {
            quality: self.screen_share_quality.screen(),
            share_audio: self.screen_share_audio,
        }
    }

    /// Tauri allows arbitrary language strings; both presenters fall back to
    /// English for dictionaries which are not bundled.
    pub fn effective_language(&self) -> &'static str {
        if self.language == "pt-BR" {
            "pt-BR"
        } else {
            "en"
        }
    }

    /// Only use after an error, never for a missing file. An invalid theme
    /// must not accidentally re-enable typing broadcasts or remote embeds
    /// when the persisted privacy choices cannot be recovered safely.
    pub fn privacy_safe_fallback() -> Self {
        Self {
            minimize_to_tray: false,
            show_notification_content: false,
            send_typing_indicators: false,
            display_embeds: false,
            ..Default::default()
        }
    }
}

/// Parse once and pass only the canonical destination to the browser.
pub fn external_web_url(value: &str) -> Option<url::Url> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

impl Preferences {
    pub fn warns_for_link(&self, url: &url::Url) -> bool {
        self.warn_external_links
            && !self
                .native_trusted_link_origins
                .contains(&url.origin().ascii_serialization())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceError {
    Unavailable,
    Invalid,
    WriteFailed,
}
impl fmt::Display for PreferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "The saved preferences could not be loaded.",
            Self::WriteFailed => "The preference change could not be saved.",
            Self::Invalid => "The saved preferences contain invalid or unsupported values.",
        })
    }
}
impl std::error::Error for PreferenceError {}

/// `app_config_dir` is the application directory, already including
/// `com.rdxlabs.stonks`, not the XDG configuration root or app-data directory.
pub fn path_in(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(PREFERENCES_FILE)
}

pub fn parse(bytes: &[u8]) -> Result<Preferences, PreferenceError> {
    serde_json::from_slice(bytes).map_err(|_| PreferenceError::Invalid)
}

pub fn load_file(path: &Path) -> Result<Preferences, PreferenceError> {
    match std::fs::read(path) {
        Ok(bytes) => parse(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(_) => Err(PreferenceError::Unavailable),
    }
}

/// Editable native preferences, including the shared desktop preview choice.
#[derive(Debug, Clone)]
pub enum Change {
    InputDevice(String),
    OutputDevice(String),
    VideoDevice(String),
    CameraQuality(VideoQuality),
    ScreenShareQuality(VideoQuality),
    ScreenShareAudio(bool),
    MicrophoneVolume(u8),
    OutputVolume(u8),
    NoiseSuppression(bool),
    EchoCancellation(bool),
    AutomaticGainControl(bool),
    Theme(ThemePreference),
    Accent([u8; 3]),
    CompactMode(bool),
    ReducedMotion(bool),
    MinimizeToTray(bool),
    Spellcheck(bool),
    HardwareAcceleration(bool),
    NativeStartMinimized(bool),
    DeveloperMode(bool),
    Language(Language),
    DateFormat(DateFormat),
    TimeFormat(TimeFormat),
    DesktopNotifications(bool),
    MentionsOnly(bool),
    DirectMessageSounds(bool),
    ServerMessageSounds(bool),
    MentionSounds(bool),
    NotificationSounds(bool),
    CallSounds(bool),
    ShowNotificationContent(bool),
    TypingIndicators(bool),
    DisplayEmbeds(bool),
    LinkWarnings(bool),
    TrustLinkOrigin(String),
    ClearTrustedLinkOrigins,
    Keybind(keybinds::Action, String),
}

#[derive(Debug, Clone, Copy)]
pub enum Language {
    English,
    Portuguese,
}

/// Patch the latest on-disk object, not a serialization of the native projection.
/// Malformed files are never repaired implicitly. Atomic replacement keeps other
/// readers from seeing partial JSON; simultaneous Tauri writers are not coordinated.
pub fn write_change(path: &Path, change: Change) -> Result<Preferences, PreferenceError> {
    use serde_json::{json, Value};
    let mut document = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice::<Value>(&bytes).map_err(|_| PreferenceError::Invalid)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(_) => return Err(PreferenceError::Unavailable),
    };
    if !document.is_object() {
        return Err(PreferenceError::Invalid);
    }
    // Validate the old document too: editing a field must not silently repair it.
    let previous: Preferences =
        serde_json::from_value(document.clone()).map_err(|_| PreferenceError::Invalid)?;
    let (key, value) = match change {
        Change::InputDevice(value) => ("inputDevice", json!(value)),
        Change::OutputDevice(value) => ("outputDevice", json!(value)),
        Change::VideoDevice(value) => ("videoDevice", json!(value)),
        Change::CameraQuality(value) => ("cameraQuality", json!(value)),
        Change::ScreenShareQuality(value) => ("screenShareQuality", json!(value)),
        Change::ScreenShareAudio(value) => ("screenShareAudio", json!(value)),
        Change::MicrophoneVolume(value) if value <= 200 => ("microphoneVolume", json!(value)),
        Change::OutputVolume(value) if value <= 200 => ("outputVolume", json!(value)),
        Change::MicrophoneVolume(_) | Change::OutputVolume(_) => {
            return Err(PreferenceError::Invalid);
        }
        Change::NoiseSuppression(value) => (
            "noiseSuppressionMode",
            json!(if value { "rnnoise" } else { "disabled" }),
        ),
        Change::EchoCancellation(value) => ("echoCancellation", json!(value)),
        Change::AutomaticGainControl(value) => ("automaticGainControl", json!(value)),
        Change::Theme(value) => ("theme", json!(value)),
        Change::Accent([r, g, b]) => ("accent", json!(format!("#{r:02x}{g:02x}{b:02x}"))),
        Change::CompactMode(value) => ("compactMode", json!(value)),
        Change::ReducedMotion(value) => ("reducedMotion", json!(value)),
        Change::MinimizeToTray(value) => ("minimizeToTray", json!(value)),
        Change::Spellcheck(value) => ("spellcheck", json!(value)),
        Change::HardwareAcceleration(value) => ("hardwareAcceleration", json!(value)),
        Change::NativeStartMinimized(value) => ("nativeStartMinimized", json!(value)),
        Change::DeveloperMode(value) => ("developerMode", json!(value)),
        Change::Language(value) => (
            "language",
            json!(match value {
                Language::English => "en",
                Language::Portuguese => "pt-BR",
            }),
        ),
        Change::DateFormat(value) => ("dateFormat", json!(value)),
        Change::TimeFormat(value) => ("timeFormat", json!(value)),
        Change::DesktopNotifications(value) => ("desktopNotifications", json!(value)),
        Change::MentionsOnly(value) => ("mentionsOnly", json!(value)),
        Change::DirectMessageSounds(value) => ("directMessageSounds", json!(value)),
        Change::ServerMessageSounds(value) => ("serverMessageSounds", json!(value)),
        Change::MentionSounds(value) => ("mentionSounds", json!(value)),
        Change::NotificationSounds(value) => ("notificationSounds", json!(value)),
        Change::CallSounds(value) => ("callSounds", json!(value)),
        Change::ShowNotificationContent(value) => ("showNotificationContent", json!(value)),
        Change::TypingIndicators(value) => ("sendTypingIndicators", json!(value)),
        Change::DisplayEmbeds(value) => ("displayEmbeds", json!(value)),
        Change::LinkWarnings(value) => ("warnExternalLinks", json!(value)),
        Change::TrustLinkOrigin(origin) => {
            let url = external_web_url(&origin).ok_or(PreferenceError::Invalid)?;
            if url.origin().ascii_serialization() != origin {
                return Err(PreferenceError::Invalid);
            }
            let mut origins = previous.native_trusted_link_origins.clone();
            origins.insert(origin);
            ("nativeTrustedLinkOrigins", json!(origins))
        }
        Change::ClearTrustedLinkOrigins => ("nativeTrustedLinkOrigins", json!([])),
        Change::Keybind(action, value) => {
            let value = keybinds::normalize(&value).ok_or(PreferenceError::Invalid)?;
            if previous.keybinds.conflict(action, &value).is_some() {
                return Err(PreferenceError::Invalid);
            }
            let mut bindings = document
                .get("keybinds")
                .cloned()
                .unwrap_or_else(|| json!({}));
            bindings[action.name()] = json!(value);
            ("keybinds", bindings)
        }
    };
    document[key] = value;
    let saved: Preferences =
        serde_json::from_value(document.clone()).map_err(|_| PreferenceError::Invalid)?;
    if saved == previous {
        return Ok(saved);
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|_| PreferenceError::WriteFailed)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| PreferenceError::WriteFailed)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), &document)
        .map_err(|_| PreferenceError::WriteFailed)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| PreferenceError::WriteFailed)?;
    temporary
        .persist(path)
        .map_err(|_| PreferenceError::WriteFailed)?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "stonks-pref-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn missing_file_uses_tauri_defaults_without_creating_any_file() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        assert_eq!(load_file(&path).unwrap(), Preferences::default());
        assert!(!path.exists());
        assert!(Preferences::default().send_typing_indicators);
        assert!(Preferences::default().display_embeds);
        assert_eq!(Preferences::default().accent.as_str(), "#777cc1");
    }
    #[test]
    fn loads_supported_fields_and_preserves_the_entire_saved_json() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        let bytes=br##"{"theme":"system","accent":"#Ab12Ef","compactMode":true,"reducedMotion":true,"language":"pt-BR","dateFormat":"year-first","timeFormat":"12-hour","sendTypingIndicators":false,"displayEmbeds":false,"warnExternalLinks":false,"noiseSuppressionMode":"webrtc","keybinds":{"unknownFutureAction":"F8"},"futureSetting":{"keep":true}}"##;
        std::fs::write(&path, bytes).unwrap();
        let saved = load_file(&path).unwrap();
        assert_eq!(saved.theme, ThemePreference::System);
        assert_eq!(saved.accent.as_str(), "#Ab12Ef");
        assert_eq!(saved.effective_language(), "pt-BR");
        assert_eq!(saved.date_format.as_str(), "year-first");
        assert_eq!(saved.time_format.as_str(), "12-hour");
        assert!(saved.compact_mode && saved.reduced_motion);
        assert!(
            !saved.send_typing_indicators && !saved.display_embeds && !saved.warn_external_links
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn invalid_saved_fields_report_errors_and_are_never_repaired() {
        for bytes in [
            b"not json".as_slice(),
            br#"{"theme":"sepia"}"#,
            br#"{"accent":"red"}"#,
            br##"{"accent":"#12345z"}"##,
            br#"{"sendTypingIndicators":"false"}"#,
            br#"{"dateFormat":"locale"}"#,
            br#"{"timeFormat":"auto"}"#,
            br#"null"#,
        ] {
            let directory = Directory::new();
            let path = path_in(&directory.0);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(load_file(&path), Err(PreferenceError::Invalid));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn unknown_language_falls_back_but_known_privacy_defaults_are_preserved() {
        let preferences =
            parse(br#"{"language":"xx-future","sendTypingIndicators":false}"#).unwrap();
        assert_eq!(preferences.language, "xx-future");
        assert_eq!(preferences.effective_language(), "en");
        assert!(!preferences.send_typing_indicators);
        assert!(preferences.display_embeds);
        assert!(!Preferences::privacy_safe_fallback().send_typing_indicators);
        assert!(!Preferences::privacy_safe_fallback().display_embeds);
    }
    #[test]
    fn writes_only_the_requested_preference_and_keeps_nested_tauri_data() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        let original = serde_json::json!({
            "theme": "system", "sendTypingIndicators": false,
            "keybinds": { "toggleMute": "F8", "futureAction": "F9" },
            "futureSetting": [1, { "keep": true }]
        });
        std::fs::write(&path, original.to_string()).unwrap();
        write_change(&path, Change::Language(Language::Portuguese)).unwrap();
        let saved = write_change(&path, Change::CallSounds(false)).unwrap();
        assert_eq!(saved.effective_language(), "pt-BR");
        assert!(!saved.call_sounds);
        assert!(!saved.send_typing_indicators);
        assert_eq!(load_file(&path), Ok(saved));
        let mut expected = original;
        expected["language"] = serde_json::json!("pt-BR");
        expected["callSounds"] = serde_json::json!(false);
        let actual: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn desktop_preferences_use_shared_keys_and_preserve_unrelated_settings() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        let defaults = load_file(&path).unwrap();
        assert!(defaults.minimize_to_tray && defaults.spellcheck && defaults.hardware_acceleration);
        assert!(!Preferences::privacy_safe_fallback().minimize_to_tray);

        std::fs::write(&path, r#"{"startMinimized":true,"futureSetting":[1]}"#).unwrap();
        write_change(&path, Change::MinimizeToTray(false)).unwrap();
        write_change(&path, Change::Spellcheck(false)).unwrap();
        let saved = write_change(&path, Change::HardwareAcceleration(false)).unwrap();
        assert!(!saved.minimize_to_tray && !saved.spellcheck && !saved.hardware_acceleration);
        assert_eq!(load_file(&path), Ok(saved));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            raw,
            serde_json::json!({
                "startMinimized": true, "futureSetting": [1], "minimizeToTray": false,
                "spellcheck": false, "hardwareAcceleration": false,
            })
        );
    }

    #[test]
    fn native_startup_and_developer_settings_preserve_tauri_tray_fields() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        std::fs::write(&path, r#"{"startMinimized":true,"minimizeToTray":false}"#).unwrap();
        let initial = load_file(&path).unwrap();
        assert!(!initial.native_start_minimized);
        assert!(!initial.developer_mode);

        write_change(&path, Change::NativeStartMinimized(true)).unwrap();
        let saved = write_change(&path, Change::DeveloperMode(true)).unwrap();
        assert!(saved.native_start_minimized && saved.developer_mode);
        assert_eq!(load_file(&path), Ok(saved));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(json["startMinimized"], true);
        assert_eq!(json["minimizeToTray"], false);
    }

    #[test]
    fn external_links_only_accept_credential_free_web_urls() {
        for invalid in [
            "file:///tmp/test",
            "javascript:alert(1)",
            "mailto:user@example.com",
            "//example.com",
            "https://example.com@other.example/",
            "https://:secret@example.com/",
            "https://exam\nple.com/",
        ] {
            assert!(external_web_url(invalid).is_none(), "{invalid:?}");
        }
        let url = external_web_url("HTTPS://EXAMPLE.COM:443/path").unwrap();
        assert_eq!(url.as_str(), "https://example.com/path");
    }

    #[test]
    fn native_link_trust_is_exact_persistent_and_clearable() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        std::fs::write(
            &path,
            r#"{"trustedDomains":["example.com"],"futureField":42}"#,
        )
        .unwrap();
        let trusted = external_web_url("https://example.com/path").unwrap();
        assert!(load_file(&path).unwrap().warns_for_link(&trusted));
        assert_eq!(
            write_change(&path, Change::TrustLinkOrigin(trusted.to_string())),
            Err(PreferenceError::Invalid)
        );
        write_change(
            &path,
            Change::TrustLinkOrigin(trusted.origin().ascii_serialization()),
        )
        .unwrap();
        let saved = load_file(&path).unwrap();
        assert!(!saved.warns_for_link(&trusted));
        for other in [
            "http://example.com",
            "https://example.com:8443",
            "https://sub.example.com",
            "https://example.com.evil.test",
        ] {
            assert!(saved.warns_for_link(&external_web_url(other).unwrap()));
        }
        let saved = write_change(&path, Change::LinkWarnings(false)).unwrap();
        assert!(!saved.warns_for_link(&external_web_url("https://other.example").unwrap()));
        write_change(&path, Change::ClearTrustedLinkOrigins).unwrap();
        write_change(&path, Change::LinkWarnings(true)).unwrap();
        assert!(load_file(&path).unwrap().warns_for_link(&trusted));
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["trustedDomains"], serde_json::json!(["example.com"]));
        assert_eq!(raw["futureField"], 42);
    }

    #[test]
    fn notification_content_uses_the_shared_preference_key() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        std::fs::write(&path, r#"{"desktopNotifications":true}"#).unwrap();
        assert!(load_file(&path).unwrap().show_notification_content);
        assert!(!Preferences::privacy_safe_fallback().show_notification_content);

        let saved = write_change(&path, Change::ShowNotificationContent(false)).unwrap();
        assert!(!saved.show_notification_content);
        assert_eq!(load_file(&path), Ok(saved));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(raw["showNotificationContent"], false);
        assert_eq!(raw["desktopNotifications"], true);
    }

    #[test]
    fn privacy_changes_persist_independently_and_preserve_trusted_domains() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        std::fs::write(&path, r#"{"trustedDomains":["example.com"]}"#).unwrap();
        let saved = write_change(&path, Change::TypingIndicators(false)).unwrap();
        assert!(!saved.send_typing_indicators);
        assert!(saved.display_embeds);
        let saved = write_change(&path, Change::DisplayEmbeds(false)).unwrap();
        assert!(!saved.send_typing_indicators);
        assert!(!saved.display_embeds);
        assert_eq!(load_file(&path), Ok(saved));
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            document["trustedDomains"],
            serde_json::json!(["example.com"])
        );
    }

    #[test]
    fn refuses_to_overwrite_invalid_documents_even_when_editing_the_invalid_field() {
        let directory = Directory::new();
        let path = path_in(&directory.0);
        for invalid in [r#"{"theme":"sepia"}"#, "[]", "not json"] {
            std::fs::write(&path, invalid).unwrap();
            assert_eq!(
                write_change(&path, Change::Theme(ThemePreference::Light)),
                Err(PreferenceError::Invalid)
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
    }

    #[test]
    fn first_edit_creates_the_file_with_only_the_edited_field() {
        let directory = Directory::new();
        let path = path_in(&directory.0.join("config"));
        assert!(
            write_change(&path, Change::CompactMode(true))
                .unwrap()
                .compact_mode
        );
        let actual: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(actual, serde_json::json!({"compactMode": true}));
    }

    #[test]
    fn read_errors_are_distinct_from_invalid_json() {
        let directory = Directory::new();
        assert_eq!(load_file(&directory.0), Err(PreferenceError::Unavailable));
    }
}

#[cfg(all(test, feature = "native-voice", target_os = "linux"))]
mod voice_tests {
    use super::*;
    use crate::voice::{camera::CameraQuality, screen::ScreenQuality};
    use serde_json::{json, Value};

    #[test]
    fn voice_patches_preserve_tauri_fields_and_feed_native_options() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        std::fs::write(
            &path,
            br#"{"echoCancellation":false,"alwaysAskCameraQuality":false}"#,
        )
        .unwrap();
        for change in [
            Change::InputDevice("pipewire:mic".into()),
            Change::OutputDevice("pipewire:sink".into()),
            Change::VideoDevice("pipewire:cam".into()),
            Change::MicrophoneVolume(165),
            Change::OutputVolume(80),
            Change::NoiseSuppression(false),
            Change::AutomaticGainControl(false),
            Change::CameraQuality(VideoQuality::FullHd60),
            Change::ScreenShareQuality(VideoQuality::FullHd30),
            Change::ScreenShareAudio(true),
        ] {
            write_change(&path, change).unwrap();
        }
        assert_eq!(
            write_change(&path, Change::OutputVolume(201)),
            Err(PreferenceError::Invalid)
        );
        let prefs = load_file(&path).unwrap();
        let audio = prefs.voice_audio();
        assert_eq!(
            (audio.input_device.as_str(), audio.output_device.as_str()),
            ("pipewire:mic", "pipewire:sink")
        );
        assert_eq!(
            (audio.microphone_volume, audio.output_volume, audio.rnnoise),
            (165, 80, false)
        );
        assert!(!audio.echo_cancellation);
        assert!(!audio.automatic_gain_control);
        write_change(&path, Change::EchoCancellation(true)).unwrap();
        assert!(load_file(&path).unwrap().voice_audio().echo_cancellation);
        let camera = prefs.camera_options();
        assert_eq!(camera.device, "pipewire:cam");
        assert_eq!(camera.quality, CameraQuality::FullHd60);
        let screen = prefs.screen_options();
        assert_eq!(screen.quality, ScreenQuality::FullHd30);
        assert!(screen.share_audio);
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["echoCancellation"], json!(true));
        assert_eq!(raw["automaticGainControl"], json!(false));
        assert_eq!(raw["alwaysAskCameraQuality"], json!(false));
        assert_eq!(raw["cameraQuality"], json!("1080p60"));
    }

    #[test]
    fn voice_defaults_are_compatible_with_tauri() {
        let prefs = parse(br#"{"noiseSuppressionMode":"webrtc"}"#).unwrap();
        let audio = prefs.voice_audio();
        assert!(audio.rnnoise);
        assert!(audio.echo_cancellation);
        assert!(audio.automatic_gain_control);
        assert_eq!(audio.microphone_volume, 35);
        assert_eq!(audio.output_volume, 100);
        assert!(!prefs.screen_options().share_audio);
        assert_eq!(prefs.camera_options().device, "default");
    }
}
