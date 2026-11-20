use super::*;
use std::collections::HashSet;

fn message_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .or_else(|| value.get("messages").and_then(Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(|message| {
            message
                .get("_id")
                .or_else(|| message.get("id"))
                .and_then(Value::as_str)
                .filter(|id| valid_id(id))
                .map(str::to_owned)
        })
        .collect()
}
fn timestamp(id: &str) -> Option<u64> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    id.as_bytes()
        .get(..10)?
        .iter()
        .try_fold(0u64, |timestamp, byte| {
            timestamp.checked_mul(32)?.checked_add(
                ALPHABET
                    .iter()
                    .position(|candidate| *candidate == byte.to_ascii_uppercase())?
                    as u64,
            )
        })
}
async fn reset_request(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    request: RequestBuilder,
) -> Result<Option<Value>, CommandError> {
    for attempt in 0..=7 {
        let response = policy
            .execute_authenticated(
                context,
                request.try_clone().ok_or_else(|| {
                    error(
                        ErrorCode::Unavailable,
                        "The reset request could not be retried.",
                    )
                })?,
                RequestClass::Standard,
                MAX_JSON_RESPONSE_BYTES,
            )
            .await
            .map_err(policy_error)?;
        let (status, header_delay, bytes) = response.consume(
            |status, headers, bytes| {
                let delay = headers
                    .get("retry-after")
                    .or_else(|| headers.get("x-ratelimit-reset-after"))
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value >= 0.)
                    .map(|seconds| (seconds * 1000.) as u64);
                Ok((status, delay, bytes.to_vec()))
            },
            || error(ErrorCode::Unavailable, "The channel reset was cancelled."),
        )?;
        if status == StatusCode::TOO_MANY_REQUESTS && attempt < 7 {
            let body_delay = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|body| body.get("retry_after").and_then(Value::as_f64))
                .filter(|value| value.is_finite() && *value >= 0.)
                .map(|milliseconds| milliseconds as u64);
            let delay = header_delay
                .or(body_delay)
                .unwrap_or(500u64.saturating_mul(1 << attempt))
                .clamp(250, 15_000);
            // The containing entity owns an Abortable around this entire
            // operation, including backoff and single-message pacing.
            tokio::time::sleep(Duration::from_millis(delay + 100)).await;
            continue;
        }
        if !status.is_success() {
            return Err(status_error(status));
        }
        return if bytes.is_empty() {
            Ok(None)
        } else {
            serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                error(
                    ErrorCode::InvalidResponse,
                    "The server returned invalid reset data.",
                )
            })
        };
    }
    Err(error(
        ErrorCode::Unavailable,
        "The channel reset could not be completed.",
    ))
}
/// Remove all channel history using Stoat's recent bulk route and paced
/// single-message deletion for history older than six days. No channel or
/// settings are recreated. Cancellation may leave a partially reset channel.
pub async fn reset(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel_id: &str,
) -> Result<usize, CommandError> {
    require(manager, channel_id, MANAGE_MESSAGES)?;
    let context = context(policy)?;
    reset_with(policy, &context, channel_id, || {
        require(manager, channel_id, MANAGE_MESSAGES)
    })
    .await
}
async fn reset_with(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    channel_id: &str,
    authorize: impl Fn() -> Result<(), CommandError>,
) -> Result<usize, CommandError> {
    let mut deleted = 0usize;
    let mut seen = HashSet::new();
    loop {
        authorize()?;
        let page_request = request(
            policy,
            context,
            Method::GET,
            &["channels", channel_id, "messages"],
            None,
        )?
        .query(&[("limit", "100"), ("sort", "Latest")]);
        let page = reset_request(policy, context, page_request)
            .await?
            .unwrap_or_else(|| json!([]));
        let ids = message_ids(&page);
        if ids.is_empty() {
            return Ok(deleted);
        }
        // Bound incompatible deployments that acknowledge deletes but return
        // the same messages indefinitely. Already confirmed IDs are never
        // submitted twice or counted twice.
        if ids.iter().any(|id| seen.contains(id)) {
            return Err(error(
                ErrorCode::Unavailable,
                "The server repeated messages already deleted. The channel reset stopped.",
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let cutoff = now.saturating_sub(6 * 24 * 60 * 60 * 1000);
        let (recent, older): (Vec<_>, Vec<_>) = ids
            .into_iter()
            .partition(|id| timestamp(id).is_some_and(|value| value >= cutoff));
        if recent.len() > 1 {
            authorize()?;
            reset_request(
                policy,
                context,
                request(
                    policy,
                    context,
                    Method::DELETE,
                    &["channels", channel_id, "messages", "bulk"],
                    Some(json!({"ids":recent})),
                )?,
            )
            .await?;
            deleted += recent.len();
            seen.extend(recent);
        } else if let Some(id) = recent.first() {
            authorize()?;
            reset_request(
                policy,
                context,
                request(
                    policy,
                    context,
                    Method::DELETE,
                    &["channels", channel_id, "messages", id],
                    None,
                )?,
            )
            .await?;
            deleted += 1;
            seen.insert(id.clone());
        }
        for id in older {
            authorize()?;
            reset_request(
                policy,
                context,
                request(
                    policy,
                    context,
                    Method::DELETE,
                    &["channels", channel_id, "messages", &id],
                    None,
                )?,
            )
            .await?;
            deleted += 1;
            seen.insert(id);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_page_shapes_and_unsafe_ids() {
        assert_eq!(
            message_ids(&json!([{"_id":"A1"},{"id":"B2"},{"_id":"../bad"},{}])),
            ["A1", "B2"]
        );
        assert_eq!(message_ids(&json!({"messages":[{"_id":"A1"}]})), ["A1"]);
        assert_eq!(
            timestamp("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            Some(1_469_922_850_259)
        );
    }
    #[tokio::test]
    async fn reset_uses_bulk_for_recent_and_single_for_older_then_rechecks_history() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut encoded = [b'0'; 10];
        let mut millis = now;
        for byte in encoded.iter_mut().rev() {
            *byte = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ"[(millis % 32) as usize];
            millis /= 32;
        }
        let prefix = String::from_utf8(encoded.to_vec()).unwrap();
        let a = format!("{prefix}0000000000000001");
        let b = format!("{prefix}0000000000000002");
        let old = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let channel = "01M0P54M56DA883E6ZYDK00BKC";
        let expected = vec![a.clone(), b.clone()];
        let server = tokio::spawn(async move {
            for index in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    bytes.push(stream.read_u8().await.unwrap());
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let header = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
                assert!(header.contains("x-session-token: local-token"));
                let body = match index {
                    0 => {
                        assert!(header.starts_with("get /channels/"));
                        assert!(header.contains("limit=100&sort=latest"));
                        json!([{"_id":a},{"_id":b},{"_id":old}]).to_string()
                    }
                    1 => {
                        assert!(header.starts_with("delete /channels/"));
                        assert!(header.contains("/messages/bulk"));
                        let size = header
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        let mut body = vec![0; size];
                        stream.read_exact(&mut body).await.unwrap();
                        assert_eq!(
                            serde_json::from_slice::<Value>(&body).unwrap(),
                            json!({"ids":expected})
                        );
                        "".into()
                    }
                    2 => {
                        assert!(header.contains(&format!("/messages/{}", old.to_ascii_lowercase())));
                        "".into()
                    }
                    _ => "[]".into(),
                };
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
            }
        });
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            reset(
                &super::super::tests::policy(&api),
                &super::super::tests::manager(),
                channel,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result, 3);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn logout_cancels_inflight_reset_before_any_deletion() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = std::sync::Arc::new(super::super::tests::policy(&format!(
            "http://{}",
            listener.local_addr().unwrap()
        )));
        let pending_policy = policy.clone();
        let pending = tokio::spawn(async move {
            reset(
                &pending_policy,
                &super::super::tests::manager(),
                "01M0P54M56DA883E6ZYDK00BKC",
            )
            .await
        });
        let (_stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        policy.logout();
        assert!(tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
    #[tokio::test]
    async fn reset_retries_rate_limits_with_the_same_read_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    bytes.push(stream.read_u8().await.unwrap());
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(String::from_utf8(bytes)
                    .unwrap()
                    .starts_with("GET /channels/"));
                if index == 0 {
                    stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(3),
                reset(
                    &super::super::tests::policy(&api),
                    &super::super::tests::manager(),
                    "01M0P54M56DA883E6ZYDK00BKC"
                )
            )
            .await
            .unwrap()
            .unwrap(),
            0
        );
        server.await.unwrap();
    }
}
