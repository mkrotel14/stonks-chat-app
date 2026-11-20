//! Authenticated REST calls that back the connection surface (groups,
//! members, users). Tauri-free: callers pass the shared [`HttpPolicy`].

use serde_json::Value;

use super::gateway::decode_items;
use super::models::{ClientChannel, ClientUser, CreateGroupRequest, ServerMemberSnapshot};
use crate::error::{CommandError, ErrorCode};
use crate::http::{
    AuthenticatedContext, ContextError, HttpPolicy, HttpPolicyError, RequestClass,
    MAX_JSON_RESPONSE_BYTES,
};

pub fn selected_session(policy: &HttpPolicy) -> Result<AuthenticatedContext, CommandError> {
    policy.authenticated_context().map_err(|error| match error {
        ContextError::NotConfigured => {
            CommandError::new(ErrorCode::NotConfigured, "Choose a Stoat server first.")
        }
        ContextError::ConfigurationUnavailable => CommandError::new(
            ErrorCode::NotConfigured,
            "The saved server configuration is unavailable.",
        ),
        ContextError::SecureStorage => CommandError::new(
            ErrorCode::SecureStorage,
            "The saved session could not be loaded securely.",
        ),
        ContextError::AuthenticationRequired | ContextError::Cancelled => CommandError::new(
            ErrorCode::AuthenticationFailed,
            "Your session expired. Log in again.",
        ),
    })
}

fn unavailable(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::Unavailable, message)
}

fn invalid_response(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidResponse, message)
}

fn request_error(
    error: HttpPolicyError,
    unavailable_message: &'static str,
    invalid_response_message: &'static str,
) -> CommandError {
    match error {
        HttpPolicyError::InvalidResponse | HttpPolicyError::BodyTooLarge => {
            invalid_response(invalid_response_message)
        }
        HttpPolicyError::Unavailable | HttpPolicyError::Timeout | HttpPolicyError::Cancelled => {
            unavailable(unavailable_message)
        }
    }
}

pub async fn create_group(
    policy: &HttpPolicy,
    request: CreateGroupRequest,
) -> Result<ClientChannel, CommandError> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "The group name must contain between 1 and 32 characters.",
        ));
    }
    if request.recipients.is_empty() || request.recipients.len() > 49 {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "Select at least one friend for the group.",
        ));
    }
    let context = selected_session(policy)?;
    let request = policy
        .client()
        .post(format!("{}/channels/create", context.server.endpoints.api))
        .header("x-session-token", &context.token)
        .json(&serde_json::json!({ "type": "Group", "name": name, "users": request.recipients }));
    let response = policy
        .execute_authenticated(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| {
            request_error(
                error,
                "The group could not be created. Try again.",
                "The server returned an invalid group response.",
            )
        })?;
    response.consume(
        |status, _headers, bytes| {
            if !status.is_success() {
                return Err(if status.as_u16() == 401 || status.as_u16() == 403 {
                    CommandError::new(
                        ErrorCode::AuthenticationFailed,
                        "Your session could not create this group. Log in again.",
                    )
                } else {
                    unavailable("The group could not be created. Try again.")
                });
            }
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| invalid_response("The server returned an invalid group response."))?;
            serde_json::from_value(value)
                .map_err(|_| invalid_response("The server returned an invalid group response."))
        },
        || unavailable("The group could not be created. Try again."),
    )
}

pub async fn fetch_server_members(
    policy: &HttpPolicy,
    server_id: String,
) -> Result<ServerMemberSnapshot, CommandError> {
    if server_id.is_empty()
        || server_id.len() > 64
        || !server_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "The member request contained an invalid server ID.",
        ));
    }
    let context = selected_session(policy)?;
    let request = policy
        .client()
        .get(format!(
            "{}/servers/{server_id}/members",
            context.server.endpoints.api
        ))
        .query(&[("exclude_offline", "false")])
        .header("x-session-token", &context.token);
    let response = policy
        .execute_authenticated(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| {
            request_error(
                error,
                "Server members could not be loaded. Try again.",
                "The server returned an invalid member response.",
            )
        })?;
    response.consume(
        |status, _headers, bytes| {
            if !status.is_success() {
                return Err(if status.as_u16() == 401 || status.as_u16() == 403 {
                    CommandError::new(
                        ErrorCode::AuthenticationFailed,
                        "Your session could not load server members. Log in again.",
                    )
                } else {
                    unavailable("Server members could not be loaded. Try again.")
                });
            }
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| invalid_response("The server returned an invalid member response."))?;
            let users = decode_items(
                value
                    .get("users")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        invalid_response("The server returned an invalid member response.")
                    })?,
                "server member users",
            )
            .map_err(|_| invalid_response("The server returned an invalid member response."))?;
            let members = decode_items(
                value
                    .get("members")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        invalid_response("The server returned an invalid member response.")
                    })?,
                "server members",
            )
            .map_err(|_| invalid_response("The server returned an invalid member response."))?;
            Ok(ServerMemberSnapshot { users, members })
        },
        || unavailable("Server members could not be loaded. Try again."),
    )
}

pub async fn fetch_user(policy: &HttpPolicy, user_id: String) -> Result<ClientUser, CommandError> {
    if user_id.is_empty()
        || user_id.len() > 64
        || !user_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "The user request contained an invalid user ID.",
        ));
    }
    let context = selected_session(policy)?;
    let request = policy
        .client()
        .get(format!("{}/users/{user_id}", context.server.endpoints.api))
        .header("x-session-token", &context.token);
    let response = policy
        .execute_authenticated(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| {
            request_error(
                error,
                "The user could not be loaded. Try again.",
                "The server returned an invalid user response.",
            )
        })?;
    response.consume(
        |status, _headers, bytes| {
            if !status.is_success() {
                return Err(if status.as_u16() == 404 {
                    CommandError::new(ErrorCode::NotFound, "The requested user was not found.")
                } else if status.as_u16() == 401 || status.as_u16() == 403 {
                    CommandError::new(
                        ErrorCode::AuthenticationFailed,
                        "Your session could not load this user. Log in again.",
                    )
                } else {
                    unavailable("The user could not be loaded. Try again.")
                });
            }
            serde_json::from_slice::<ClientUser>(bytes)
                .map_err(|_| invalid_response("The server returned an invalid user response."))
        },
        || unavailable("The user could not be loaded. Try again."),
    )
}
