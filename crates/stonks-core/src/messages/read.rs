//! Channel read state shared by native presenters. Cursors are monotonic
//! ULIDs; an ack clears only mentions at or before the confirmed cursor.
use super::{
    normalize::valid_id,
    request::{
        authenticated_context, execute_authenticated, invalid_input, message_status_error,
        unavailable,
    },
};
use crate::{
    connection::{AppSnapshot, ClientChannel, ClientUnread, ClientUnreadId, GatewayEvent},
    error::CommandError,
    http::{AuthenticatedContext, HttpPolicy, RequestClass},
};
use serde_json::Value;

pub async fn acknowledge(
    policy: &HttpPolicy,
    channel: &str,
    message: &str,
) -> Result<(), CommandError> {
    let context = authenticated_context(policy)?;
    acknowledge_in_context(policy, &context, channel, message).await
}

/// Bind a queued acknowledgement to the account that created the intent.
pub async fn acknowledge_in_context(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    channel: &str,
    message: &str,
) -> Result<(), CommandError> {
    if !valid_id(channel) || !valid_id(message) {
        return Err(invalid_input(
            "The acknowledgement contained an invalid ID.",
        ));
    }
    let mut url = url::Url::parse(&context.server.endpoints.api)
        .map_err(|_| invalid_input("The acknowledgement address was invalid."))?;
    url.path_segments_mut()
        .map_err(|_| invalid_input("The acknowledgement address was invalid."))?
        .pop_if_empty()
        .extend(["channels", channel, "ack", message]);
    let request = policy
        .client()
        .put(url)
        .header("x-session-token", &context.token);
    execute_authenticated(
        policy,
        context,
        request,
        RequestClass::Standard,
        "Could not mark the channel as read.",
    )
    .await?
    .consume(
        |status, _, _| {
            if status.is_success() {
                Ok(())
            } else {
                Err(message_status_error(
                    status,
                    "The channel could not be marked as read.",
                ))
            }
        },
        || unavailable("Could not mark the channel as read."),
    )
}

pub fn cursor<'a>(snapshot: &'a AppSnapshot, channel: &str) -> Option<&'a str> {
    snapshot
        .unreads
        .iter()
        .filter(|unread| unread.id.channel == channel && unread.id.user == snapshot.current_user.id)
        .filter_map(|unread| unread.last_id.as_deref())
        .max()
}

/// Project the sidebar badge without advancing the confirmed cursor.
pub fn unread_badge(snapshot: &AppSnapshot, channel: &ClientChannel) -> Option<usize> {
    if channel.is_voice() {
        return None;
    }
    let seen = cursor(snapshot, &channel.id);
    let newer = |message: &str| seen.is_none_or(|seen| message > seen);
    let mentions: std::collections::BTreeSet<_> = snapshot
        .unreads
        .iter()
        .filter(|unread| {
            unread.id.channel == channel.id && unread.id.user == snapshot.current_user.id
        })
        .flat_map(|unread| &unread.mentions)
        .filter(|id| newer(id))
        .collect();
    (channel.last_message_id.as_deref().is_some_and(newer) || !mentions.is_empty())
        .then_some(mentions.len())
}

pub fn advance(snapshot: &mut AppSnapshot, channel: &str, message: &str) -> bool {
    if !valid_id(channel) || !valid_id(message) {
        return false;
    }
    let old = cursor(snapshot, channel).map(str::to_owned);
    let latest = old
        .as_deref()
        .map_or(message, |old| old.max(message))
        .to_owned();
    let user = snapshot.current_user.id.clone();
    let mut mentions = std::collections::BTreeSet::new();
    for unread in &snapshot.unreads {
        if unread.id.channel == channel && unread.id.user == user {
            mentions.extend(
                unread
                    .mentions
                    .iter()
                    .filter(|id| id.as_str() > latest.as_str())
                    .cloned(),
            );
        }
    }
    snapshot
        .unreads
        .retain(|unread| unread.id.channel != channel || unread.id.user != user);
    snapshot.unreads.push(ClientUnread {
        id: ClientUnreadId {
            channel: channel.into(),
            user,
        },
        last_id: Some(latest.clone()),
        mentions: mentions.into_iter().collect(),
    });
    old.as_deref() != Some(latest.as_str())
}

