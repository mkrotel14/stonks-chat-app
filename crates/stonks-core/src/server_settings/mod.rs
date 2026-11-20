//! Server overview edits and read-only settings data, independent of Tauri.
use crate::{
    connection::ClientServer,
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass, MAX_JSON_RESPONSE_BYTES},
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use url::Url;

mod mutations;
pub use mutations::*;

pub const SYSTEM_EVENTS: [&str; 4] = ["user_joined", "user_left", "user_kicked", "user_banned"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub name: String,
    pub description: String,
    pub system_messages: BTreeMap<String, String>,
}
impl Overview {
    pub fn from_server(server: &ClientServer) -> Self {
        Self {
            name: server.name.clone(),
            description: server.description.clone().unwrap_or_default(),
            system_messages: SYSTEM_EVENTS
                .iter()
                .map(|key| {
                    (
                        key.to_string(),
                        server
                            .system_messages
                            .as_ref()
                            .and_then(|channels| channels.get(*key))
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect(),
        }
    }
    pub fn body(&self) -> Result<Value, CommandError> {
        if self.name.trim().is_empty()
            || self.name.trim().chars().count() > 32
            || self.description.chars().count() > 1024
        {
            return Err(error(
                ErrorCode::InvalidInput,
                "The server name or description is invalid.",
            ));
        }
        if self.system_messages.iter().any(|(key, value)| {
            !SYSTEM_EVENTS.contains(&key.as_str()) || (!value.is_empty() && !valid_id(value))
        }) {
            return Err(error(
                ErrorCode::InvalidInput,
                "A system-message channel is invalid.",
            ));
        }
        Ok(
            json!({"name":self.name.trim(),"description":self.description.trim(),"system_messages":self.system_messages}),
        )
    }
}
fn error(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, message)
}
fn invalid() -> CommandError {
    error(
        ErrorCode::InvalidResponse,
        "The server returned invalid settings data.",
    )
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

async fn request(
    policy: &HttpPolicy,
    method: Method,
    path: &[&str],
    body: Option<Value>,
) -> Result<Option<Value>, CommandError> {
    let context = policy.authenticated_context().map_err(|_| {
        error(
            ErrorCode::AuthenticationFailed,
            "Log in again to access server settings.",
        )
    })?;
    let mut url = Url::parse(&context.server.endpoints.api).map_err(|_| {
        error(
            ErrorCode::NotConfigured,
            "The selected server API address is invalid.",
        )
    })?;
    url.path_segments_mut()
        .map_err(|_| invalid())?
        .pop_if_empty()
        .extend(path);
    let mut request = policy
        .client()
        .request(method, url)
        .header("x-session-token", &context.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    policy
        .execute_authenticated(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Server settings could not be reached. Try again.",
            )
        })?
        .consume(
            |status, _, bytes| {
                if !status.is_success() {
                    return Err(match status.as_u16() {
                        401 => error(
                            ErrorCode::AuthenticationFailed,
                            "Log in again to access server settings.",
                        ),
                        403 => error(
                            ErrorCode::AuthenticationFailed,
                            "Your account does not have permission to manage this server.",
                        ),
                        404 => error(
                            ErrorCode::NotFound,
                            "This server settings resource is no longer available.",
                        ),
                        _ => error(
                            ErrorCode::Unavailable,
                            "The server rejected the settings request.",
                        ),
                    });
                }
                if bytes.is_empty() {
                    Ok(None)
                } else {
                    serde_json::from_slice(bytes)
                        .map(Some)
                        .map_err(|_| invalid())
                }
            },
            || {
                error(
                    ErrorCode::Unavailable,
                    "The server settings request was cancelled.",
                )
            },
        )
}

/// Compatible deployments may return an empty/partial PATCH response. Read
/// the canonical server back before publishing a successful update.
pub async fn save_overview(
    policy: &HttpPolicy,
    server_id: &str,
    overview: &Overview,
) -> Result<ClientServer, CommandError> {
    if !valid_id(server_id) {
        return Err(error(ErrorCode::InvalidInput, "The server ID is invalid."));
    }
    let body = overview.body()?;
    let value = request(policy, Method::PATCH, &["servers", server_id], Some(body)).await?;
    if let Some(value) = value {
        if let Ok(server) = serde_json::from_value::<ClientServer>(value) {
            if server.id != server_id {
                return Err(invalid());
            }
            return Ok(server);
        }
    }
    let value = request(policy, Method::GET, &["servers", server_id], None)
        .await?
        .ok_or_else(invalid)?;
    let server: ClientServer = serde_json::from_value(value).map_err(|_| invalid())?;
    if server.id != server_id {
        return Err(invalid());
    }
    Ok(server)
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerEmoji {
    #[serde(rename = "_id", alias = "id")]
    pub id: String,
    pub name: String,
    #[serde(default, alias = "creatorId", alias = "creator")]
    pub creator_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ServerInvite {
    #[serde(rename = "_id", alias = "id", alias = "code")]
    pub code: String,
    #[serde(rename = "server", alias = "server_id", alias = "serverId")]
    pub server_id: String,
    #[serde(rename = "channel", alias = "channel_id", alias = "channelId")]
    pub channel_id: String,
    #[serde(rename = "creator", alias = "creator_id", alias = "creatorId")]
    pub creator_id: String,
    #[serde(default)]
    pub uses: Option<u64>,
    #[serde(default)]
    pub max_uses: Option<u64>,
    #[serde(default)]
    pub expires: Option<String>,
}
fn list<T: serde::de::DeserializeOwned>(value: Value, key: &str) -> Result<Vec<T>, CommandError> {
    let values = if value.is_array() {
        value
    } else {
        value.get(key).cloned().ok_or_else(invalid)?
    };
    serde_json::from_value(values).map_err(|_| invalid())
}
pub async fn fetch_emojis(
    policy: &HttpPolicy,
    server_id: &str,
) -> Result<Vec<ServerEmoji>, CommandError> {
    if !valid_id(server_id) {
        return Err(error(ErrorCode::InvalidInput, "The server ID is invalid."));
    }
    list(
        request(policy, Method::GET, &["servers", server_id, "emojis"], None)
            .await?
            .ok_or_else(invalid)?,
        "emojis",
    )
}
pub async fn fetch_invites(
    policy: &HttpPolicy,
    server_id: &str,
) -> Result<Vec<ServerInvite>, CommandError> {
    if !valid_id(server_id) {
        return Err(error(ErrorCode::InvalidInput, "The server ID is invalid."));
    }
    let invites: Vec<ServerInvite> = list(
        request(
            policy,
            Method::GET,
            &["servers", server_id, "invites"],
            None,
        )
        .await?
        .ok_or_else(invalid)?,
        "invites",
    )?;
    if invites.iter().any(|invite| invite.server_id != server_id) {
        return Err(invalid());
    }
    Ok(invites)
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
    const ID: &str = "01M0P54M56DA883E6ZYDK00BKC";
    struct Session(ServerSelection);
    impl SessionAccess for Session {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(self.0.clone()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            Ok(Some("test-session".into()))
        }
    }
    pub(super) fn policy(api: String) -> HttpPolicy {
        HttpPolicy::new(Arc::new(Session(serde_json::from_value(json!({"deploymentUrl":api,"displayName":"Test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))).unwrap()
    }
    fn overview() -> Overview {
        Overview {
            name: " New name ".into(),
            description: " New description ".into(),
            system_messages: BTreeMap::from([("user_joined".into(), ID.into())]),
        }
    }
    #[test]
    fn validates_names_description_and_system_channels_before_network() {
        assert_eq!(overview().body().unwrap()["name"], "New name");
        let mut value = overview();
        value.name = " ".into();
        assert!(value.body().is_err());
        value.name = "a".repeat(33);
        assert!(value.body().is_err());
        value.name = "valid".into();
        value.description = "x".repeat(1025);
        assert!(value.body().is_err());
        value.description.clear();
        value.system_messages.insert("unknown".into(), ID.into());
        assert!(value.body().is_err());
        value.system_messages = BTreeMap::from([("user_joined".into(), "".into())]);
        assert!(value.body().is_ok());
    }
    #[tokio::test]
    async fn patches_then_refreshes_after_empty_success_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/api/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                loop {
                    header.push(stream.read_u8().await.unwrap());
                    if header.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
                assert!(header.contains("x-session-token: test-session"));
                if index == 0 {
                    assert!(header.starts_with("patch /api/servers/"));
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).await.unwrap();
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(body["name"], "New name");
                    assert_eq!(body["system_messages"]["user_joined"], ID);
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                } else {
                    assert!(header.starts_with("get /api/servers/"));
                    let body=json!({"_id":ID,"name":"New name","description":"New description","owner":ID}).to_string();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
                }
            }
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            save_overview(&policy(api), ID, &overview()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.name, "New name");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn permission_failures_are_visible_and_do_not_expose_response_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/api/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            loop {
                header.push(stream.read_u8().await.unwrap());
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let body = "private backend diagnostics";
            stream.write_all(format!("HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        });
        let failure = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            fetch_invites(&policy(api), ID),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(failure.to_string().contains("permission"));
        assert!(!failure.to_string().contains("private"));
        server.await.unwrap();
    }
    #[test]
    fn list_projection_accepts_wrapped_and_raw_resources() {
        let emojis: Vec<ServerEmoji> =
            list(json!({"emojis":[{"id":ID,"name":"wave"}]}), "emojis").unwrap();
        assert_eq!(emojis[0].name, "wave");
        let invites: Vec<ServerInvite> = list(
            json!([{ "code":"abc", "server":ID,"channel":ID,"creator":ID,"uses":2 }]),
            "invites",
        )
        .unwrap();
        assert_eq!(invites[0].uses, Some(2));
        assert!(list::<ServerEmoji>(json!({"emojis":"bad"}), "emojis").is_err());
    }
}
