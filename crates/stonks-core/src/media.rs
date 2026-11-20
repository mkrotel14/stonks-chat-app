//! Credential-free, bounded media downloads through the shared HTTP policy.
use crate::{
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass},
};
use url::Url;

pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

fn invalid() -> CommandError {
    CommandError::new(ErrorCode::InvalidResponse, "The image could not be loaded.")
}

fn image_url(value: &str) -> Result<Url, CommandError> {
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

/// Build file URLs using encoded path segments, preserving deployment prefixes.
pub fn file_url(base: Option<&str>, tag: &str, id: &str) -> Option<String> {
    if tag.is_empty() || id.is_empty() {
        return None;
    }
    let mut url = image_url(base?).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend([tag, id]);
    Some(url.into())
}

enum MediaResponse {
    Bytes(Vec<u8>),
    Redirect(String),
}

fn redirect_url(current: &Url, location: &str) -> Result<Url, CommandError> {
    let next = current.join(location).map_err(|_| invalid())?;
    let next = image_url(next.as_str())?;
    if current.scheme() == "https" && next.scheme() != "https" {
        return Err(invalid());
    }
    Ok(next)
}

pub async fn fetch_image(policy: &HttpPolicy, value: &str) -> Result<Vec<u8>, CommandError> {
    let context = policy.require_deployment().map_err(|_| invalid())?;
    let mut url = image_url(value)?;
    // Follow bounded media redirects explicitly, without API credentials.
    // The same policy epoch covers every hop, so logout cancels the chain.
    for hop in 0..=3 {
        let request = policy
            .client()
            .get(url.clone())
            .header("Accept", "image/png,image/jpeg,image/webp,image/gif");
        let response = policy
            .execute_deployment(&context, request, RequestClass::Standard, MAX_IMAGE_BYTES)
            .await
            .map_err(|_| invalid())?
            .consume(
                |status, headers, bytes| {
                    if status.is_redirection() {
                        let location = headers
                            .get("location")
                            .and_then(|value| value.to_str().ok())
                            .ok_or_else(invalid)?;
                        return Ok(MediaResponse::Redirect(location.to_owned()));
                    }
                    if !status.is_success() || bytes.is_empty() {
                        return Err(invalid());
                    }
                    Ok(MediaResponse::Bytes(bytes.to_vec()))
                },
                invalid,
            )?;
        match response {
            MediaResponse::Bytes(bytes) => return Ok(bytes),
            MediaResponse::Redirect(location) if hop < 3 => url = redirect_url(&url, &location)?,
            _ => return Err(invalid()),
        }
    }
    Err(invalid())
}

/// Preserve animation by using the original Autumn avatar.
pub fn avatar_url(base: Option<&str>, tag: &str, id: &str) -> Option<String> {
    let mut url = image_url(&file_url(base, tag, id)?).ok()?;
    url.path_segments_mut().ok()?.push("original");
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::{ContextError, SessionAccess},
        server::{
            models::{ServerCapabilities, ServerEndpoints},
            ServerSelection,
        },
    };
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct MediaSession;
    impl SessionAccess for MediaSession {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(ServerSelection {
                deployment_url: "https://example.com".into(),
                display_name: "Test".into(),
                official: false,
                api_version: "test".into(),
                capabilities: ServerCapabilities::default(),
                endpoints: ServerEndpoints {
                    api: "https://example.com/api".into(),
                    websocket: "wss://example.com".into(),
                    autumn: None,
                    january: None,
                    gifbox: None,
                    livekit_enabled: false,
                },
            }))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            panic!("media must never access the session token")
        }
    }

    #[tokio::test]
    async fn downloads_without_credentials_and_rejects_http_failures() {
        for (response, expected) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nPNG".to_owned(),
                true,
            ),
            (
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_owned(),
                false,
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: file:///invalid\r\nContent-Length: 0\r\n\r\n"
                    .to_owned(),
                false,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    MAX_IMAGE_BYTES + 1
                ),
                false,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/image", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let byte = stream.read_u8().await.unwrap();
                    request.push(byte);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(!request.contains("x-session-token"));
                assert!(!request.contains("authorization:"));
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            let policy = HttpPolicy::new(Arc::new(MediaSession)).unwrap();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                fetch_image(&policy, &url),
            )
            .await
            .unwrap();
            assert_eq!(result.is_ok(), expected);
            if expected {
                assert_eq!(result.unwrap(), b"PNG");
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn logout_cancels_an_inflight_media_download() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/slow", listener.local_addr().unwrap());
        let policy = Arc::new(HttpPolicy::new(Arc::new(MediaSession)).unwrap());
        let request_policy = policy.clone();
        let request = tokio::spawn(async move { fetch_image(&request_policy, &url).await });
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
    #[test]
    fn validates_media_urls_without_allowing_credentials_or_local_files() {
        for value in [
            "file:///etc/passwd",
            "data:image/png;base64,AA",
            "https://user:secret@example.com/a",
            "https://",
        ] {
            assert!(image_url(value).is_err());
        }
        assert!(image_url("https://cdn.example.com/avatars/a").is_ok());
        assert!(image_url("http://localhost:8080/files/a").is_ok());
    }
    #[test]
    fn file_paths_are_encoded_and_keep_deployment_prefixes() {
        assert_eq!(
            file_url(Some("https://cdn.example.com/media/"), "avatars", "a/b ?"),
            Some("https://cdn.example.com/media/avatars/a%2Fb%20%3F".into())
        );
        assert!(file_url(None, "avatars", "id").is_none());
        assert!(file_url(Some("file:///tmp"), "avatars", "id").is_none());
    }

    #[test]
    fn avatar_paths_preserve_original_animation() {
        assert_eq!(
            avatar_url(Some("https://cdn.example.com/media"), "avatars", "id"),
            Some("https://cdn.example.com/media/avatars/id/original".into())
        );
    }

    #[tokio::test]
    async fn follows_original_media_redirects_without_sending_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/avatars/id/original",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            for hop in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    request.push(stream.read_u8().await.unwrap());
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(
                    !request.contains("authorization:") && !request.contains("x-session-token")
                );
                let response = if hop == 0 {
                    "HTTP/1.1 308 Permanent Redirect\r\nLocation: /avatars/id/file.gif\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    assert!(request.starts_with("get /avatars/id/file.gif"));
                    "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nGIF"
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let policy = HttpPolicy::new(Arc::new(MediaSession)).unwrap();
        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                fetch_image(&policy, &url)
            )
            .await
            .unwrap()
            .unwrap(),
            b"GIF"
        );
        server.await.unwrap();
        assert!(redirect_url(
            &Url::parse("https://example.com").unwrap(),
            "http://example.com/file"
        )
        .is_err());
    }
}