/// Set the local read cursor to the previous visible message, or clear it if
/// there is none. Unlike `acknowledge`, this does not contact the server or clear mentions.
pub fn mark_unread(snapshot: &mut AppSnapshot, channel: &str, previous: Option<&str>) {
    if !valid_id(channel) || previous.is_some_and(|id| !valid_id(id)) {
        return;
    }
    let user = &snapshot.current_user.id;
    let mut found = false;
    for unread in &mut snapshot.unreads {
        if unread.id.channel == channel && unread.id.user == *user {
            unread.last_id = previous.map(str::to_owned);
            found = true;
        }
    }
    if !found {
        snapshot.unreads.push(ClientUnread {
            id: ClientUnreadId {
                channel: channel.into(),
                user: user.clone(),
            },
            last_id: previous.map(str::to_owned),
            mentions: vec![],
        });
    }
}

pub fn apply_event(snapshot: &mut AppSnapshot, event: &GatewayEvent) {
    match event.event_type.as_str() {
        "ChannelAck" if event.data["user"].as_str() == Some(snapshot.current_user.id.as_str()) => {
            if let (Some(channel), Some(message)) = (
                event.data["id"].as_str(),
                event
                    .data
                    .get("message_id")
                    .or_else(|| event.data.get("messageId"))
                    .and_then(Value::as_str),
            ) {
                advance(snapshot, channel, message);
            }
        }
        "Message" => {
            // Call starts are excluded from reading history and can never be
            // painted as an ack target. Keep the previous readable latest ID.
            if event.data["system"]["type"].as_str() == Some("call_started") {
                return;
            }
            let (Some(channel), Some(message)) =
                (event.data["channel"].as_str(), event.data["_id"].as_str())
            else {
                return;
            };
            if !valid_id(message) {
                return;
            }
            let Some(index) = snapshot
                .channels
                .iter()
                .position(|entry| entry.id == channel)
            else {
                return;
            };
            if snapshot.channels[index]
                .last_message_id
                .as_deref()
                .is_none_or(|old| message > old)
            {
                snapshot.channels[index].last_message_id = Some(message.into());
            }
            if cursor(snapshot, channel).is_some_and(|old| old >= message) {
                return;
            }
            if !snapshot.unreads.iter().any(|unread| {
                unread.id.channel == channel && unread.id.user == snapshot.current_user.id
            }) {
                snapshot.unreads.push(ClientUnread {
                    id: ClientUnreadId {
                        channel: channel.into(),
                        user: snapshot.current_user.id.clone(),
                    },
                    last_id: None,
                    mentions: vec![],
                });
            }
            if event.data["author"].as_str() == Some(snapshot.current_user.id.as_str()) {
                return;
            }
            let me = &snapshot.current_user.id;
            let direct = event.data["mentions"]
                .as_array()
                .is_some_and(|mentions| mentions.iter().any(|id| id.as_str() == Some(me)));
            let roles = snapshot.channels[index]
                .server
                .as_ref()
                .and_then(|server| {
                    snapshot
                        .members
                        .iter()
                        .find(|member| &member.id.server == server && &member.id.user == me)
                })
                .map(|member| &member.roles);
            let role = event.data["role_mentions"]
                .as_array()
                .is_some_and(|mentions| {
                    roles.is_some_and(|roles| {
                        mentions.iter().any(|id| {
                            id.as_str()
                                .is_some_and(|id| roles.iter().any(|role| role == id))
                        })
                    })
                });
            let mass = snapshot.channels[index].server.is_some()
                && event.data["content"].as_str().is_some_and(|text| {
                    let text = without_code(text);
                    text.contains("@everyone") || text.contains("@online")
                });
            if !(direct || role || mass) {
                return;
            }
            if let Some(unread) = snapshot
                .unreads
                .iter_mut()
                .find(|unread| unread.id.channel == channel && unread.id.user == *me)
            {
                if !unread.mentions.iter().any(|id| id == message) {
                    unread.mentions.push(message.into());
                }
            } else {
                snapshot.unreads.push(ClientUnread {
                    id: ClientUnreadId {
                        channel: channel.into(),
                        user: me.clone(),
                    },
                    last_id: None,
                    mentions: vec![message.into()],
                });
            }
        }
        "MessageDelete" => {
            let (Some(channel), Some(message)) =
                (event.data["channel"].as_str(), event.data["id"].as_str())
            else {
                return;
            };
            for unread in &mut snapshot.unreads {
                if unread.id.channel == channel {
                    unread.mentions.retain(|id| id != message);
                }
            }
            if let Some(channel) = snapshot
                .channels
                .iter_mut()
                .find(|entry| entry.id == channel)
            {
                if channel.last_message_id.as_deref() == Some(message) {
                    channel.last_message_id = None;
                }
            }
        }
        _ => {}
    }
}

