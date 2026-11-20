//! Message send and history APIs with Tauri-compatible payloads and normalization.

use std::collections::HashMap;

use serde_json::Value;

use super::models::{MessagePage, MessageQuery, SendMessageRequest, TimelineMessage};
use super::normalize::{member_user_id, normalize_message, response_parts, valid_id};
use super::request::{
    authenticated_context, consume_json, execute_authenticated, invalid_input, request_messages,
};
use crate::error::CommandError;
use crate::http::{HttpPolicy, RequestClass};
use crate::server::ServerSelection;

/// Send a plain text message without a reply target.
pub async fn send_text(
    policy: &HttpPolicy,
    channel: &str,
    content: &str,
    key: &str,
) -> Result<TimelineMessage, CommandError> {
    send_text_with_reply(policy, channel, content, None, key).await
}

/// Send text with an optional, non-mentioning reply to a message in this channel.
pub async fn send_text_with_reply(
    policy: &HttpPolicy,
    channel: &str,
    content: &str,
    reply_to: Option<&str>,
    key: &str,
) -> Result<TimelineMessage, CommandError> {
    send_message(
        policy,
        SendMessageRequest {
            channel_id: channel.to_owned(),
            content: Some(content.trim().to_owned()),
            attachments: Vec::new(),
            reply_to: reply_to.map(str::to_owned),
            idempotency_key: key.to_owned(),
        },
    )
    .await
}

/// Send text, uploaded Autumn attachment IDs, or both, with an optional reply.
pub async fn send_message(
    policy: &HttpPolicy,
    request: SendMessageRequest,
) -> Result<TimelineMessage, CommandError> {
    let SendMessageRequest {
        channel_id,
        content,
        attachments,
        reply_to,
        idempotency_key,
    } = request;
    if !valid_id(&channel_id) || reply_to.as_deref().is_some_and(|id| !valid_id(id)) {
        return Err(invalid_input(
            "The message request contained an invalid ID.",
        ));
    }
    let key = reqwest::header::HeaderValue::from_str(&idempotency_key)
        .ok()
        .filter(|_| !idempotency_key.is_empty() && idempotency_key.len() <= 64)
        .ok_or_else(|| invalid_input("The message idempotency key is invalid."))?;
    if attachments
        .iter()
        .any(|id| id.trim().is_empty() || id.chars().any(char::is_control))
    {
        return Err(invalid_input(
            "The message contained an invalid attachment ID.",
        ));
    }
    let content = content
        .as_deref()
        .map(str::trim_end)
        .filter(|text| !text.is_empty());
    if content.is_none() && attachments.is_empty() {
        return Err(invalid_input("Write a message or attach a file."));
    }
    let context = authenticated_context(policy)?;
    crate::server::validate_selection(&context.server)?;
    let mut body = serde_json::json!({"content": content, "attachments": attachments});
    if let Some(id) = reply_to {
        body["replies"] = serde_json::json!([{
            "id": id,
            "mention": false,
            "fail_if_not_exists": false,
        }]);
    }
    let request = policy
        .client()
        .post(super::request::message_url(
            &context.server,
            &channel_id,
            None,
        )?)
        .header("x-session-token", &context.token)
        .header("Idempotency-Key", key)
        .json(&body);
    let response = execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Standard,
        "Could not send the message.",
    )
    .await?;
    consume_json(
        response,
        "The server rejected the message.",
        "Could not send the message.",
        |value| {
            normalize_message(
                &value,
                &HashMap::new(),
                &HashMap::new(),
                context.server.endpoints.autumn.as_deref(),
            )
        },
    )
}

#[cfg(test)]
mod composer_tests {
    use super::*;
    use crate::http::{ContextError, SessionAccess};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct Session(ServerSelection);
    impl SessionAccess for Session {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(self.0.clone()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            Ok(Some("local-test-token".into()))
        }
    }
    #[tokio::test]
    async fn text_send_posts_trimmed_content_with_an_idempotency_key() {
        assert_send(None, false).await;
    }

    #[tokio::test]
    async fn reply_send_posts_tauri_reply_flags_and_normalizes_the_response() {
        assert_send(Some("01M0P54M56DA883E6ZYDK00BKD"), false).await;
    }

    #[tokio::test]
    async fn attachment_only_send_posts_opaque_ids_with_null_content_and_reply_flags() {
        assert_send(Some("01M0P54M56DA883E6ZYDK00BKD"), true).await;
    }

