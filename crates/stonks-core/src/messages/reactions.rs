//! Reaction mutations share the Tauri request contract and current authorization.
use super::{models::ReactionRequest, request::*};
use crate::{
    connection::ConnectionManager,
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass},
};

pub const REACT: u64 = 1 << 29;
pub const USE_EXTERNAL_EMOJIS: u64 = 1 << 41;

pub async fn set_membership(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    request: &ReactionRequest,
    present: bool,
) -> Result<(), CommandError> {
    validate_reaction_request(request)?;
    manager
        .require_channel_permission(&request.channel_id, REACT)
        .map_err(|_| {
            CommandError::new(
                ErrorCode::AuthenticationFailed,
                "You do not have permission to react in this channel.",
            )
        })?;
    send(policy, request, present).await
}
async fn send(
    policy: &HttpPolicy,
    request: &ReactionRequest,
    present: bool,
) -> Result<(), CommandError> {
    let context = authenticated_context(policy)?;
    let target = reaction_url(&context.server, request)?;
    let builder = if present {
        policy.client().put(target)
    } else {
        policy
            .client()
            .delete(target)
            .json(&serde_json::json!({"remove_all":false}))
    }
    .header("x-session-token", &context.token);
    let response = execute_authenticated(
        policy,
        &context,
        builder,
        RequestClass::Standard,
        "The reaction could not be updated.",
    )
    .await?;
    reaction_response(response, "The reaction could not be updated.")
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
    async fn put_and_delete_match_source_contract_and_keep_unicode_in_one_path_segment() {
        for present in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                loop {
                    bytes.push(stream.read_u8().await.unwrap());
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let header = String::from_utf8(bytes).unwrap();
                assert!(header.starts_with(if present { "PUT /" } else { "DELETE /" }));
                assert!(header.contains("/reactions/%F0%9F%92%80"));
                assert!(header.contains("x-session-token: local-test-token"));
                if !present {
                    let size: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; size];
                    stream.read_exact(&mut body).await.unwrap();
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                        serde_json::json!({"remove_all":false})
                    );
                }
                stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            });
            let selection=serde_json::from_value(serde_json::json!({"deploymentUrl":api,"displayName":"test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap();
            let policy = HttpPolicy::new(Arc::new(Session(selection))).unwrap();
            let request = ReactionRequest {
                channel_id: "01M0P54M56DA883E6ZYDK00BKC".into(),
                message_id: "01M0P54M56DA883E6ZYDK00BKD".into(),
                emoji: "💀".into(),
            };
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                send(&policy, &request, present),
            )
            .await
            .unwrap()
            .unwrap();
            server.await.unwrap();
        }
    }
}
