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
        Ok(Some("local-member-test".into()))
    }
}
fn policy(api: String) -> HttpPolicy {
    HttpPolicy::new(Arc::new(Session(serde_json::from_value(json!({"deploymentUrl":api,"displayName":"Fixture","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))).unwrap()
}
#[test]
fn patches_match_source_clear_and_roles_contract_without_clearing_omitted_fields() {
    let patch = MemberPatch {
        nickname: Some("  ".into()),
        pronouns: Some(None),
        avatar: Some(Some("avatar123".into())),
        roles: Some(vec!["role123".into()]),
    };
    assert_eq!(
        patch.body().unwrap(),
        json!({"remove":["Nickname","Pronouns"],"avatar":"avatar123","roles":["role123"]})
    );
    let patch = MemberPatch {
        nickname: Some(" Nick ".into()),
        ..Default::default()
    };
    assert_eq!(patch.body().unwrap(), json!({"nickname":"Nick"}));
    assert!(MemberPatch::default().body().is_err());
    assert!(MemberPatch {
        pronouns: Some(Some("x".repeat(25))),
        ..Default::default()
    }
    .body()
    .is_err());
    assert!(MemberPatch {
        roles: Some(vec!["role/evil".into()]),
        ..Default::default()
    }
    .body()
    .is_err());
    assert_eq!(
        MemberPatch {
            avatar: Some(Some("autumn-file_id".into())),
            ..Default::default()
        }
        .body()
        .unwrap()["avatar"],
        "autumn-file_id"
    );
    assert!(MemberPatch {
        avatar: Some(Some("file\nheader".into())),
        ..Default::default()
    }
    .body()
    .is_err());
}
#[test]
fn relationship_policy_matches_source_aliases_and_does_not_guess_unknown_values() {
    assert_eq!(
        relationship_kind(Some(" Friend ")),
        relationship_kind(Some("friends"))
    );
    assert_ne!(relationship_kind(None), relationship_kind(Some("Blocked")));
    assert_ne!(
        relationship_kind(Some("pending")),
        relationship_kind(Some("BestFriend"))
    );
    assert_eq!(
        relationship_actions(Some("Friend"), false),
        vec![Action::Message, Action::Remove, Action::Block]
    );
    assert_eq!(
        relationship_actions(Some(" Pending_Incoming "), false),
        vec![Action::Accept, Action::Decline, Action::Block]
    );
    assert_eq!(
        relationship_actions(Some("request-sent"), false),
        vec![Action::Cancel, Action::Block]
    );
    assert_eq!(
        relationship_actions(Some("Blocked"), false),
        vec![Action::Unblock]
    );
    assert_eq!(
        relationship_actions(None, false),
        vec![Action::Add, Action::Block]
    );
    assert!(relationship_actions(Some("BestFriend"), false).is_empty());
    assert!(relationship_actions(Some("pending"), false).is_empty());
    assert_eq!(
        relationship_actions(Some("Blocked"), true),
        vec![Action::Message]
    );
}
#[test]
fn context_actions_follow_self_and_each_permission_without_exposing_self_moderation() {
    assert_eq!(context_actions(true, 0), vec![Action::Profile]);
    assert_eq!(
        context_actions(false, 0),
        vec![
            Action::Profile,
            Action::Message,
            Action::Block,
            Action::Report
        ]
    );
    assert_eq!(
        context_actions(true, CHANGE_AVATAR),
        vec![Action::Profile, Action::Identity]
    );
    assert_eq!(
        context_actions(false, MANAGE_NICKNAMES | ASSIGN_ROLES | KICK_MEMBERS),
        vec![
            Action::Profile,
            Action::Message,
            Action::Identity,
            Action::Roles,
            Action::Kick,
            Action::Block,
            Action::Report
        ]
    );
    assert_eq!(
        context_actions(true, u64::MAX),
        vec![Action::Profile, Action::Identity, Action::Roles]
    );
    assert!(!context_actions(false, CHANGE_NICKNAME | CHANGE_AVATAR).contains(&Action::Identity));
}
#[test]
fn report_and_mfa_requests_validate_and_preserve_source_payloads() {
    assert_eq!(
        report_body("user123", " context ").unwrap(),
        json!({"content":{"type":"User","id":"user123","report_reason":"NoneSpecified"},"additional_context":"context"})
    );
    assert!(report_body("user/other", "").is_err());
    assert!(report_body("user123", &"x".repeat(1001)).is_err());
    assert_eq!(
        mfa_body("totp", " 123 456 ").unwrap(),
        json!({"totp_code":"123456"})
    );
    assert_eq!(
        mfa_body("password", " pass ").unwrap(),
        json!({"password":"pass"})
    );
    assert!(mfa_body("unsupported", "code").is_err());
    assert!(mfa_body("totp", " ").is_err());
}
#[tokio::test]
async fn report_posts_exact_contract_and_accepts_empty_success() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = policy(format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0; 16384];
        let mut size = 0;
        loop {
            size += socket.read(&mut buffer[size..]).await.unwrap();
            let bytes = &buffer[..size];
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let header = std::str::from_utf8(&bytes[..end]).unwrap();
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap();
                if size >= end + 4 + length {
                    let headers = header.to_owned();
                    let body: Value =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    socket
                        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                    return (headers, body);
                }
            }
        }
    });
    report(&policy, "user123", " reason ").await.unwrap();
    let (headers, body) = server.await.unwrap();
    assert!(headers.starts_with("POST /safety/report HTTP/1.1"));
    assert!(headers
        .to_lowercase()
        .contains("x-session-token: local-member-test"));
    assert_eq!(body, report_body("user123", "reason").unwrap());
}
#[tokio::test]
async fn moderation_requires_live_permissions_before_network() {
    let policy = policy("http://127.0.0.1:1".into());
    let authorization = ConnectionManager::default();
    let error = moderate(&policy, &authorization, "server123", "user123", false)
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "authentication_failed"
    );
}
#[tokio::test]
async fn profile_add_uses_known_user_put_without_requiring_a_discriminator() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = policy(format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 8192];
        let mut size = 0;
        loop {
            let n = socket.read(&mut bytes[size..]).await.unwrap();
            assert!(n > 0);
            size += n;
            if bytes[..size].windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(bytes[..size].to_vec()).unwrap();
        let response = r#"{"_id":"user123","username":"known","relationship":"Outgoing"}"#;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
        request
    });
    let user: ClientUser =
        serde_json::from_value(json!({"_id":"user123","username":"known"})).unwrap();
    let changed = relationship(&policy, &user, Action::Add).await.unwrap();
    assert_eq!(changed.relationship.as_deref(), Some("Outgoing"));
    assert!(server
        .await
        .unwrap()
        .starts_with("PUT /users/user123/friend HTTP/1.1"));
}