    async fn assert_send(reply_to: Option<&'static str>, attachment_only: bool) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/api/", listener.local_addr().unwrap());
        let channel = "01M0P54M56DA883E6ZYDK00BKC";
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            loop {
                header.push(stream.read_u8().await.unwrap());
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
            assert!(header.starts_with(&format!(
                "post /api/channels/{}/messages",
                channel.to_lowercase()
            )));
            assert!(header.contains("idempotency-key: retry-key"));
            let size: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; size];
            stream.read_exact(&mut body).await.unwrap();
            let expected = match reply_to {
                Some(id) => serde_json::json!({"content": "hello", "attachments": [], "replies": [{
                    "id": id, "mention": false, "fail_if_not_exists": false,
                }]}),
                None => serde_json::json!({"content": "hello", "attachments": []}),
            };
            let mut expected = expected;
            if attachment_only {
                expected["content"] = Value::Null;
                expected["attachments"] = serde_json::json!(["autumn-file_id"]);
            }
            assert!(header.contains("x-session-token: local-test-token"));
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);
            let body = serde_json::json!({
                "_id": channel, "channel": channel, "author": channel, "content": "hello",
                "replies": reply_to.map(|id| serde_json::json!([id, {"id": channel}, null, "invalid", {"id": 5}])),
            })
            .to_string();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let selection=serde_json::from_value(serde_json::json!({"deploymentUrl":api,"displayName":"test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap();
        let policy = HttpPolicy::new(Arc::new(Session(selection))).unwrap();
        let message = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            if attachment_only {
                send_message(
                    &policy,
                    SendMessageRequest {
                        channel_id: channel.into(),
                        content: Some("  ".into()),
                        attachments: vec!["autumn-file_id".into()],
                        reply_to: reply_to.map(str::to_owned),
                        idempotency_key: "retry-key".into(),
                    },
                )
                .await
            } else {
                send_text_with_reply(&policy, channel, " hello ", reply_to, "retry-key").await
            }
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(message.content.as_deref(), Some("hello"));
        let expected_replies = reply_to.map_or_else(Vec::new, |id| vec![id, channel]);
        assert_eq!(message.reply_ids, expected_replies);
        server.await.unwrap();
        assert!(
            send_text_with_reply(&policy, channel, "hello", Some("invalid"), "retry-key")
                .await
                .is_err()
        );
        assert!(send_text(&policy, channel, "   ", "retry-key")
            .await
            .is_err());
    }
}

/// Fetch a page of channel history (`before`/`nearby` paging, 50 per page).
pub async fn fetch_messages(
    policy: &HttpPolicy,
    query: &MessageQuery,
) -> Result<MessagePage, CommandError> {
    if !valid_id(&query.channel_id)
        || query.before.as_deref().is_some_and(|id| !valid_id(id))
        || query.nearby.as_deref().is_some_and(|id| !valid_id(id))
        || (query.before.is_some() && query.nearby.is_some())
    {
        return Err(invalid_input(
            "The message request contained an invalid ID.",
        ));
    }
    let context = authenticated_context(policy)?;
    let response = request_messages(policy, &context, query).await?;
    consume_json(
        response,
        "The server rejected the message request.",
        "Could not fetch messages.",
        |value| normalize_page(value, &context.server),
    )
}

fn normalize_page(value: Value, server: &ServerSelection) -> Result<MessagePage, CommandError> {
    let (raw_messages, raw_users, raw_members) = response_parts(value);
    let users: HashMap<String, Value> = raw_users
        .into_iter()
        .filter_map(|user| {
            user.get("_id")
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), user.clone()))
        })
        .collect();
    let members: HashMap<String, Value> = raw_members
        .into_iter()
        .filter_map(|member| member_user_id(&member).map(|id| (id.to_owned(), member.clone())))
        .collect();
    let messages = raw_messages
        .iter()
        .map(|message| {
            normalize_message(
                message,
                &users,
                &members,
                server.endpoints.autumn.as_deref(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MessagePage {
        has_more: messages.len() == 50,
        messages,
    })
}

/// Fetch a single message (reply previews, jump-to-message).
pub async fn fetch_message(
    policy: &HttpPolicy,
    channel_id: &str,
    message_id: &str,
) -> Result<TimelineMessage, CommandError> {
    if !valid_id(channel_id) || !valid_id(message_id) {
        return Err(invalid_input(
            "The message request contained an invalid ID.",
        ));
    }
    let context = authenticated_context(policy)?;
    let request = policy
        .client()
        .get(super::request::message_url(
            &context.server,
            channel_id,
            Some(message_id),
        )?)
        .header("x-session-token", &context.token);
    let response = execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Standard,
        "Could not fetch the message.",
    )
    .await?;
    consume_json(
        response,
        "The message could not be fetched",
        "Could not fetch the message.",
        |value| {
            normalize_message(
                &value,
                &HashMap::new(),
                &HashMap::new(),
                context.server.endpoints.autumn.as_deref(),
            )
        },
    )
}
