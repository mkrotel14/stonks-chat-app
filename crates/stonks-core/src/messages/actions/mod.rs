//! Native equivalents of the Tauri message mutation commands.
use std::collections::HashMap;

use reqwest::{Method, Url};
use serde_json::{json, Value};

use super::{
    models::TimelineMessage,
    normalize::{normalize_message, valid_id},
    request::{
        authenticated_context, consume_json, execute_authenticated, invalid_input,
        invalid_response, message_status_error, message_url, unavailable,
    },
};
use crate::{
    channel_actions::MANAGE_MESSAGES,
    connection::ConnectionManager,
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass},
};

pub const SEND_MESSAGE: u64 = 1 << 22;

fn validate_target(channel: &str, message: &str, error: &'static str) -> Result<(), CommandError> {
    if valid_id(channel) && valid_id(message) {
        Ok(())
    } else {
        Err(invalid_input(error))
    }
}

fn require(
    manager: &ConnectionManager,
    channel: &str,
    permission: u64,
) -> Result<(), CommandError> {
    manager
        .require_channel_permission(channel, permission)
        .map_err(|_| {
            CommandError::new(
                ErrorCode::AuthenticationFailed,
                "You do not have permission to perform this message action.",
            )
        })
}

pub async fn edit_message(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel: &str,
    message: &str,
    content: &str,
) -> Result<TimelineMessage, CommandError> {
    validate_target(
        channel,
        message,
        "The edit request contained an invalid ID.",
    )?;
    let content = content.trim_end();
    if content.is_empty() {
        return Err(invalid_input("A message cannot be empty."));
    }
    require(manager, channel, SEND_MESSAGE)?;
    let context = authenticated_context(policy)?;
    let request = policy
        .client()
        .patch(message_url(&context.server, channel, Some(message))?)
        .header("x-session-token", &context.token)
        .json(&json!({"content": content}));
    let failure = "Could not edit the message.";
    let response =
        execute_authenticated(policy, &context, request, RequestClass::Standard, failure).await?;
    consume_json(
        response,
        "The message could not be edited",
        failure,
        |value| {
            normalize_message(
                &value,
                &HashMap::new(),
                &HashMap::new(),
                context.server.endpoints.autumn.as_deref(),
            )
        },
    )
}

/// The backend authorizes deletion for authors or moderators. Local state cannot
/// authoritatively establish ownership, so only preflight channel visibility.
pub async fn delete_message(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel: &str,
    message: &str,
) -> Result<(), CommandError> {
    validate_target(
        channel,
        message,
        "The delete request contained an invalid ID.",
    )?;
    require(manager, channel, 1 << 20)?;
    manage_message(policy, channel, message, None).await
}

pub async fn set_message_pinned(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel: &str,
    message: &str,
    pinned: bool,
) -> Result<(), CommandError> {
    validate_target(channel, message, "The pin request contained an invalid ID.")?;
    require(manager, channel, MANAGE_MESSAGES)?;
    manage_message(policy, channel, message, Some(pinned)).await
}

async fn manage_message(
    policy: &HttpPolicy,
    channel: &str,
    message: &str,
    pin: Option<bool>,
) -> Result<(), CommandError> {
    let context = authenticated_context(policy)?;
    let mut url = message_url(&context.server, channel, Some(message))?;
    let (method, failure, rejected) = if let Some(pinned) = pin {
        url.path_segments_mut()
            .map_err(|_| {
                CommandError::new(
                    ErrorCode::NotConfigured,
                    "The selected Stoat API address is invalid.",
                )
            })?
            .push("pin");
        (
            if pinned { Method::POST } else { Method::DELETE },
            "Could not update the pinned message.",
            "The server rejected the pin request.",
        )
    } else {
        (
            Method::DELETE,
            "Could not delete the message.",
            "The message could not be deleted.",
        )
    };
    let request = policy
        .client()
        .request(method, url)
        .header("x-session-token", &context.token);
    execute_authenticated(policy, &context, request, RequestClass::Standard, failure)
        .await?
        .consume(
            |status, _, _| {
                if status.is_success() {
                    Ok(())
                } else {
                    Err(message_status_error(status, rejected))
                }
            },
            || unavailable(failure),
        )
}

/// Submit a message report. As at the Tauri command boundary, context is sent verbatim;
/// presenters can trim the optional prompt before calling this function.
pub async fn report_message(
    policy: &HttpPolicy,
    message: &str,
    additional_context: &str,
) -> Result<(), CommandError> {
    // The safety endpoint has a different ID validator from the channel-message commands.
    if message.is_empty()
        || message.len() > 64
        || !message.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(invalid_input("The report target is invalid."));
    }
    if additional_context.chars().count() > 1000 {
        return Err(invalid_input(
            "The report description must be 1000 characters or fewer.",
        ));
    }
    let context = authenticated_context(policy)?;
    let bad_url = || {
        CommandError::new(
            ErrorCode::NotConfigured,
            "The selected server API is unavailable.",
        )
    };
    let mut url = Url::parse(&context.server.endpoints.api).map_err(|_| bad_url())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(bad_url());
    }
    url.path_segments_mut()
        .map_err(|_| bad_url())?
        .extend(["safety", "report"]);
    let request = policy
        .client()
        .post(url)
        .header("x-session-token", &context.token)
        .json(&json!({
            "content": {"type": "Message", "id": message, "report_reason": "NoneSpecified"},
            "additional_context": additional_context,
        }));
    let failure = "Could not send the report.";
    execute_authenticated(policy, &context, request, RequestClass::Standard, failure)
        .await?
        .consume(
            |status, _, bytes| {
                if !status.is_success() {
                    let code = match status.as_u16() {
                        401 | 403 => ErrorCode::AuthenticationFailed,
                        404 => ErrorCode::NotFound,
                        400..=499 if status.as_u16() != 429 => ErrorCode::InvalidInput,
                        _ => ErrorCode::Unavailable,
                    };
                    return Err(CommandError::new(code, "The report could not be sent"));
                }
                if !bytes.is_empty() {
                    serde_json::from_slice::<Value>(bytes).map_err(|_| {
                        invalid_response("The Stoat server returned an invalid response.")
                    })?;
                }
                Ok(())
            },
            || unavailable(failure),
        )
}

#[cfg(test)]
mod tests;
