use reqwest::StatusCode;
use serde_json::Value;
use url::Url;

use crate::http::{
    AuthenticatedContext, BoundedResponse, ContextError, HttpPolicy, HttpPolicyError, RequestClass,
    MAX_JSON_RESPONSE_BYTES,
};
use crate::error::{CommandError, ErrorCode};
use crate::server::ServerSelection;

use super::models::{MessageQuery, ReactionRequest};
use super::normalize::valid_id;

fn error(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, message)
}

pub fn invalid_input(message: &'static str) -> CommandError {
    error(ErrorCode::InvalidInput, message)
}

pub fn unavailable(message: &'static str) -> CommandError {
    error(ErrorCode::Unavailable, message)
}

pub fn invalid_response(message: &'static str) -> CommandError {
    error(ErrorCode::InvalidResponse, message)
}

pub fn message_status_error(status: StatusCode, message: &'static str) -> CommandError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        error(
            ErrorCode::AuthenticationFailed,
            "The server could not authorize the message request.",
        )
    } else if status == StatusCode::NOT_FOUND {
        error(
            ErrorCode::NotFound,
            "The requested message resource was not found.",
        )
    } else if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
        error(ErrorCode::InvalidInput, message)
    } else {
        error(ErrorCode::Unavailable, message)
    }
}

pub fn json_response_bytes(
    status: StatusCode,
    bytes: &[u8],
    failure_message: &'static str,
) -> Result<Value, CommandError> {
    if !status.is_success() {
        return Err(message_status_error(status, failure_message));
    }
    serde_json::from_slice(bytes)
        .map_err(|_| invalid_response("The Stoat server returned an invalid message response."))
}

pub async fn request_messages(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    query: &MessageQuery,
) -> Result<BoundedResponse, CommandError> {
    let mut url = Url::parse(&context.server.endpoints.api).map_err(|_| {
        error(
            ErrorCode::NotConfigured,
            "The selected Stoat API address is invalid.",
        )
    })?;
    url.path_segments_mut()
        .map_err(|_| {
            error(
                ErrorCode::NotConfigured,
                "The selected Stoat API address is invalid.",
            )
        })?
        .extend(["channels", &query.channel_id, "messages"]);
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("limit", "50")
            .append_pair("sort", "Latest")
            .append_pair("include_users", "true");
        if let Some(before) = query.before.as_deref() {
            pairs.append_pair("before", before);
        }
        if let Some(nearby) = query.nearby.as_deref() {
            pairs.append_pair("nearby", nearby);
        }
    }
    let request = policy
        .client()
        .get(url)
        .header("x-session-token", &context.token);
    execute_authenticated(
        policy,
        context,
        request,
        RequestClass::Standard,
        "Could not fetch messages.",
    )
    .await
}

pub fn message_url(
    server: &ServerSelection,
    channel_id: &str,
    message_id: Option<&str>,
) -> Result<Url, CommandError> {
    let mut url = Url::parse(&server.endpoints.api).map_err(|_| {
        error(
            ErrorCode::NotConfigured,
            "The selected Stoat API address is invalid.",
        )
    })?;
    let mut path = url.path_segments_mut().map_err(|_| {
        error(
            ErrorCode::NotConfigured,
            "The selected Stoat API address is invalid.",
        )
    })?;
    path.pop_if_empty().extend(["channels", channel_id, "messages"]);
    if let Some(message_id) = message_id {
        path.push(message_id);
    }
    drop(path);
    Ok(url)
}

pub fn consume_json<T>(
    response: BoundedResponse,
    failure_message: &'static str,
    cancelled_message: &'static str,
    parse: impl FnOnce(Value) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    response.consume(
        |status, _headers, bytes| {
            json_response_bytes(status, bytes, failure_message).and_then(parse)
        },
        || unavailable(cancelled_message),
    )
}

pub fn authenticated_context(
    policy: &HttpPolicy,
) -> Result<AuthenticatedContext, CommandError> {
    policy
        .authenticated_context()
        .map_err(|context_error| match context_error {
            ContextError::NotConfigured => {
                error(ErrorCode::NotConfigured, "Choose a Stoat server first.")
            }
            ContextError::ConfigurationUnavailable => {
                unavailable("Could not load the server configuration.")
            }
            ContextError::SecureStorage => error(
                ErrorCode::SecureStorage,
                "Could not load the saved Stoat session.",
            ),
            ContextError::AuthenticationRequired => error(
                ErrorCode::AuthenticationFailed,
                "Your session expired. Log in again.",
            ),
            ContextError::Cancelled => unavailable("Could not load the server configuration."),
        })
}

pub async fn execute_authenticated(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    request: reqwest::RequestBuilder,
    class: RequestClass,
    unavailable_message: &'static str,
) -> Result<BoundedResponse, CommandError> {
    policy
        .execute_authenticated(context, request, class, MAX_JSON_RESPONSE_BYTES)
        .await
        .map_err(|error| match error {
            HttpPolicyError::InvalidResponse | HttpPolicyError::BodyTooLarge => {
                invalid_response("The Stoat server returned an invalid message response.")
            }
            HttpPolicyError::Unavailable
            | HttpPolicyError::Timeout
            | HttpPolicyError::Cancelled => unavailable(unavailable_message),
        })
}

pub fn reaction_url(
    server: &ServerSelection,
    request: &ReactionRequest,
) -> Result<Url, CommandError> {
    let mut url = message_url(server, &request.channel_id, Some(&request.message_id))?;
    url.path_segments_mut()
        .map_err(|_| {
            error(
                ErrorCode::NotConfigured,
                "The selected Stoat API address is invalid.",
            )
        })?
        .extend(["reactions", &request.emoji]);
    Ok(url)
}

pub fn validate_reaction_request(request: &ReactionRequest) -> Result<(), CommandError> {
    if !valid_id(&request.channel_id) || !valid_id(&request.message_id) {
        return Err(invalid_input(
            "The reaction request contained an invalid ID.",
        ));
    }
    if request.emoji.is_empty()
        || request.emoji.len() > 128
        || request.emoji.chars().any(char::is_control)
    {
        return Err(invalid_input("The selected reaction is invalid."));
    }
    Ok(())
}

pub fn reaction_response(
    response: BoundedResponse,
    cancelled_message: &'static str,
) -> Result<(), CommandError> {
    response.consume(
        |status, _headers, _bytes| {
            if status.is_success() {
                Ok(())
            } else {
                Err(message_status_error(
                    status,
                    "The reaction could not be updated.",
                ))
            }
        },
        || unavailable(cancelled_message),
    )
}
