//! Read-only channel search/pins service, matching the Tauri request contract.
use super::{
    models::{MessagePage, MessageSearchRequest},
    normalize::{member_user_id, normalize_message, response_parts, valid_id},
    request::{authenticated_context, consume_json, execute_authenticated, invalid_input},
};
use crate::{
    error::CommandError,
    http::{HttpPolicy, RequestClass},
    server::ServerSelection,
};
use serde_json::Value;
use std::collections::HashMap;

pub fn request_body(request: &MessageSearchRequest) -> Result<Value, CommandError> {
    if !valid_id(&request.channel_id) || request.before.as_deref().is_some_and(|id| !valid_id(id)) {
        return Err(invalid_input("The search request contained an invalid ID."));
    }
    let query = request
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty());
    if request.pinned && query.is_some() {
        return Err(invalid_input(
            "Pinned messages and text cannot be searched together.",
        ));
    }
    // Preserve the existing native contract: its 64-character guard counts
    // UTF-8 bytes. The input applies the same bound before starting requests.
    if query.is_some_and(|query| query.len() > 64) {
        return Err(invalid_input("Search text must be 64 characters or fewer."));
    }
    if !matches!(request.sort.as_str(), "Latest" | "Oldest" | "Relevance") {
        return Err(invalid_input("The search sort is invalid."));
    }
    let mut body = serde_json::json!({"limit":50,"include_users":true,"sort":request.sort});
    if request.pinned {
        body["pinned"] = true.into();
    }
    if let Some(query) = query {
        body["query"] = query.into();
    }
    if let Some(before) = &request.before {
        body["before"] = before.clone().into();
    }
    Ok(body)
}

pub async fn search_messages(
    policy: &HttpPolicy,
    request: &MessageSearchRequest,
) -> Result<MessagePage, CommandError> {
    let body = request_body(request)?;
    let context = authenticated_context(policy)?;
    let mut url = url::Url::parse(&context.server.endpoints.api)
        .map_err(|_| invalid_input("The search address was invalid."))?;
    url.path_segments_mut()
        .map_err(|_| invalid_input("The search address was invalid."))?
        .pop_if_empty()
        .extend(["channels", &request.channel_id, "search"]);
    let request = policy
        .client()
        .post(url)
        .header("x-session-token", &context.token)
        .json(&body);
    let response = execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Standard,
        "Could not search messages.",
    )
    .await?;
    consume_json(
        response,
        "The message search failed.",
        "Could not search messages.",
        |value| normalize_page(value, &context.server),
    )
}

fn normalize_page(value: Value, server: &ServerSelection) -> Result<MessagePage, CommandError> {
    let (messages, users, members) = response_parts(value);
    let users: HashMap<_, _> = users
        .into_iter()
        .filter_map(|user| {
            user.get("_id")
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), user.clone()))
        })
        .collect();
    let members: HashMap<_, _> = members
        .into_iter()
        .filter_map(|member| member_user_id(&member).map(|id| (id.to_owned(), member.clone())))
        .collect();
    let messages = messages
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

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "01M0P54M56DA883E6ZYDK00BKC";
    fn request() -> MessageSearchRequest {
        MessageSearchRequest {
            channel_id: CHANNEL.into(),
            query: Some(" hello ".into()),
            pinned: false,
            before: None,
            sort: "Relevance".into(),
        }
    }
    #[test]
    fn payload_matches_search_and_pins_contract() {
        assert_eq!(
            request_body(&request()).unwrap(),
            serde_json::json!({"limit":50,"include_users":true,"sort":"Relevance","query":"hello"})
        );
        let mut request = request();
        request.query = None;
        request.pinned = true;
        request.sort = "Latest".into();
        request.before = Some(CHANNEL.into());
        assert_eq!(
            request_body(&request).unwrap(),
            serde_json::json!({"limit":50,"include_users":true,"sort":"Latest","pinned":true,"before":CHANNEL})
        );
    }
    #[test]
    fn invalid_query_cursor_sort_and_combined_pins_fail_before_io() {
        let mut request = request();
        request.query = Some("x".repeat(65));
        assert!(request_body(&request).is_err());
        request.query = Some("x".repeat(64));
        assert!(request_body(&request).is_ok());
        request.pinned = true;
        assert!(request_body(&request).is_err());
        request.pinned = false;
        request.sort = "unsupported".into();
        assert!(request_body(&request).is_err());
        request.sort = "Latest".into();
        request.before = Some("bad".into());
        assert!(request_body(&request).is_err());
    }
    struct TestSession(ServerSelection);
    impl crate::http::SessionAccess for TestSession {
        fn server_selection(&self) -> Result<Option<ServerSelection>, crate::http::ContextError> {
            Ok(Some(self.0.clone()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, crate::http::ContextError> {
            Ok(Some("local-test-token".into()))
        }
    }
    fn policy(api: &str) -> HttpPolicy {
        HttpPolicy::new(std::sync::Arc::new(TestSession(serde_json::from_value(serde_json::json!({"deploymentUrl":api,"displayName":"Test","official":false,"apiVersion":"test","endpoints":{"api":api,"websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))).unwrap()
    }
    #[tokio::test]
    async fn search_posts_authenticated_query_and_normalizes_user_and_member() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
                "post /api/channels/{}/search",
                CHANNEL.to_ascii_lowercase()
            )));
            assert!(headers.contains("x-session-token: local-test-token"));
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
                serde_json::json!({"limit":50,"include_users":true,"sort":"Relevance","query":"hello"})
            );
            let body=serde_json::json!({"messages":[{"_id":CHANNEL,"channel":CHANNEL,"author":"author","content":"hello"}],"users":[{"_id":"author","username":"Author"}],"members":[{"_id":{"server":"s","user":"author"},"nickname":"Nick"}]}).to_string();
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
        let page = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            search_messages(&policy(&api), &request()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(page.messages[0].author_name, "Nick");
        assert!(!page.has_more);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn logout_cancels_inflight_search() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = std::sync::Arc::new(policy(&format!(
            "http://{}",
            listener.local_addr().unwrap()
        )));
        let pending_policy = policy.clone();
        let pending =
            tokio::spawn(async move { search_messages(&pending_policy, &request()).await });
        let (_stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
        policy.logout();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }
}
