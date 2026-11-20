//! Member profile, relationship and moderation contracts ported from Tauri.
use crate::{
    connection::{
        ClientChannel, ClientFileReference, ClientMember, ClientServer, ClientUser,
        ConnectionManager,
    },
    error::{CommandError, ErrorCode},
    http::{HttpPolicy, RequestClass},
    messages::request::{
        authenticated_context, execute_authenticated, invalid_input, invalid_response, unavailable,
    },
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

mod policy;
pub use policy::{
    context_actions, relationship_actions, relationship_kind, Action, RelationshipKind,
};
pub const KICK_MEMBERS: u64 = 1 << 6;
pub const BAN_MEMBERS: u64 = 1 << 7;
pub const ASSIGN_ROLES: u64 = 1 << 9;
pub const CHANGE_NICKNAME: u64 = 1 << 10;
pub const MANAGE_NICKNAMES: u64 = 1 << 11;
pub const CHANGE_AVATAR: u64 = 1 << 12;

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn valid_file_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}
fn ids(server: &str, user: &str) -> Result<(), CommandError> {
    if valid_id(server) && valid_id(user) {
        Ok(())
    } else {
        Err(invalid_input("The member action contained an invalid ID."))
    }
}
fn allowed(result: Result<(), String>) -> Result<(), CommandError> {
    result.map_err(|_| {
        CommandError::new(
            ErrorCode::AuthenticationFailed,
            "You no longer have permission to perform this member action.",
        )
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberPatch {
    pub nickname: Option<String>,
    pub pronouns: Option<Option<String>>,
    pub avatar: Option<Option<String>>,
    pub roles: Option<Vec<String>>,
}
impl MemberPatch {
    pub fn body(&self) -> Result<Value, CommandError> {
        if self.nickname.is_none()
            && self.pronouns.is_none()
            && self.avatar.is_none()
            && self.roles.is_none()
        {
            return Err(invalid_input(
                "The member update did not contain any changes.",
            ));
        }
        if self
            .nickname
            .as_ref()
            .is_some_and(|v| v.chars().count() > 32)
            || self
                .pronouns
                .as_ref()
                .and_then(|v| v.as_ref())
                .is_some_and(|v| v.chars().count() > 24)
        {
            return Err(invalid_input("The nickname or pronouns are too long."));
        }
        if self
            .roles
            .as_ref()
            .is_some_and(|v| v.iter().any(|v| !valid_id(v)))
            || self
                .avatar
                .as_ref()
                .and_then(|v| v.as_ref())
                .is_some_and(|v| !valid_file_id(v))
        {
            return Err(invalid_input("The role or avatar ID is invalid."));
        }
        let mut body = serde_json::Map::new();
        let mut remove = Vec::new();
        if let Some(value) = &self.nickname {
            if value.trim().is_empty() {
                remove.push("Nickname");
            } else {
                body.insert("nickname".into(), json!(value.trim()));
            }
        }
        if let Some(value) = &self.pronouns {
            match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                Some(v) => {
                    body.insert("pronouns".into(), json!(v));
                }
                None => remove.push("Pronouns"),
            }
        }
        if let Some(value) = &self.avatar {
            match value {
                Some(v) => {
                    body.insert("avatar".into(), json!(v));
                }
                None => remove.push("Avatar"),
            }
        }
        if let Some(value) = &self.roles {
            body.insert("roles".into(), json!(value));
        }
        if !remove.is_empty() {
            body.insert("remove".into(), json!(remove));
        }
        Ok(Value::Object(body))
    }
    pub fn apply(&self, member: &mut ClientMember) {
        if let Some(value) = &self.nickname {
            member.nickname = (!value.trim().is_empty()).then(|| value.trim().into());
        }
        if let Some(value) = &self.pronouns {
            member.pronouns = value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
        }
        if let Some(value) = &self.avatar {
            member.avatar = value.as_ref().map(|id| ClientFileReference {
                id: id.clone(),
                tag: "avatars".into(),
                content_type: None,
            });
        }
        if let Some(value) = &self.roles {
            member.roles = value.clone();
        }
    }
}

async fn request(
    policy: &HttpPolicy,
    method: Method,
    path: &[&str],
    body: Option<Value>,
    ticket: Option<&str>,
) -> Result<Option<Value>, CommandError> {
    let context = authenticated_context(policy)?;
    let mut url = url::Url::parse(&context.server.endpoints.api)
        .map_err(|_| invalid_input("The server API address is invalid."))?;
    url.path_segments_mut()
        .map_err(|_| invalid_input("The server API address is invalid."))?
        .pop_if_empty()
        .extend(path);
    let mut request = policy
        .client()
        .request(method, url)
        .header("x-session-token", &context.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    if let Some(ticket) = ticket {
        request = request.header("x-mfa-ticket", ticket);
    }
    execute_authenticated(
        policy,
        &context,
        request,
        RequestClass::Standard,
        "The member request could not be completed. Try again.",
    )
    .await?
    .consume(
        |status, _, bytes| {
            if !status.is_success() {
                return Err(CommandError::new(
                    match status.as_u16() {
                        401 | 403 => ErrorCode::AuthenticationFailed,
                        404 => ErrorCode::NotFound,
                        400..=499 if status.as_u16() != 429 => ErrorCode::InvalidInput,
                        _ => ErrorCode::Unavailable,
                    },
                    "The server could not complete this member action.",
                ));
            }
            if bytes.is_empty() {
                Ok(None)
            } else {
                serde_json::from_slice(bytes).map(Some).map_err(|_| {
                    invalid_response("The server returned an invalid member response.")
                })
            }
        },
        || unavailable("The member request was cancelled."),
    )
}
fn required(value: Option<Value>) -> Result<Value, CommandError> {
    value.ok_or_else(|| invalid_response("The server returned an empty member response."))
}
fn parse<T: serde::de::DeserializeOwned>(value: Option<Value>) -> Result<T, CommandError> {
    serde_json::from_value(required(value)?)
        .map_err(|_| invalid_response("The server returned invalid member details."))
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub background: Option<ClientFileReference>,
}
pub async fn profile(policy: &HttpPolicy, user: &str) -> Result<Profile, CommandError> {
    if !valid_id(user) {
        return Err(invalid_input("The user ID is invalid."));
    }
    parse(request(policy, Method::GET, &["users", user, "profile"], None, None).await?)
}
pub async fn open_dm(policy: &HttpPolicy, user: &str) -> Result<ClientChannel, CommandError> {
    if !valid_id(user) {
        return Err(invalid_input("The user ID is invalid."));
    }
    let channel: ClientChannel =
        parse(request(policy, Method::GET, &["users", user, "dm"], None, None).await?)?;
    if channel.channel_type != "DirectMessage" || !channel.recipients.iter().any(|id| id == user) {
        return Err(invalid_response(
            "The server returned a different direct-message channel.",
        ));
    }
    Ok(channel)
}
pub async fn edit_member(
    policy: &HttpPolicy,
    authorization: &ConnectionManager,
    server: &str,
    user: &str,
    self_edit: bool,
    patch: &MemberPatch,
) -> Result<(), CommandError> {
    ids(server, user)?;
    let body = patch.body()?;
    authorize_edit(authorization, server, self_edit, patch)?;
    request(
        policy,
        Method::PATCH,
        &["servers", server, "members", user],
        Some(body),
        None,
    )
    .await?;
    Ok(())
}
/// Check again before a preceding avatar upload, then before the member PATCH.
pub fn authorize_edit(
    authorization: &ConnectionManager,
    server: &str,
    self_edit: bool,
    patch: &MemberPatch,
) -> Result<(), CommandError> {
    if patch.roles.is_some() {
        allowed(authorization.require_server_permission(server, ASSIGN_ROLES))?;
    }
    if patch.nickname.is_some() || patch.pronouns.is_some() {
        allowed(authorization.require_server_permission(
            server,
            if self_edit {
                CHANGE_NICKNAME
            } else {
                MANAGE_NICKNAMES
            },
        ))?;
    }
    if patch.avatar.is_some() {
        allowed(authorization.require_server_permission(
            server,
            if self_edit {
                CHANGE_AVATAR
            } else {
                MANAGE_NICKNAMES
            },
        ))?;
    }
    Ok(())
}
pub async fn moderate(
    policy: &HttpPolicy,
    authorization: &ConnectionManager,
    server: &str,
    user: &str,
    ban: bool,
) -> Result<(), CommandError> {
    ids(server, user)?;
    allowed(
        authorization
            .require_server_permission(server, if ban { BAN_MEMBERS } else { KICK_MEMBERS }),
    )?;
    request(
        policy,
        if ban { Method::PUT } else { Method::DELETE },
        &[
            "servers",
            server,
            if ban { "bans" } else { "members" },
            user,
        ],
        Some(json!({})),
        None,
    )
    .await?;
    Ok(())
}
/// Send a friend request from the Friends form, which has a handle rather than a user ID.
pub async fn send_friend_request(
    policy: &HttpPolicy,
    username: &str,
) -> Result<ClientUser, CommandError> {
    let username = username.trim();
    if username.len() > 128
        || username.chars().any(char::is_control)
        || !username
            .split_once('#')
            .is_some_and(|(name, discriminator)| !name.is_empty() && !discriminator.is_empty())
    {
        return Err(invalid_input(
            "Enter a username and discriminator in the form username#0000.",
        ));
    }
    parse(
        request(
            policy,
            Method::POST,
            &["users", "friend"],
            Some(json!({ "username": username })),
            None,
        )
        .await?,
    )
}

pub async fn relationship(
    policy: &HttpPolicy,
    user: &ClientUser,
    action: Action,
) -> Result<ClientUser, CommandError> {
    if !valid_id(&user.id) {
        return Err(invalid_input("The user ID is invalid."));
    }
    let (method, path, body) = match action {
        Action::Add | Action::Accept => (Method::PUT, vec!["users", &user.id, "friend"], None),
        Action::Decline | Action::Cancel | Action::Remove => {
            (Method::DELETE, vec!["users", &user.id, "friend"], None)
        }
        Action::Block => (Method::PUT, vec!["users", &user.id, "block"], None),
        Action::Unblock => (Method::DELETE, vec!["users", &user.id, "block"], None),
        _ => return Err(invalid_input("Choose a relationship action.")),
    };
    let result: ClientUser = parse(request(policy, method, &path, body, None).await?)?;
    if result.id != user.id {
        return Err(invalid_response(
            "The relationship response did not match the selected user.",
        ));
    }
    Ok(result)
}
pub fn report_body(user: &str, context: &str) -> Result<Value, CommandError> {
    if !valid_id(user) || context.chars().count() > 1000 {
        return Err(invalid_input(
            "Choose a valid user and describe the report in 1000 characters or fewer.",
        ));
    }
    Ok(
        json!({"content":{"type":"User","id":user,"report_reason":"NoneSpecified"},"additional_context":context.trim()}),
    )
}
pub async fn report(policy: &HttpPolicy, user: &str, context: &str) -> Result<(), CommandError> {
    request(
        policy,
        Method::POST,
        &["safety", "report"],
        Some(report_body(user, context)?),
        None,
    )
    .await?;
    Ok(())
}
pub async fn mfa_methods(policy: &HttpPolicy) -> Result<Vec<String>, CommandError> {
    let mfa = required(request(policy, Method::GET, &["auth", "mfa"], None, None).await?)?;
    let flag = |key| match mfa.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid_response("The account returned an invalid MFA status.")),
    };
    if !mfa.is_object() {
        return Err(invalid_response(
            "The account returned an invalid MFA status.",
        ));
    }
    Ok(if flag("totp_mfa")? {
        let mut methods = vec!["totp".into()];
        if flag("recovery_active")? {
            methods.push("recovery".into());
        }
        methods
    } else {
        vec!["password".into()]
    })
}
pub fn mfa_body(method: &str, value: &str) -> Result<Value, CommandError> {
    if value.trim().is_empty() {
        return Err(invalid_input("Enter your MFA verification value."));
    }
    match method {
        "password" => Ok(json!({"password":value.trim()})),
        "totp" => Ok(json!({"totp_code":value.trim().replace(' ',"")})),
        "recovery" => Ok(json!({"recovery_code":value.trim()})),
        _ => Err(invalid_input("Choose a valid MFA method.")),
    }
}
pub async fn transfer(
    policy: &HttpPolicy,
    authorization: &ConnectionManager,
    server: &str,
    user: &str,
    method: &str,
    value: &str,
) -> Result<ClientServer, CommandError> {
    ids(server, user)?;
    allowed(authorization.require_server_owner(server))?;
    let ticket = mfa_ticket(policy, method, value).await?;
    transfer_with_ticket(policy, authorization, server, user, &ticket).await
}
pub async fn mfa_ticket(
    policy: &HttpPolicy,
    method: &str,
    value: &str,
) -> Result<String, CommandError> {
    let ticket = required(
        request(
            policy,
            Method::PUT,
            &["auth", "mfa", "ticket"],
            Some(mfa_body(method, value)?),
            None,
        )
        .await?,
    )?;
    ticket
        .get("token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 1024)
        .map(str::to_owned)
        .ok_or_else(|| invalid_response("The server did not return a valid MFA ticket."))
}
pub async fn transfer_with_ticket(
    policy: &HttpPolicy,
    authorization: &ConnectionManager,
    server: &str,
    user: &str,
    ticket: &str,
) -> Result<ClientServer, CommandError> {
    ids(server, user)?;
    if ticket.is_empty() || ticket.len() > 1024 {
        return Err(invalid_input("The MFA ticket is invalid."));
    }
    allowed(authorization.require_server_owner(server))?;
    let result: ClientServer = parse(
        request(
            policy,
            Method::PATCH,
            &["servers", server],
            Some(json!({"owner":user})),
            Some(ticket),
        )
        .await?,
    )?;
    if result.id != server || result.owner != user {
        return Err(invalid_response(
            "The ownership response did not match the selected server and user.",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
