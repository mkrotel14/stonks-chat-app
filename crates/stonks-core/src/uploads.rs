//! Authenticated native uploads to the selected deployment’s Autumn service.
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::{
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass},
    messages::request::{
        authenticated_context, consume_json, execute_authenticated, invalid_input, invalid_response,
    },
};
use reqwest::multipart;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;
#[derive(Clone, Debug)]
pub struct UploadFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
impl UploadFile {
    pub fn validate(&self, limit: usize) -> Result<(), CommandError> {
        if self.bytes.is_empty()
            || self.bytes.len() > limit
            || !self.content_type.starts_with("image/")
            || self.filename.is_empty()
            || self.filename.chars().any(char::is_control)
        {
            return Err(invalid_input("Choose a valid image within the size limit."));
        }
        Ok(())
    }
}
pub async fn upload_image(
    policy: &HttpPolicy,
    tag: &str,
    file: &UploadFile,
) -> Result<String, CommandError> {
    if !matches!(tag, "avatars" | "backgrounds" | "icons" | "emojis") {
        return Err(invalid_input("The image category is invalid."));
    }
    file.validate(if tag == "emojis" {
        2_000_000
    } else {
        8_000_000
    })?;
    let context = authenticated_context(policy)?;
    let mut url = autumn_endpoint(&context.server)?;
    url.path_segments_mut()
        .map_err(|_| invalid_input("The file service address is invalid."))?
        .pop_if_empty()
        .push(tag);
    let part = multipart::Part::bytes(file.bytes.clone())
        .file_name(file.filename.clone())
        .mime_str(&file.content_type)
        .map_err(|_| invalid_input("The image type is invalid."))?;
    let request = policy
        .client()
        .post(url)
        .header("x-session-token", &context.token)
        .multipart(multipart::Form::new().part("file", part));
    let response = execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Transfer,
        "Could not upload the image.",
    )
    .await?;
    consume_json(
        response,
        "The image could not be uploaded.",
        "The image upload was cancelled.",
        |value| {
            value
                .as_str()
                .or_else(|| {
                    value
                        .get("id")
                        .or_else(|| value.get("_id"))
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    value
                        .get("file")
                        .and_then(|file| file.get("id").or_else(|| file.get("_id")))
                        .and_then(serde_json::Value::as_str)
                })
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| invalid_response("The image upload response had no ID."))
        },
    )
}
/// Upload a file chosen by the trusted native caller (not an IPC path or grant).
/// Streams no more than the validated length from the opened regular file.
pub async fn upload_attachment(
    policy: &HttpPolicy,
    path: &Path,
    spoiler: bool,
) -> Result<crate::messages::models::UploadedAttachment, CommandError> {
    use crate::messages::models::{attachment_content_type, UploadedAttachment};

    let context = authenticated_context(policy)?;
    let mut url = autumn_endpoint(&context.server)?;
    url.path_segments_mut()
        .map_err(|_| invalid_input("The file service address is invalid."))?
        .pop_if_empty()
        .push("attachments");
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.len() <= 255 && !name.chars().any(char::is_control))
        .ok_or_else(|| invalid_input("The selected file has an invalid filename."))?;
    let content_type = attachment_content_type(filename);
    let filename = if spoiler && !filename.starts_with("SPOILER_") {
        format!("SPOILER_{filename}")
    } else {
        filename.to_owned()
    };
    let path = path.to_owned();
    let limit = context.server.attachment_size_limit();
    let (file, size) = tokio::task::spawn_blocking(move || open_attachment(&path, limit))
        .await
        .map_err(|_| invalid_input("Could not open the selected attachment."))??;
    let reader = tokio::fs::File::from_std(file).take(size);
    let part = multipart::Part::stream_with_length(
        reqwest::Body::wrap_stream(ReaderStream::new(reader)),
        size,
    )
    .file_name(filename)
    .mime_str(content_type)
    .map_err(|_| invalid_input("The selected file type is invalid."))?;
    let request = policy
        .client()
        .post(url)
        .header("x-session-token", &context.token)
        .multipart(multipart::Form::new().part("file", part));
    let response = execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Transfer,
        "Could not upload the attachment.",
    )
    .await?;
    consume_json(
        response,
        "The attachment could not be uploaded.",
        "The attachment upload was cancelled.",
        |value| {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.trim().is_empty() && !id.chars().any(char::is_control))
                .ok_or_else(|| {
                    invalid_response("The attachment service returned an invalid upload response.")
                })?;
            Ok(UploadedAttachment { id: id.to_owned() })
        },
    )
}