fn without_code(mut source: &str) -> String {
    let mut output = String::new();
    while let Some(ch) = source.chars().next() {
        source = &source[ch.len_utf8()..];
        if ch == '\\' {
            if let Some(escaped) = source.chars().next() {
                source = &source[escaped.len_utf8()..];
            }
            continue;
        }
        if ch != '`' {
            output.push(ch);
            continue;
        }
        let mut size = 1;
        while size < 3 && source.starts_with('`') {
            size += 1;
            source = &source[1..];
        }
        let marker = "`".repeat(size);
        if let Some(end) = source.find(&marker) {
            source = &source[end + size..];
        } else {
            output.push_str(&marker);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::{ContextError, SessionAccess},
        server::ServerSelection,
    };
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
    fn id(n: u8) -> String {
        format!("{n:026}")
    }
    fn snapshot() -> AppSnapshot {
        AppSnapshot {
            current_user: serde_json::from_value(serde_json::json!({"_id":"me","username":"Me"}))
                .unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![serde_json::from_value(
                serde_json::json!({"_id":CHANNEL,"channel_type":"TextChannel","server":"server"}),
            )
            .unwrap()],
            members: vec![serde_json::from_value(
                serde_json::json!({"_id":{"server":"server","user":"me"},"roles":["role"]}),
            )
            .unwrap()],
            unreads: vec![],
            voice_states: vec![],
        }
    }
    fn event(kind: &str, data: Value) -> GatewayEvent {
        GatewayEvent {
            event_type: kind.into(),
            data,
        }
    }
    #[test]
    fn mark_unread_rewinds_and_clears_only_the_current_users_cursor() {
        let mut snapshot = snapshot();
        snapshot.channels[0].last_message_id = Some(id(5));
        mark_unread(&mut snapshot, CHANNEL, Some(&id(5)));
        snapshot.unreads[0].mentions = vec![id(4)];
        let mut other = snapshot.unreads[0].clone();
        other.id.user = "someone-else".into();
        snapshot.unreads.push(other);
        mark_unread(&mut snapshot, CHANNEL, Some(&id(3)));
        assert_eq!(cursor(&snapshot, CHANNEL), Some(id(3).as_str()));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), Some(1));
        mark_unread(&mut snapshot, CHANNEL, None);
        assert_eq!(cursor(&snapshot, CHANNEL), None);
        assert_eq!(snapshot.unreads[0].mentions, [id(4)]);
        assert_eq!(snapshot.unreads[1].last_id, Some(id(5)));
    }

    #[test]
    fn voice_channels_have_no_unread_badge_even_with_message_activity() {
        let mut snapshot = snapshot();
        snapshot.channels[0].last_message_id = Some(id(2));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), Some(0));
        snapshot.channels[0].voice = Some(serde_json::json!({}));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
        snapshot.channels[0].voice = None;
        snapshot.channels[0].channel_type = "VoiceChannel".into();
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
    }

    #[test]
    fn sidebar_badge_requires_activity_strictly_newer_than_confirmed_cursor() {
        let mut snapshot = snapshot();
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
        snapshot.channels[0].last_message_id = Some(id(2));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), Some(0));
        advance(&mut snapshot, CHANNEL, &id(3));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
        snapshot.channels[0].last_message_id = Some(id(3));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
        snapshot.channels[0].last_message_id = Some(id(5));
        snapshot.unreads[0].mentions = vec![id(2), id(5), id(5)];
        let mut other = snapshot.unreads[0].clone();
        other.id.user = "other".into();
        other.last_id = Some(id(9));
        other.mentions = vec![id(6)];
        snapshot.unreads.insert(0, other);
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), Some(1));
        advance(&mut snapshot, CHANNEL, &id(5));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
    }

    #[test]
    fn live_call_start_neither_creates_unread_nor_masks_previous_unread() {
        let mut snapshot = snapshot();
        let call = event(
            "Message",
            serde_json::json!({"_id":id(5),"channel":CHANNEL,"author":"other",
                "system":{"type":"call_started","by":"other"}}),
        );
        apply_event(&mut snapshot, &call);
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
        snapshot.channels[0].last_message_id = Some(id(4));
        apply_event(&mut snapshot, &call);
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), Some(0));
        advance(&mut snapshot, CHANNEL, &id(4));
        assert_eq!(unread_badge(&snapshot, &snapshot.channels[0]), None);
    }

    #[test]
    fn confirmed_cursor_never_regresses_and_keeps_newer_mentions() {
        let mut snapshot = snapshot();
        advance(&mut snapshot, CHANNEL, &id(2));
        snapshot.unreads[0].mentions = vec![id(1), id(3), id(5)];
        assert!(advance(&mut snapshot, CHANNEL, &id(3)));
        assert_eq!(snapshot.unreads[0].mentions, [id(5)]);
        assert!(!advance(&mut snapshot, CHANNEL, &id(1)));
        assert_eq!(cursor(&snapshot, CHANNEL), Some(id(3).as_str()));
    }
    #[test]
    fn remote_ack_is_scoped_to_the_current_user() {
        let mut snapshot = snapshot();
        apply_event(
            &mut snapshot,
            &event(
                "ChannelAck",
                serde_json::json!({"id":CHANNEL,"user":"another","message_id":id(8)}),
            ),
        );
        assert!(snapshot.unreads.is_empty());
        apply_event(
            &mut snapshot,
            &event(
                "ChannelAck",
                serde_json::json!({"id":CHANNEL,"user":"me","messageId":id(4)}),
            ),
        );
        assert_eq!(cursor(&snapshot, CHANNEL), Some(id(4).as_str()));
    }
    #[test]
    fn live_messages_create_unread_and_deduplicate_mentions() {
        let mut snapshot = snapshot();
        apply_event(
            &mut snapshot,
            &event(
                "Message",
                serde_json::json!({"_id":id(2),"channel":CHANNEL,"author":"other","content":"hello"}),
            ),
        );
        assert_eq!(snapshot.unreads.len(), 1);
        assert!(snapshot.unreads[0].last_id.is_none());
        let mention = event(
            "Message",
            serde_json::json!({"_id":id(3),"channel":CHANNEL,"author":"other","mentions":["me"]}),
        );
        apply_event(&mut snapshot, &mention);
        apply_event(&mut snapshot, &mention);
        assert_eq!(snapshot.unreads[0].mentions, [id(3)]);
        assert_eq!(
            snapshot.channels[0].last_message_id.as_deref(),
            Some(id(3).as_str())
        );
        advance(&mut snapshot, CHANNEL, &id(3));
        apply_event(&mut snapshot, &mention);
        assert!(snapshot.unreads[0].mentions.is_empty());
    }
    #[test]
    fn role_mass_mentions_and_deletes_match_read_projection() {
        let mut snapshot = snapshot();
        for (n, data) in [
            (1, serde_json::json!({"role_mentions":["role"]})),
            (2, serde_json::json!({"content":"`@everyone`"})),
            (3, serde_json::json!({"content":"@online"})),
        ] {
            let mut data = data;
            data["_id"] = id(n).into();
            data["channel"] = CHANNEL.into();
            data["author"] = "other".into();
            apply_event(&mut snapshot, &event("Message", data));
        }
        assert_eq!(snapshot.unreads[0].mentions, [id(1), id(3)]);
        apply_event(
            &mut snapshot,
            &event(
                "MessageDelete",
                serde_json::json!({"id":id(3),"channel":CHANNEL}),
            ),
        );
        assert_eq!(snapshot.unreads[0].mentions, [id(1)]);
        assert_eq!(
            without_code("Olá \\@everyone ```@online``` @everyone"),
            "Olá everyone  @everyone"
        );
    }
    struct TestSession(ServerSelection);
    impl SessionAccess for TestSession {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(self.0.clone()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            Ok(Some("local-test-token".into()))
        }
    }
    fn policy(api: &str) -> HttpPolicy {
        HttpPolicy::new(Arc::new(TestSession(serde_json::from_value(serde_json::json!({"deploymentUrl":api,"displayName":"Test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))).unwrap()
    }
    #[tokio::test]
    async fn ack_puts_the_exact_cursor_and_accepts_empty_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/api/", listener.local_addr().unwrap());
        let target = id(5);
        let expected = target.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                bytes.push(stream.read_u8().await.unwrap());
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            assert!(request.starts_with(&format!(
                "put /api/channels/{}/ack/{expected}",
                CHANNEL.to_ascii_lowercase()
            )));
            assert!(request.contains("x-session-token: local-test-token"));
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            acknowledge(&policy(&api), CHANNEL, &target),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        assert!(acknowledge(&HttpPolicy::default(), "bad", &target)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn logout_cancels_inflight_acknowledgement() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = Arc::new(policy(&format!(
            "http://{}",
            listener.local_addr().unwrap()
        )));
        let request_policy = policy.clone();
        let target = id(5);
        let request =
            tokio::spawn(async move { acknowledge(&request_policy, CHANNEL, &target).await });
        let (_stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
        policy.logout();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3), request)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }
}
