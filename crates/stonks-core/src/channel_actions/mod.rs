//! Channel management, preserving the existing native command contract.
use crate::{
    connection::{ClientChannel, ConnectionManager, PermissionOverride},
    error::{CommandError, ErrorCode},
    http::{
        AuthenticatedContext, ContextError, HttpPolicy, HttpPolicyError, RequestClass,
        MAX_JSON_RESPONSE_BYTES,
    },
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
mod reset;
pub use reset::reset;

pub const MANAGE_CHANNEL: u64 = 1;
pub const MANAGE_PERMISSIONS: u64 = 1 << 2;
pub const MANAGE_MESSAGES: u64 = 1 << 23;
pub const INVITE_OTHERS: u64 = 1 << 25;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelAccess {
    pub manage_channel: bool,
    pub manage_permissions: bool,
    pub manage_messages: bool,
    pub invite: bool,
}
impl ChannelAccess {
    pub fn from_manager(manager: &ConnectionManager, channel: &str) -> Self {
        let can = |bit| manager.require_channel_permission(channel, bit).is_ok();
        Self {
            manage_channel: can(MANAGE_CHANNEL),
            manage_permissions: can(MANAGE_PERMISSIONS),
            manage_messages: can(MANAGE_MESSAGES),
            invite: can(INVITE_OTHERS),
        }
    }
    pub fn settings(self) -> bool {
        self.manage_channel || self.manage_permissions
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub name: String,
    pub description: String,
    pub slowmode: u64,
    pub nsfw: bool,
}
impl Overview {
    pub fn from_channel(channel: &ClientChannel) -> Self {
        Self {
            name: channel.name.clone().unwrap_or_default(),
            description: channel.description.clone().unwrap_or_default(),
            slowmode: channel.slowmode.unwrap_or_default(),
            nsfw: channel.nsfw,
        }
    }
    pub fn body(&self) -> Result<Value, CommandError> {
        if self.name.trim().is_empty() || self.name.trim().chars().count() > 32 {
            return Err(error(
                ErrorCode::InvalidInput,
                "The channel name is invalid.",
            ));
        }
        if self.description.chars().count() > 1024 {
            return Err(error(
                ErrorCode::InvalidInput,
                "The channel description is too long.",
            ));
        }
        if self.slowmode > 21600 {
            return Err(error(
                ErrorCode::InvalidInput,
                "Slowmode cannot exceed six hours.",
            ));
        }
        Ok(
            json!({"name":self.name.trim(),"description":self.description,"slowmode":self.slowmode,"nsfw":self.nsfw}),
        )
    }
}
fn error(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, message)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}
fn require(
    manager: &ConnectionManager,
    channel: &str,
    permission: u64,
) -> Result<(), CommandError> {
    if !valid_id(channel) {
        return Err(error(ErrorCode::InvalidInput, "The channel ID is invalid."));
    }
    manager
        .require_channel_permission(channel, permission)
        .map_err(|_| {
            error(
                ErrorCode::AuthenticationFailed,
                "You do not have permission to manage this channel.",
            )
        })
}
fn context(policy: &HttpPolicy) -> Result<AuthenticatedContext, CommandError> {
    policy
        .authenticated_context()
        .map_err(|reason| match reason {
            ContextError::NotConfigured => {
                error(ErrorCode::NotConfigured, "Choose a Stoat server first.")
            }
            ContextError::ConfigurationUnavailable => error(
                ErrorCode::NotConfigured,
                "The selected server configuration is unavailable.",
            ),
            ContextError::SecureStorage => error(
                ErrorCode::SecureStorage,
                "Could not access secure session storage.",
            ),
            ContextError::AuthenticationRequired | ContextError::Cancelled => error(
                ErrorCode::AuthenticationFailed,
                "Your session expired. Log in again.",
            ),
        })
}
fn policy_error(reason: HttpPolicyError) -> CommandError {
    match reason {
        HttpPolicyError::InvalidResponse | HttpPolicyError::BodyTooLarge => error(
            ErrorCode::InvalidResponse,
            "The server returned an unreadable response.",
        ),
        _ => error(
            ErrorCode::Unavailable,
            "The channel action could not be completed. Try again.",
        ),
    }
}

fn request(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    method: Method,
    path: &[&str],
    body: Option<Value>,
) -> Result<RequestBuilder, CommandError> {
    let mut url = url::Url::parse(&context.server.endpoints.api).map_err(|_| {
        error(
            ErrorCode::NotConfigured,
            "The selected server API address is invalid.",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(error(
            ErrorCode::NotConfigured,
            "The selected server API address is invalid.",
        ));
    }
    url.path_segments_mut()
        .map_err(|_| {
            error(
                ErrorCode::NotConfigured,
                "The selected server API address is invalid.",
            )
        })?
        .pop_if_empty()
        .extend(path);
    let mut request = policy
        .client()
        .request(method, url)
        .header("x-session-token", &context.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    Ok(request)
}
async fn execute(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    request: RequestBuilder,
) -> Result<Option<Value>, CommandError> {
    let response = policy
        .execute_authenticated(
            context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(policy_error)?;
    response.consume(
        |status, _, bytes| {
            if !status.is_success() {
                return Err(status_error(status));
            }
            if bytes.is_empty() {
                Ok(None)
            } else {
                serde_json::from_slice(bytes).map(Some).map_err(|_| {
                    error(
                        ErrorCode::InvalidResponse,
                        "The server returned invalid channel data.",
                    )
                })
            }
        },
        || error(ErrorCode::Unavailable, "The channel action was cancelled."),
    )
}
fn status_error(status: StatusCode) -> CommandError {
    match status.as_u16() {
        401 => error(
            ErrorCode::AuthenticationFailed,
            "Log in again to manage this channel.",
        ),
        403 => error(
            ErrorCode::AuthenticationFailed,
            "You do not have permission to manage this channel.",
        ),
        404 => error(ErrorCode::NotFound, "The channel is no longer available."),
        429 => error(
            ErrorCode::Unavailable,
            "The server is busy. Try again shortly.",
        ),
        _ => error(
            ErrorCode::Unavailable,
            "The server rejected the channel action.",
        ),
    }
}
fn channel(value: Option<Value>, expected: &str) -> Result<ClientChannel, CommandError> {
    let channel: ClientChannel = serde_json::from_value(value.ok_or_else(|| {
        error(
            ErrorCode::InvalidResponse,
            "The server returned an empty channel response.",
        )
    })?)
    .map_err(|_| {
        error(
            ErrorCode::InvalidResponse,
            "The server returned invalid channel data.",
        )
    })?;
    if channel.id != expected {
        return Err(error(
            ErrorCode::InvalidResponse,
            "The server returned a different channel.",
        ));
    }
    Ok(channel)
}
pub async fn update(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel_id: &str,
    overview: &Overview,
) -> Result<ClientChannel, CommandError> {
    require(manager, channel_id, MANAGE_CHANNEL)?;
    let body = overview.body()?;
    let context = context(policy)?;
    channel(
        execute(
            policy,
            &context,
            request(
                policy,
                &context,
                Method::PATCH,
                &["channels", channel_id],
                Some(body),
            )?,
        )
        .await?,
        channel_id,
    )
}
pub async fn set_permissions(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel_id: &str,
    role: Option<&str>,
    permissions: &PermissionOverride,
) -> Result<ClientChannel, CommandError> {
    if role.is_some_and(|id| !valid_id(id)) {
        return Err(error(
            ErrorCode::InvalidInput,
            "The permission target is invalid.",
        ));
    }
    require(manager, channel_id, MANAGE_PERMISSIONS)?;
    let context = context(policy)?;
    let body = json!({"permissions":{"allow":permissions.a,"deny":permissions.d}});
    channel(
        execute(
            policy,
            &context,
            request(
                policy,
                &context,
                Method::PUT,
                &[
                    "channels",
                    channel_id,
                    "permissions",
                    role.unwrap_or("default"),
                ],
                Some(body),
            )?,
        )
        .await?,
        channel_id,
    )
}
pub async fn delete(
    policy: &HttpPolicy,
    manager: &ConnectionManager,
    channel_id: &str,
) -> Result<(), CommandError> {
    require(manager, channel_id, MANAGE_CHANNEL)?;
    let context = context(policy)?;
    execute(
        policy,
        &context,
        request(
            policy,
            &context,
            Method::DELETE,
            &["channels", channel_id],
            None,
        )?,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
