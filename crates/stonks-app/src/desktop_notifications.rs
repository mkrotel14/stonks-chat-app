//! Message notification policy and native desktop delivery. Navigation stays in the shell.
use futures_channel::{
    mpsc::{Receiver, Sender},
    oneshot,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use futures_util::FutureExt;
use futures_util::StreamExt;
#[cfg(target_os = "linux")]
use std::collections::VecDeque;
use std::sync::Arc;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use stonks_core::preferences::voice_sounds::{Cue, Store};
use stonks_core::{
    channel_preferences::{ChannelPreferences, NotificationMode},
    http::{AuthenticatedContext, HttpPolicy},
    preferences::Preferences,
};
use url::Url;
#[cfg(target_os = "linux")]
use zbus::zvariant::Value;

#[derive(Clone, Copy)]
pub(super) struct Policy {
    desktop_notifications: bool,
    mentions_only: bool,
    direct_message_sounds: bool,
    server_message_sounds: bool,
    mention_sounds: bool,
}

impl From<&Preferences> for Policy {
    fn from(saved: &Preferences) -> Self {
        Self {
            desktop_notifications: saved.desktop_notifications,
            mentions_only: saved.mentions_only,
            direct_message_sounds: saved.direct_message_sounds,
            server_message_sounds: saved.server_message_sounds,
            mention_sounds: saved.mention_sounds,
        }
    }
}

impl Policy {
    pub(super) fn allows(
        self,
        foreground: bool,
        channels: &ChannelPreferences,
        channel: &str,
        mentioned: bool,
        now: i64,
    ) -> bool {
        self.desktop_notifications
            && !foreground
            && !channels.is_muted(channel, now)
            && match channels.channel_notification_modes.get(channel) {
                Some(NotificationMode::None) => false,
                Some(NotificationMode::Mention) => mentioned,
                Some(NotificationMode::All) => true,
                None => !self.mentions_only || mentioned,
            }
    }

    pub(super) fn sound(self, direct: bool, mentioned: bool) -> bool {
        if mentioned {
            self.mention_sounds
        } else if direct {
            self.direct_message_sounds
        } else {
            self.server_message_sounds
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Route {
    Server(String),
    Channel {
        server: Option<String>,
        channel: String,
        message: Option<String>,
    },
}

/// Inbound web URLs must belong to the active deployment. Never pass these
/// strings to a system opener or use them to select a different deployment.
pub(super) fn parse_route(value: &str, deployment: &str) -> Option<Route> {
    if value.len() > 2048
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['\\', '%', '?', '#'])
        || value.starts_with("//")
        || value
            .split('/')
            .any(|component| matches!(component, "." | ".."))
    {
        return None;
    }
    let base = Url::parse(deployment).ok()?;
    let url = if value.starts_with('/') {
        base.join(value).ok()?
    } else {
        Url::parse(value).ok()?
    };
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let path = match url.scheme() {
        "stonks" if url.port().is_none() => format!("/{}{}", url.host_str()?, url.path()),
        "http" | "https" if url.origin() == base.origin() => url.path().to_owned(),
        _ => return None,
    };
    let parts: Vec<_> = path
        .strip_prefix('/')?
        .trim_end_matches('/')
        .split('/')
        .collect();
    match parts.as_slice() {
        ["server", server] if valid_id(server) => Some(Route::Server((*server).into())),
        ["channel", channel, tail @ ..] => channel_route(None, channel, tail),
        ["server", server, "channel", channel, tail @ ..] if valid_id(server) => {
            channel_route(Some((*server).into()), channel, tail)
        }
        _ => None,
    }
}

fn valid_id(value: &str) -> bool {
    value.len() == 26 && value.bytes().all(|byte| {
        matches!(byte.to_ascii_uppercase(), b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z')
    })
}

fn channel_route(server: Option<String>, channel: &str, tail: &[&str]) -> Option<Route> {
    if !valid_id(channel) {
        return None;
    }
    let message = match tail {
        [] => None,
        [id] if valid_id(id) => Some((*id).into()),
        _ => return None,
    };
    Some(Route::Channel {
        server,
        channel: channel.into(),
        message,
    })
}

pub(super) struct Target {
    pub route: Route,
    pub user: String,
    pub account: Arc<AuthenticatedContext>,
}

pub(super) struct Delivery {
    pub title: String,
    pub body: String,
    pub sound: bool,
    pub target: Target,
}

pub(super) fn preview_body(
    show_content: bool,
    sender: Option<&str>,
    content: impl FnOnce() -> String,
) -> String {
    if !show_content {
        return stonks_ui::i18n::t("notifications.hiddenContent");
    }
    let content = content();
    match sender {
        Some(sender) => format!("{sender}: {content}"),
        None => content,
    }
}

/// Notification bodies can be parsed as markup by the desktop service.
#[cfg(any(target_os = "linux", test))]
fn plain_body(text: &str) -> String {
    let mut out = String::new();
    for c in text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(500)
    {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app: &str,
        replaces: u32,
        icon: &str,
        title: &str,
        body: &str,
        actions: &[&str],
        hints: std::collections::HashMap<&str, Value<'_>>,
        timeout: i32,
    ) -> zbus::Result<u32>;
    fn close_notification(&self, id: u32) -> zbus::Result<()>;
    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

#[cfg(target_os = "linux")]
pub(super) async fn deliver(
    policy: Arc<HttpPolicy>,
    mut requests: Receiver<Delivery>,
    mut clicks: Sender<Target>,
    stop: oneshot::Receiver<()>,
) -> zbus::Result<()> {
    let conn = zbus::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(3))
        .build()
        .await?;
    // Pin calls and signals to the same daemon instance: IDs may be reused
    // after a daemon restart, and must not activate old account routes.
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let owner = dbus
        .get_name_owner("org.freedesktop.Notifications".try_into()?)
        .await?;
    let proxy = NotificationsProxy::builder(&conn)
        .destination(owner)?
        .build()
        .await?;
    let mut actions = proxy.receive_action_invoked().await?.fuse();
    let mut closed = proxy.receive_notification_closed().await?.fuse();
    let mut owner_changes = proxy.inner().receive_owner_changed().await?.fuse();
    let mut pending: VecDeque<(u32, Target)> = VecDeque::new();
    let mut sound = None::<tokio::task::JoinHandle<()>>;
    let mut stop = stop.fuse();
    loop {
        futures_util::select_biased! {
            _ = stop => break,
            request = requests.next() => {
                let Some(request) = request else { break; };
                if !policy.is_authenticated_context_current(&request.target.account) { continue; }
                let title: String = request.title.chars().filter(|c| !c.is_control()).take(120).collect();
                let hints = [
                    ("desktop-entry", Value::from("Stonks Chat")),
                    ("urgency", Value::from(1u8)),
                    ("suppress-sound", Value::from(true)),
                ].into_iter().collect();
                let id = proxy.notify("Stonks Chat", 0, "dialog-information", &title,
                    &plain_body(&request.body), &["default", "Open"], hints, 0).await;
                let Ok(id) = id else { continue; };
                pending.retain(|(old, _)| *old != id);
                let account_current = policy.is_authenticated_context_current(&request.target.account);
                pending.push_back((id, request.target));
                if pending.len() > 64 {
                    if let Some((old, _)) = pending.pop_front() { let _ = proxy.close_notification(old).await; }
                }
                if request.sound && account_current && sound.as_ref().is_none_or(|task| task.is_finished()) {
                    sound = Some(tokio::task::spawn_blocking(|| {
                        let _ = Store::local().and_then(|store| store.preview(Cue::Message));
                    }));
                }
            },
            action = actions.next() => {
                let Some(action) = action else { break; };
                if let Ok(args) = action.args() {
                    if args.action_key() == &"default" {
                        if let Some(index) = pending.iter().position(|(id, _)| id == args.id()) {
                            if let Some((_, target)) = pending.remove(index) { let _ = clicks.try_send(target); }
                            let _ = proxy.close_notification(*args.id()).await;
                        }
                    }
                }
            },
            event = closed.next() => {
                let Some(event) = event else { break; };
                if let Ok(args) = event.args() { pending.retain(|(id, _)| id != args.id()); }
            },
            _ = owner_changes.next() => break,
        }
    }
    for (id, _) in pending {
        let _ = proxy.close_notification(id).await;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) async fn deliver(
    policy: Arc<HttpPolicy>,
    mut requests: Receiver<Delivery>,
    mut clicks: Sender<Target>,
    stop: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    use crate::desktop::tray_windows::{notify, Balloon};
    let mut stop = stop.fuse();
    let mut sound_task = None::<tokio::task::JoinHandle<()>>;
    loop {
        let request = futures_util::select_biased! {
            _ = stop => break,
            request = requests.next() => request,
        };
        let Some(request) = request else {
            break;
        };
        if !policy.is_authenticated_context_current(&request.target.account) {
            continue;
        }
        let (clicked, reply) = oneshot::channel();
        let (shown, shown_reply) = oneshot::channel();
        let current_policy = policy.clone();
        let account = request.target.account.clone();
        if !notify(Balloon {
            title: request.title,
            body: request.body,
            current: Box::new(move || current_policy.is_authenticated_context_current(&account)),
            shown: Some(shown),
            clicked,
        }) {
            continue;
        }
        let accepted = futures_util::select_biased! {
            _ = stop => break,
            result = shown_reply.fuse() => result.is_ok(),
        };
        if !accepted {
            continue;
        }
        if request.sound
            && policy.is_authenticated_context_current(&request.target.account)
            && sound_task.as_ref().is_none_or(|task| task.is_finished())
        {
            sound_task = Some(tokio::task::spawn_blocking(|| {
                let _ = Store::local().and_then(|store| store.preview(Cue::Message));
            }));
        }
        let activated = futures_util::select_biased! {
            _ = stop => break,
            result = reply.fuse() => result == Ok(true),
        };
        if activated && policy.is_authenticated_context_current(&request.target.account) {
            let _ = clicks.try_send(request.target);
        }
    }
    Ok(())
}

// Keep the shell's worker lifecycle intact until a native delivery backend exists.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) async fn deliver(
    _policy: Arc<HttpPolicy>,
    mut requests: Receiver<Delivery>,
    _clicks: Sender<Target>,
    stop: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    use futures_util::FutureExt;
    let mut stop = stop.fuse();
    loop {
        futures_util::select_biased! {
            _ = stop => break,
            request = requests.next() => match request {
                Some(Delivery { title, body, sound, target }) => drop((title, body, sound, target)),
                None => break,
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stonks_core::channel_preferences::Change;

    #[test]
    fn foreground_disable_mutes_and_channel_overrides() {
        let policy = Policy::from(
            &stonks_core::preferences::parse(r#"{"mentionsOnly":true}"#.as_bytes()).unwrap(),
        );
        let mut channels = ChannelPreferences::default();
        assert!(!policy.allows(false, &channels, "c", false, 0));
        assert!(policy.allows(false, &channels, "c", true, 0));
        channels.apply("c", Change::Notifications(Some(NotificationMode::All)));
        assert!(policy.allows(false, &channels, "c", false, 0));
        assert!(!policy.allows(true, &channels, "c", true, 0));
        channels.apply("c", Change::Mute(Some(Some(5))));
        assert!(!policy.allows(false, &channels, "c", true, 4));
        assert!(policy.allows(false, &channels, "c", false, 5));
        channels.apply("c", Change::Notifications(Some(NotificationMode::None)));
        assert!(!policy.allows(false, &channels, "c", true, 5));
        let disabled = Policy::from(
            &stonks_core::preferences::parse(r#"{"desktopNotifications":false}"#.as_bytes())
                .unwrap(),
        );
        assert!(!disabled.allows(false, &ChannelPreferences::default(), "c", true, 5));
    }

    #[test]
    fn mention_sound_takes_precedence_over_channel_category() {
        let policy = Policy::from(
            &stonks_core::preferences::parse(
                r#"{"mentionSounds":false,"serverMessageSounds":false}"#.as_bytes(),
            )
            .unwrap(),
        );
        assert!(policy.sound(true, false));
        assert!(!policy.sound(true, true));
        assert!(!policy.sound(false, false));
    }

    #[test]
    fn routes_only_the_active_deployment_or_relative_and_stonks_routes() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let base = "https://chat.example:8443";
        let path = format!("/server/{id}/channel/{id}/{id}");
        let expected = Some(Route::Channel {
            server: Some(id.into()),
            channel: id.into(),
            message: Some(id.into()),
        });
        assert_eq!(parse_route(&path, base), expected);
        assert_eq!(parse_route(&format!("{base}{path}"), base), expected);
        assert_eq!(parse_route(&format!("stonks:/{path}"), base), expected);
        for invalid in [
            format!("https://chat.example{path}"),
            format!("/{base}{path}"),
            format!("//chat.example:8443{path}"),
            format!("{path}?"),
            format!("{path}%2F"),
            format!("{base}/../{path}"),
            format!("/channel/{id}/extra"),
            format!("https://user@chat.example:8443{path}"),
        ] {
            assert_eq!(parse_route(&invalid, base), None, "{invalid}");
        }
    }

    #[test]
    fn hidden_preview_does_not_read_content_or_include_sender() {
        let body = preview_body(false, Some("private-name"), || panic!("content read"));
        assert_eq!(body, stonks_ui::i18n::t("notifications.hiddenContent"));
        assert!(!body.contains("private-name"));
        assert_eq!(
            preview_body(true, Some("Sender"), || "Text".into()),
            "Sender: Text"
        );
    }

    #[test]
    fn remote_notification_content_is_plain_and_bounded() {
        assert_eq!(
            plain_body("<a href='https://example.com'>A&B</a>\0"),
            "&lt;a href='https://example.com'&gt;A&amp;B&lt;/a&gt;"
        );
        assert_eq!(plain_body(&"é".repeat(600)).chars().count(), 500);
    }
}