#[tokio::test]
async fn friend_request_rejects_invalid_handles_before_authentication() {
    struct NoSession;
    impl SessionAccess for NoSession {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            panic!("invalid input must not load the session")
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            panic!("invalid input must not load the token")
        }
    }
    let policy = HttpPolicy::new(Arc::new(NoSession)).unwrap();
    let oversized = format!("{}#1", "é".repeat(64));
    for handle in [
        "",
        "friend",
        "#0001",
        "friend#",
        "fri\nend#0001",
        &oversized,
    ] {
        let error = send_friend_request(&policy, handle).await.unwrap_err();
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "invalid_input"
        );
    }
}

async fn post_fixture(
    response_status: &str,
    response_body: &str,
    run: impl AsyncFnOnce(&HttpPolicy) -> (),
) -> (String, Value) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = policy(format!("http://{}/api", listener.local_addr().unwrap()));
    let mock = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = receive_json(&mut socket).await;
        let length = response_body.len();
        socket
            .write_all(
                format!("HTTP/1.1 {response_status}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{response_body}").as_bytes(),
            )
            .await
            .unwrap();
        request
    };
    let (request, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(mock, run(&policy))
    })
    .await
    .expect("local request did not complete");
    request
}

#[tokio::test]
async fn friend_request_posts_trimmed_handle_and_decodes_the_user() {
    let (headers, body) = post_fixture(
        "200 OK",
        r#"{"_id":"user123","username":"friend","discriminator":"0001","relationship":"Outgoing"}"#,
        async |policy| {
            let user = send_friend_request(policy, "  friend#0001  ")
                .await
                .unwrap();
            assert_eq!(user.id, "user123");
            assert_eq!(user.relationship.as_deref(), Some("Outgoing"));
        },
    )
    .await;
    assert_eq!(
        headers.lines().next(),
        Some("POST /api/users/friend HTTP/1.1")
    );
    assert!(headers
        .to_ascii_lowercase()
        .lines()
        .any(|line| line == "x-session-token: local-member-test"));
    assert_eq!(body, json!({"username":"friend#0001"}));
}

#[tokio::test]
async fn friend_request_maps_status_and_requires_a_user_response() {
    for (status, body, code) in [
        ("404 Not Found", "", "not_found"),
        ("429 Too Many Requests", "", "unavailable"),
        ("400 Bad Request", "", "invalid_input"),
        ("401 Unauthorized", "", "authentication_failed"),
        ("204 No Content", "", "invalid_response"),
        ("200 OK", "{}", "invalid_response"),
    ] {
        post_fixture(status, body, async |policy| {
            let error = send_friend_request(policy, "friend#0001")
                .await
                .unwrap_err();
            assert_eq!(serde_json::to_value(error).unwrap()["code"], code);
        })
        .await;
    }
}

