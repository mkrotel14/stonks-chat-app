//! Boundary for the real attachment transport. The composer owns scheduling,
//! draft revisions and retry identity; the adapter owns upload and message I/O.
use super::super::state::Attempt;
use std::{future::Future, pin::Pin, sync::Arc};
use stonks_core::messages::models::TimelineMessage;

pub struct SendFailure {
    /// Display-safe, localized error (no private paths or credentials).
    pub message: String,
    /// Ordered, reusable upload IDs for the successfully uploaded file prefix.
    /// Include the incoming prefix. Return it unchanged if nothing was added.
    pub uploaded: Vec<String>,
}

/// Called on the core runtime for attachment-bearing attempts only. Upload
/// `files[uploaded.len().min(attempt.files.len())..]` in order, honor spoiler metadata, then submit with
/// the complete ID list, `content`, `reply_to` and the original idempotency key.
/// Return success only after the server confirms and normalizes the message.
/// Dropping the future on account reset must stop further stages of work.
pub type AttachmentRoute = Arc<
    dyn Fn(Attempt) -> Pin<Box<dyn Future<Output = Result<TimelineMessage, SendFailure>> + Send>>
        + Send
        + Sync,
>;

pub fn core_attachment_route(policy: Arc<stonks_core::http::HttpPolicy>) -> AttachmentRoute {
    use stonks_core::{
        messages::{models::SendMessageRequest, service},
        uploads,
    };
    Arc::new(move |attempt| {
        let policy = policy.clone();
        Box::pin(async move {
            let mut uploaded = attempt.uploaded;
            let result = async {
                let pending = attempt
                    .files
                    .get(uploaded.len()..)
                    .ok_or_else(|| stonks_ui::i18n::t("messages.attachmentSendUnavailable"))?;
                for file in pending {
                    let (dir, path) = tokio::task::spawn_blocking({
                        let file = file.clone();
                        move || stage(&file)
                    })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .ok_or_else(|| stonks_ui::i18n::t("messages.attachmentReadFailed"))?;
                    let result = uploads::upload_attachment(&policy, &path, file.spoiler).await;
                    drop(dir);
                    uploaded.push(result.map_err(|error| error.to_string())?.id);
                }
                service::send_message(
                    &policy,
                    SendMessageRequest {
                        channel_id: attempt.channel,
                        content: Some(attempt.content),
                        attachments: uploaded.clone(),
                        reply_to: attempt.reply_to,
                        idempotency_key: attempt.key,
                    },
                )
                .await
                .map_err(|error| error.to_string())
            }
            .await;
            result.map_err(|message| SendFailure { message, uploaded })
        })
    })
}

// Core accepts a native path. Stage the captured bytes, not the original path,
// so the preview and retries refer to the same file content.
fn stage(file: &super::Attachment) -> std::io::Result<(tempfile::TempDir, std::path::PathBuf)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join(&file.filename);
    std::fs::write(&path, &file.bytes)?;
    Ok((dir, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_captured_bytes_with_original_name_and_cleans_up() {
        let file = super::super::Attachment {
            id: 0,
            filename: "some file.pdf".into(),
            bytes: b"captured file".as_slice().into(),
            spoiler: true,
        };
        let (dir, path) = stage(&file).unwrap();
        assert_eq!(path.file_name().unwrap(), "some file.pdf");
        assert_eq!(std::fs::read(&path).unwrap(), b"captured file");
        drop(dir);
        assert!(!path.exists());
    }
}
