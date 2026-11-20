use super::*;
use crate::{
    http::{ContextError, SessionAccess},
    server::ServerSelection,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
const MESSAGE: &str = "01M0P54M56DA883E6ZYDK00BKD";

struct Session(ServerSelection);
impl SessionAccess for Session {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(Some(self.0.clone()))
    }
    fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
        Ok(Some("local-test-token".into()))
    }
}
fn policy(api: &str) -> HttpPolicy {
    let selection = serde_json::from_value(json!({
        "deploymentUrl": api, "displayName": "test", "official": false, "apiVersion": "test",
        "endpoints": {"api": api, "websocket": "wss://example.com", "livekitEnabled": false}
    }))
    .unwrap();
    HttpPolicy::new(Arc::new(Session(selection))).unwrap()
}

fn manager(bits: u64) -> ConnectionManager {
    let manager = ConnectionManager::default();
    let generation = manager.begin(None).unwrap();
    manager.publish_ready(generation, json!({"currentUser": {"_id": "me"}, "ready": {
        "channels": [{"_id": CHANNEL, "server": "server"}],
        "servers": [{"_id": "server", "owner": "other", "roles": {}, "default_permissions": (1 << 20) | bits}],
        "members": [{"_id": {"server": "server", "user": "me"}, "roles": []}]
    }}), || Ok(())).unwrap();
    manager
}

#[tokio::test]
async fn mutation_http_contracts() {
    let target = format!("/api/channels/{CHANNEL}/messages/{MESSAGE}");
    for (method, path, expected_body) in [
        (
            "PATCH",
            target.clone(),
            Some(json!({"content": "  edited"})),
        ),
        ("DELETE", target.clone(), None),
        ("POST", format!("{target}/pin"), None),
        ("DELETE", format!("{target}/pin"), None),
        (
            "POST",
            "/api/safety/report".into(),
            Some(json!({
                "content": {"type": "Message", "id": MESSAGE, "report_reason": "NoneSpecified"},
                "additional_context": " context "
            })),
        ),
    ] {
        timeout(Duration::from_secs(3), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let policy = policy(&format!("http://{}/api", listener.local_addr().unwrap()));
            let pin = path.ends_with("/pin");
            let report = path.ends_with("/report");
            let mock = async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(socket.read_u8().await.unwrap());
                }
                let header = String::from_utf8(header).unwrap();
                assert_eq!(header.lines().next().unwrap(), format!("{method} {path} HTTP/1.1"));
                assert!(header.contains("x-session-token: local-test-token\r\n"));
                let length = header.lines().find_map(|line| line.strip_prefix("content-length: "))
                    .map(|value| value.parse().unwrap()).unwrap_or(0);
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await.unwrap();
                let actual = (!body.is_empty()).then(|| serde_json::from_slice::<Value>(&body).unwrap());
                assert_eq!(actual, expected_body);
                if method == "PATCH" {
                    let body = json!({"_id": MESSAGE, "channel": CHANNEL, "author": MESSAGE, "content": "  edited"}).to_string();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                } else {
                    socket.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").await.unwrap();
                }
            };
            let client = async {
                if report {
                    report_message(&policy, MESSAGE, " context ").await.unwrap();
                } else if pin {
                    set_message_pinned(&policy, &manager(MANAGE_MESSAGES), CHANNEL, MESSAGE, method == "POST").await.unwrap();
                } else if method == "PATCH" {
                    let result = edit_message(&policy, &manager(SEND_MESSAGE), CHANNEL, MESSAGE, "  edited \n").await.unwrap();
                    assert_eq!(result.content.as_deref(), Some("  edited"));
                } else {
                    delete_message(&policy, &manager(0), CHANNEL, MESSAGE).await.unwrap();
                }
            };
            tokio::join!(mock, client);
        }).await.unwrap();
    }
}

#[tokio::test]
async fn invalid_input_and_permission_failures_precede_http() {
    let policy = policy("http://127.0.0.1:1");
    for error in [
        edit_message(
            &policy,
            &manager(MANAGE_MESSAGES),
            CHANNEL,
            MESSAGE,
            "edited",
        )
        .await
        .unwrap_err(),
        set_message_pinned(&policy, &manager(SEND_MESSAGE), CHANNEL, MESSAGE, false)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "authentication_failed"
        );
    }
    for error in [
        edit_message(&policy, &manager(SEND_MESSAGE), CHANNEL, MESSAGE, " \n")
            .await
            .unwrap_err(),
        delete_message(&policy, &manager(MANAGE_MESSAGES), "bad/id", MESSAGE)
            .await
            .unwrap_err(),
        report_message(&policy, "bad/id", "").await.unwrap_err(),
        report_message(&policy, MESSAGE, &"é".repeat(1001))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "invalid_input"
        );
    }
}
