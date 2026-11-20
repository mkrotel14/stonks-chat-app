//! Credential-free streamed Save As. Failed/cancelled transfers never replace the destination.
use futures_util::StreamExt;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use stonks_core::http::HttpPolicy;
use tokio::io::AsyncWriteExt;
use url::Url;
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);
fn safe_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "The media URL is invalid.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("The media URL is invalid.".into());
    }
    Ok(url)
}
pub fn filename(value: &str) -> String {
    let value = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("media")
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect::<String>();
    if value.is_empty() || matches!(value.as_str(), "." | "..") {
        "media".into()
    } else {
        value
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub async fn save(policy: &HttpPolicy, url: &str, destination: &Path) -> Result<(), String> {
    save_bounded(policy, url, destination, MAX_DOWNLOAD_BYTES).await
}
async fn save_bounded(
    policy: &HttpPolicy,
    url: &str,
    destination: &Path,
    limit: u64,
) -> Result<(), String> {
    let context = policy
        .require_deployment()
        .map_err(|_| "The selected server is unavailable.")?;
    let mut url = safe_url(url)?;
    let parent = destination.parent().ok_or("Choose a valid destination.")?;
    let temporary = Temporary(parent.join(format!(
        ".stonks-media-{}-{}.part",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )));
    let path = temporary.0.clone();
    policy
        .run_deployment(
            &context,
            async {
                let mut response = None;
                for hop in 0..=3 {
                    let received = policy
                        .client()
                        .get(url.clone())
                        .timeout(Duration::from_secs(300))
                        .send()
                        .await
                        .map_err(|_| "The media could not be downloaded. Try again.")?;
                    if received.status().is_redirection() {
                        if hop == 3 {
                            return Err("The media redirected too many times.".into());
                        }
                        let location = received
                            .headers()
                            .get("location")
                            .and_then(|value| value.to_str().ok())
                            .ok_or("The media redirect is invalid.")?;
                        let next = safe_url(
                            url.join(location)
                                .map_err(|_| "The media redirect is invalid.")?
                                .as_str(),
                        )?;
                        if url.scheme() == "https" && next.scheme() != "https" {
                            return Err("The media redirect is invalid.".into());
                        }
                        url = next;
                        continue;
                    }
                    if !received.status().is_success() {
                        return Err("The media could not be downloaded. Try again.".into());
                    }
                    if received
                        .content_length()
                        .is_some_and(|length| length > limit)
                    {
                        return Err("This file exceeds the native download size limit.".into());
                    }
                    response = Some(received);
                    break;
                }
                let mut stream = response
                    .ok_or("The media could not be downloaded.")?
                    .bytes_stream();
                let mut output = tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                    .await
                    .map_err(|_| "The destination could not be written.")?;
                let mut count = 0_u64;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| "The media transfer was interrupted.")?;
                    count = count
                        .checked_add(chunk.len() as u64)
                        .filter(|count| *count <= limit)
                        .ok_or("This file exceeds the native download size limit.")?;
                    output
                        .write_all(&chunk)
                        .await
                        .map_err(|_| "The destination could not be written.")?;
                }
                output
                    .flush()
                    .await
                    .map_err(|_| "The destination could not be written.")?;
                drop(output);
                Ok(())
            },
            || "The download was cancelled.".to_owned(),
        )
        .await?;
    tokio::fs::rename(&temporary.0, destination)
        .await
        .map_err(|_| "The downloaded file could not be saved.")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use stonks_core::{
        http::{ContextError, SessionAccess},
        server::ServerSelection,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct Session;
    impl SessionAccess for Session {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(serde_json::from_value(serde_json::json!({"deploymentUrl":"https://example.com","displayName":"Test","official":false,"apiVersion":"test","endpoints":{"api":"https://example.com/api","websocket":"wss://example.com","autumn":null,"january":null,"gifbox":null,"livekitEnabled":false}})).unwrap()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            panic!("downloads must not request API credentials")
        }
    }
    #[test]
    fn filenames_cannot_traverse_or_inject_controls() {
        assert_eq!(filename("../../audio.mp3"), "audio.mp3");
        assert_eq!(filename("C:\\private\\video.mp4"), "video.mp4");
        assert_eq!(filename(".."), "media");
        assert_eq!(filename("name\n.mp3"), "name.mp3");
    }
    #[tokio::test]
    async fn streams_without_credentials_and_preserves_old_file_on_failure() {
        let directory = std::env::temp_dir().join(format!(
            "stonks-media-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        tokio::fs::create_dir(&directory).await.unwrap();
        let destination = directory.join("audio.mp3");
        tokio::fs::write(&destination, b"old").await.unwrap();
        for (body, limit, success) in [
            (b"new".as_slice(), 8, true),
            (b"too large".as_slice(), 2, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/audio", listener.local_addr().unwrap());
            let bytes = body.to_vec();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                loop {
                    request.push(stream.read_u8().await.unwrap());
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(
                    !request.contains("authorization:")
                        && !request.contains("x-session-token:")
                        && !request.contains("cookie:")
                );
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(&bytes).await.unwrap();
            });
            let policy = HttpPolicy::new(Arc::new(Session)).unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                save_bounded(&policy, &url, &destination, limit),
            )
            .await
            .unwrap();
            assert_eq!(result.is_ok(), success);
            assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"new");
            server.await.unwrap();
        }
        let mut entries = tokio::fs::read_dir(&directory).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_some());
        assert!(entries.next_entry().await.unwrap().is_none());
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }
}
