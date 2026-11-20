use super::*;
use crate::{
    http::{ContextError, SessionAccess},
    server::ServerSelection,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
struct Session(ServerSelection);
impl SessionAccess for Session {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(Some(self.0.clone()))
    }
    fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
        Ok(Some("local-token".into()))
    }
}
pub(super) fn policy(api: &str) -> HttpPolicy {
    HttpPolicy::new(Arc::new(Session(serde_json::from_value(json!({"deploymentUrl":api,"displayName":"Test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))).unwrap()
}
#[test]
fn overview_preserves_description_and_validates_native_bounds() {
    let mut overview = Overview {
        name: " Name ".into(),
        description: " description ".into(),
        slowmode: 21600,
        nsfw: true,
    };
    assert_eq!(
        overview.body().unwrap(),
        json!({"name":"Name","description":" description ","slowmode":21600,"nsfw":true})
    );
    overview.name = "é".repeat(32);
    assert!(overview.body().is_ok());
    overview.name.push('a');
    assert!(overview.body().is_err());
    overview.name = "valid".into();
    overview.description = "a".repeat(1025);
    assert!(overview.body().is_err());
    overview.description.clear();
    overview.slowmode = 21601;
    assert!(overview.body().is_err());
}
#[test]
fn validates_safe_native_ids_and_response_identity() {
    assert!(valid_id("Channel123"));
    assert!(!valid_id("../channel"));
    assert!(!valid_id(&"A".repeat(65)));
    assert!(channel(
        Some(json!({"_id":"other","channel_type":"TextChannel"})),
        CHANNEL
    )
    .is_err());
    assert!(channel(None, CHANNEL).is_err());
}
#[tokio::test]
async fn native_request_contract_and_safe_permission_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}/api/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            bytes.push(stream.read_u8().await.unwrap());
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
        assert!(headers.starts_with(&format!(
            "put /api/channels/{}/permissions/default",
            CHANNEL.to_ascii_lowercase()
        )));
        assert!(headers.contains("x-session-token: local-token"));
        let size = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"permissions":{"allow":1u64<<41,"deny":2}})
        );
        let body = "private backend details";
        stream
            .write_all(
                format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let policy = policy(&api);
    let context = context(&policy).unwrap();
    let failure = execute(
        &policy,
        &context,
        request(
            &policy,
            &context,
            Method::PUT,
            &["channels", CHANNEL, "permissions", "default"],
            Some(json!({"permissions":{"allow":1u64<<41,"deny":2}})),
        )
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(failure.to_string().contains("permission"));
    assert!(!failure.to_string().contains("private"));
    server.await.unwrap();
}

pub(super) fn manager() -> ConnectionManager {
    let manager = ConnectionManager::default();
    let generation = manager.begin(Some("user".into())).unwrap();
    manager.publish_ready(generation,json!({"currentUser":{"_id":"user"},"ready":{"servers":[{"_id":"server","owner":"user","roles":{}}],"channels":[{"_id":CHANNEL,"server":"server"}],"members":[]}}),||Ok(())).unwrap();
    manager
}
#[test]
fn authorization_is_checked_before_a_mutating_request() {
    let unavailable = ConnectionManager::default();
    assert_eq!(
        ChannelAccess::from_manager(&unavailable, CHANNEL),
        ChannelAccess::default()
    );
    let owner = manager();
    let access = ChannelAccess::from_manager(&owner, CHANNEL);
    assert!(
        access.settings()
            && access.manage_channel
            && access.manage_permissions
            && access.manage_messages
            && access.invite
    );
}
#[tokio::test]
async fn patch_updates_all_source_fields_and_checks_response_identity() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            bytes.push(stream.read_u8().await.unwrap());
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
        assert!(headers.starts_with(&format!("patch /channels/{}", CHANNEL.to_ascii_lowercase())));
        let size = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"name":"Name","description":"description","slowmode":30,"nsfw":true})
        );
        let body=json!({"_id":CHANNEL,"channel_type":"TextChannel","name":"Name","description":"description","slowmode":30,"nsfw":true}).to_string();
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
    let overview = Overview {
        name: " Name ".into(),
        description: "description".into(),
        slowmode: 30,
        nsfw: true,
    };
    let result = update(&policy(&api), &manager(), CHANNEL, &overview)
        .await
        .unwrap();
    assert_eq!(result.slowmode, Some(30));
    assert!(result.nsfw);
    server.await.unwrap();
    assert!(update(
        &HttpPolicy::default(),
        &ConnectionManager::default(),
        CHANNEL,
        &overview
    )
    .await
    .is_err());
}
