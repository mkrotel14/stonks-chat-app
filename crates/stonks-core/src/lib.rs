//! Stonks domain core: protocol models and business logic for the native desktop app.

pub mod account_settings;
pub mod auth;
pub mod connection;
pub mod error;
pub mod http;
pub mod media;
pub mod messages;
pub mod preferences;
pub mod server;
pub mod server_settings;
pub mod session;
pub mod soundboard;

pub mod channel_actions;
pub mod member_actions;
pub mod release_discovery;
pub mod uploads;

pub mod channel_preferences;
#[cfg(feature = "media-playback")]
pub mod media_playback;

#[cfg(all(
    feature = "native-voice",
    any(target_os = "linux", target_os = "windows")
))]
pub mod voice;