#[tokio::test]
async fn home_group_creation_posts_users_and_decodes_the_channel() {
    let (headers, body) = post_fixture(
        "200 OK",
        r#"{"_id":"group123","channel_type":"Group","name":"Study Group","recipients":["self123","friend123"]}"#,
        async |policy| {
            let channel = crate::connection::create_group(
                policy,
                crate::connection::CreateGroupRequest {
                    name: "  Study Group  ".into(),
                    recipients: vec!["friend123".into()],
                },
            )
            .await
            .unwrap();
            assert_eq!(channel.id, "group123");
            assert_eq!(channel.channel_type, "Group");
            assert_eq!(channel.recipients, ["self123", "friend123"]);
        },
    )
    .await;
    assert_eq!(
        headers.lines().next(),
        Some("POST /api/channels/create HTTP/1.1")
    );
    assert!(headers
        .to_ascii_lowercase()
        .lines()
        .any(|line| line == "x-session-token: local-member-test"));
    assert_eq!(
        body,
        json!({"type":"Group","name":"Study Group","users":["friend123"]})
    );
}

fn owner_manager() -> ConnectionManager {
    let manager = ConnectionManager::default();
    let generation = manager.begin(None).unwrap();
    manager.publish_ready(generation,json!({"currentUser":{"_id":"owner"},"ready":{"servers":[{"_id":"server123","name":"Server","owner":"owner"}],"channels":[],"members":[]}}),||Ok(())).unwrap();
    manager
}
async fn receive_json(socket: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut bytes = vec![0; 16384];
    let mut size = 0;
    loop {
        let n = socket.read(&mut bytes[size..]).await.unwrap();
        assert!(n > 0);
        size += n;
        if let Some(end) = bytes[..size]
            .windows(4)
            .position(|value| value == b"\r\n\r\n")
        {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if size >= end + 4 + length {
                return (
                    headers.to_owned(),
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                );
            }
        }
    }
}
async fn reply_json(socket: &mut tokio::net::TcpStream, value: &str) {
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{value}",value.len()).as_bytes()).await.unwrap();
}
#[tokio::test]
async fn verified_ticket_and_owned_transfer_are_separate_authenticated_contracts() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = policy(format!("http://{}", listener.local_addr().unwrap()));
    let authorization = owner_manager();
    let server = tokio::spawn(async move {
        let (mut verification, _) = listener.accept().await.unwrap();
        let first = receive_json(&mut verification).await;
        reply_json(&mut verification, r#"{"token":"fixture-ticket"}"#).await;
        let (mut ownership, _) = listener.accept().await.unwrap();
        let second = receive_json(&mut ownership).await;
        reply_json(
            &mut ownership,
            r#"{"_id":"server123","name":"Server","owner":"user123"}"#,
        )
        .await;
        (first, second)
    });
    let ticket = mfa_ticket(&policy, "totp", "123 456").await.unwrap();
    assert_eq!(ticket, "fixture-ticket");
    let updated = transfer_with_ticket(&policy, &authorization, "server123", "user123", &ticket)
        .await
        .unwrap();
    assert_eq!(updated.owner, "user123");
    let ((verification_headers, verification), (ownership_headers, ownership)) =
        server.await.unwrap();
    assert!(verification_headers.starts_with("PUT /auth/mfa/ticket HTTP/1.1"));
    assert_eq!(verification, json!({"totp_code":"123456"}));
    assert!(!verification_headers.to_lowercase().contains("x-mfa-ticket"));
    assert!(ownership_headers.starts_with("PATCH /servers/server123 HTTP/1.1"));
    assert_eq!(ownership, json!({"owner":"user123"}));
    assert!(ownership_headers
        .to_lowercase()
        .contains("x-mfa-ticket: fixture-ticket"));
    for headers in [verification_headers, ownership_headers] {
        assert!(headers
            .to_lowercase()
            .contains("x-session-token: local-member-test"));
    }
}
#[tokio::test]
async fn ownership_lost_after_verification_prevents_the_transfer_before_io() {
    let policy = policy("http://127.0.0.1:1".into());
    let authorization = owner_manager();
    authorization.stop(None).unwrap();
    let error = transfer_with_ticket(
        &policy,
        &authorization,
        "server123",
        "user123",
        "verified-ticket",
    )
    .await
    .unwrap_err();
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "authentication_failed"
    );
}