fn autumn_endpoint(server: &crate::server::ServerSelection) -> Result<url::Url, CommandError> {
    let autumn = server.endpoints.autumn.as_deref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotConfigured,
            "This server does not advertise a file service.",
        )
    })?;
    crate::server::validate_selection(server)?;
    url::Url::parse(autumn).map_err(|_| invalid_input("The file service address is invalid."))
}

fn open_attachment(path: &Path, limit: u64) -> Result<(std::fs::File, u64), CommandError> {
    let open_error = || invalid_input("Could not open the selected attachment.");
    // A selected path can be replaced by a FIFO before open. Don't wait for a writer.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(path).map_err(|_| open_error())?;
    let metadata = file.metadata().map_err(|_| open_error())?;
    let size = metadata.len();
    if !metadata.is_file() || size == 0 || size > limit {
        return Err(invalid_input(
            "Choose a non-empty regular file within the server's attachment size limit.",
        ));
    }
    Ok((file, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::{ContextError, SessionAccess},
        server::ServerSelection,
    };
    use std::{sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::oneshot,
    };

    struct Session(ServerSelection);
    impl SessionAccess for Session {
        fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
            Ok(Some(self.0.clone()))
        }
        fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
            Ok(Some("local-avatar-session".into()))
        }
    }
    fn policy(origin: &str) -> Arc<HttpPolicy> {
        Arc::new(HttpPolicy::new(Arc::new(Session(serde_json::from_value(serde_json::json!({
            "deploymentUrl": origin, "displayName": "Fixture", "official": false, "apiVersion": "test",
            "capabilities": {"emailVerification": false, "inviteOnly": false, "captcha": {"enabled": false}, "attachmentSizeLimit": 18},
            "endpoints": {"api": format!("{origin}/api"), "websocket": "wss://example.com", "autumn": format!("{origin}/media/"), "january": null, "gifbox": null, "livekitEnabled": false}
        })).unwrap()))).unwrap())
    }
    fn file() -> UploadFile {
        UploadFile {
            filename: "profile.gif".into(),
            content_type: "image/gif".into(),
            bytes: b"GIF89a\0fixture-pixels".to_vec(),
        }
    }
    async fn read_request(socket: &mut TcpStream) -> (String, Vec<u8>) {
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert!(count > 0, "upload ended before its declared multipart body");
            bytes.extend_from_slice(&chunk[..count]);
            assert!(
                bytes.len() <= 64 * 1024,
                "fixture request exceeded its bound"
            );
            let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("multipart request must declare its exact body length");
            if bytes.len() >= end + 4 + length {
                return (headers, bytes[end + 4..end + 4 + length].to_vec());
            }
        }
    }
    async fn respond(socket: &mut TcpStream, body: &str) {
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.ok();
    }
    #[test]
    fn bounds_and_types_are_validated() {
        let mut file = UploadFile {
            filename: "x.gif".into(),
            content_type: "image/gif".into(),
            bytes: vec![1, 2],
        };
        assert!(file.validate(2).is_ok());
        assert!(file.validate(1).is_err());
        file.content_type = "text/html".into();
        assert!(file.validate(2).is_err());
    }
    struct AttachmentFixture(std::path::PathBuf);
    impl AttachmentFixture {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("stonks-attachment-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for AttachmentFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn attachment_open_rejects_missing_empty_large_and_non_regular_files() {
        let fixture = AttachmentFixture::new();
        let path = fixture.0.join("notes.txt");
        assert!(open_attachment(&fixture.0, 10).is_err());
        assert!(open_attachment(&path, 10).is_err());
        std::fs::write(&path, []).unwrap();
        assert!(open_attachment(&path, 10).is_err());
        std::fs::write(&path, b"abc").unwrap();
        assert!(open_attachment(&path, 2).is_err());
        assert_eq!(open_attachment(&path, 3).unwrap().1, 3);
    }

    #[test]
    fn autumn_endpoint_checks_transport_and_deployment_before_authentication_on_wire() {
        let mut server = policy("https://chat.example.com")
            .authenticated_context()
            .unwrap()
            .server;
        for autumn in [
            "http://cdn.example.com/media",
            "https://localhost/media",
            "https://user:pass@cdn.example.com/media",
            "https://cdn.example.com/media?key=secret",
            "https://cdn.example.com/media#fragment",
        ] {
            server.endpoints.autumn = Some(autumn.into());
            assert!(autumn_endpoint(&server).is_err(), "accepted {autumn}");
        }
        server.endpoints.autumn = Some("https://cdn.example.com/media".into());
        assert!(autumn_endpoint(&server).is_ok());
    }

    #[tokio::test]
    async fn attachment_posts_spoiler_filename_and_regular_file_bytes_to_autumn() {
        let fixture = AttachmentFixture::new();
        let path = fixture.0.join("notes.txt");
        std::fs::write(&path, b"attachment fixture").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = policy(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = read_request(&mut socket).await;
            assert!(headers.starts_with("POST /media/attachments HTTP/1.1"));
            assert!(headers
                .to_lowercase()
                .contains("x-session-token: local-avatar-session"));
            let body = String::from_utf8(body).unwrap();
            assert!(body.contains("name=\"file\"; filename=\"SPOILER_notes.txt\""));
            assert!(body.contains("text/plain\r\n\r\nattachment fixture\r\n"));
            respond(&mut socket, r#"{"id":"autumn-file_id"}"#).await;
        });
        let uploaded = tokio::time::timeout(
            Duration::from_secs(3),
            upload_attachment(&policy, &path, true),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(uploaded.id, "autumn-file_id");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn avatar_upload_preserves_autumn_prefix_multipart_bytes_and_authentication() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = policy(&format!("http://{}", listener.local_addr().unwrap()));
        let selected = file();
        let expected = selected.bytes.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            respond(&mut socket, r#"{"file":{"_id":"autumn-file_id"}}"#).await;
            request
        });
        let id = tokio::time::timeout(
            Duration::from_secs(3),
            upload_image(&policy, "avatars", &selected),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(id, "autumn-file_id");
        let (headers, body) = server.await.unwrap();
        assert!(headers.starts_with("POST /media/avatars HTTP/1.1"));
        assert!(headers
            .to_lowercase()
            .contains("x-session-token: local-avatar-session"));
        let content_type = headers
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                    .map(|(_, value)| value.trim())
            })
            .unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert!(body.starts_with(format!("--{boundary}\r\n").as_bytes()));
        assert!(body.ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
        let metadata = String::from_utf8_lossy(&body);
        assert!(metadata.contains("name=\"file\"; filename=\"profile.gif\""));
        assert!(metadata.to_lowercase().contains("content-type: image/gif"));
        assert_eq!(metadata.matches("name=\"file\"").count(), 1);
        assert!(body
            .windows(expected.len())
            .any(|window| window == expected));
    }
    #[tokio::test]
    async fn logout_and_deployment_replacement_cancel_pending_uploads_before_late_success() {
        for deployment_replacement in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let policy = policy(&format!("http://{}", listener.local_addr().unwrap()));
            let (started, ready) = oneshot::channel();
            let (release, wait) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _ = read_request(&mut socket).await;
                started.send(()).unwrap();
                let _ = wait.await;
                respond(&mut socket, r#"{"id":"must-not-publish"}"#).await;
            });
            let sending_policy = policy.clone();
            let upload =
                tokio::spawn(
                    async move { upload_image(&sending_policy, "avatars", &file()).await },
                );
            tokio::time::timeout(Duration::from_secs(3), ready)
                .await
                .unwrap()
                .unwrap();
            if deployment_replacement {
                policy.deployment_replaced();
            } else {
                policy.logout();
            }
            let result = tokio::time::timeout(Duration::from_secs(1), upload)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap()["code"],
                "unavailable"
            );
            release.send(()).unwrap();
            server.await.unwrap();
        }
    }
}
