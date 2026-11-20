//! Channel notification edits preserve every unrelated preference field.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationMode {
    All,
    Mention,
    None,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Mute {
    pub until: Option<i64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelPreferences {
    pub channel_notification_modes: HashMap<String, NotificationMode>,
    pub channel_mutes: HashMap<String, Mute>,
}
#[derive(Clone, Copy, Debug)]
pub enum Change {
    Notifications(Option<NotificationMode>),
    Mute(Option<Option<i64>>),
}
impl ChannelPreferences {
    pub fn is_muted(&self, channel: &str, now: i64) -> bool {
        self.channel_mutes
            .get(channel)
            .is_some_and(|mute| mute.until.is_none_or(|until| until > now))
    }
    pub fn apply(&mut self, channel: &str, change: Change) {
        match change {
            Change::Notifications(Some(mode)) => {
                self.channel_notification_modes.insert(channel.into(), mode);
            }
            Change::Notifications(None) => {
                self.channel_notification_modes.remove(channel);
            }
            Change::Mute(Some(until)) => {
                self.channel_mutes.insert(channel.into(), Mute { until });
            }
            Change::Mute(None) => {
                self.channel_mutes.remove(channel);
            }
        }
    }
}
fn raw(path: &Path) -> Result<serde_json::Value, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .filter(serde_json::Value::is_object)
            .ok_or_else(|| "The saved preferences are invalid.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(_) => Err("Could not load channel notification preferences.".into()),
    }
}
pub fn load(path: &Path) -> Result<ChannelPreferences, String> {
    serde_json::from_value(raw(path)?)
        .map_err(|_| "The channel notification preferences are invalid.".into())
}
pub fn write_change(
    path: &Path,
    channel: &str,
    change: Change,
) -> Result<ChannelPreferences, String> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    static WRITE: Mutex<()> = Mutex::new(());
    let _guard = WRITE
        .lock()
        .map_err(|_| "Could not save channel preferences.")?;
    if channel.is_empty()
        || channel.len() > 64
        || !channel.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err("The channel ID is invalid.".into());
    }
    let mut value = raw(path)?;
    let mut preferences: ChannelPreferences = serde_json::from_value(value.clone())
        .map_err(|_| "The channel notification preferences are invalid.")?;
    preferences.apply(channel, change);
    value["channelNotificationModes"] =
        serde_json::to_value(&preferences.channel_notification_modes)
            .map_err(|_| "Could not save channel preferences.")?;
    value["channelMutes"] = serde_json::to_value(&preferences.channel_mutes)
        .map_err(|_| "Could not save channel preferences.")?;
    let parent = path
        .parent()
        .ok_or("The preferences location is unavailable.")?;
    std::fs::create_dir_all(parent).map_err(|_| "Could not save channel preferences.")?;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let temporary = parent.join(format!(
        ".channel-preferences-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|_| "Could not save channel preferences.")?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Could not save channel preferences.")?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not save channel preferences.")?;
        std::fs::rename(&temporary, path).map_err(|_| "Could not save channel preferences.")?;
        Ok(preferences)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
    #[test]
    fn mute_expiry_indefinite_and_unmute() {
        let mut p = ChannelPreferences::default();
        p.apply(CHANNEL, Change::Mute(Some(Some(100))));
        assert!(p.is_muted(CHANNEL, 99));
        assert!(!p.is_muted(CHANNEL, 100));
        p.apply(CHANNEL, Change::Mute(Some(None)));
        assert!(p.is_muted(CHANNEL, i64::MAX));
        p.apply(CHANNEL, Change::Mute(None));
        assert!(!p.is_muted(CHANNEL, 0));
    }
    #[test]
    fn edits_preserve_unknown_data_and_other_channels() {
        let dir = std::env::temp_dir().join(format!("stonks-channel-pref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        std::fs::write(&path,br#"{"future":{"preserve":true},"sendTypingIndicators":false,"channelMutes":{"other":{"until":null}}}"#).unwrap();
        let p = write_change(
            &path,
            CHANNEL,
            Change::Notifications(Some(NotificationMode::Mention)),
        )
        .unwrap();
        assert!(p.channel_mutes.contains_key("other"));
        let value = raw(&path).unwrap();
        assert_eq!(value["future"]["preserve"], true);
        assert_eq!(value["sendTypingIndicators"], false);
        assert_eq!(value["channelNotificationModes"][CHANNEL], "mention");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
