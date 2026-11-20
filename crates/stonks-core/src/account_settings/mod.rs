//! Account settings requests bound to one authenticated HTTP context.
use crate::{
    connection::{ClientFileReference, ClientUser},
    error::{CommandError, ErrorCode},
    http::{AuthenticatedContext, HttpPolicy, RequestClass, MAX_JSON_RESPONSE_BYTES},
};
use reqwest::Method;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub struct AccountSettingsService {
    policy: Arc<HttpPolicy>,
    context: AuthenticatedContext,
    current_session: Option<String>,
}

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidResponse,
        "The server returned invalid account settings data.",
    )
}
fn input(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, message)
}
fn parse<T: DeserializeOwned>(value: Value) -> Result<T, CommandError> {
    serde_json::from_value(value).map_err(|_| invalid())
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[derive(Clone, Deserialize)]
pub struct Profile {
    pub content: Option<String>,
    pub background: Option<ClientFileReference>,
}

pub const PRESENCES: [&str; 5] = ["Online", "Idle", "Focus", "Busy", "Invisible"];
#[derive(Clone, PartialEq, Eq)]
pub struct ProfileDraft {
    pub display_name: String,
    pub pronouns: String,
    pub status: String,
    pub presence: String,
    pub about: String,
    pub remove_background: bool,
}
impl ProfileDraft {
    pub fn from_user(user: &ClientUser, profile: &Profile) -> Self {
        Self {
            display_name: user.display_name.clone().unwrap_or_default(),
            pronouns: user.pronouns.clone().unwrap_or_default(),
            status: user
                .status
                .as_ref()
                .and_then(|s| s.text.clone())
                .unwrap_or_default(),
            presence: user
                .status
                .as_ref()
                .and_then(|s| s.presence.clone())
                .filter(|s| PRESENCES.contains(&s.as_str()))
                .unwrap_or_else(|| "Online".into()),
            about: profile.content.clone().unwrap_or_default(),
            remove_background: false,
        }
    }
    fn body(&self) -> Result<Value, CommandError> {
        if !PRESENCES.contains(&self.presence.as_str()) {
            return Err(input("Choose a valid presence."));
        }
        let optional = |s: &str| {
            if s.trim().is_empty() {
                Value::Null
            } else {
                json!(s.trim())
            }
        };
        let mut body = json!({"display_name":optional(&self.display_name),"pronouns":optional(&self.pronouns),"status":{"text":optional(&self.status),"presence":self.presence},"profile":{"content":optional(&self.about)}});
        // The backend interprets null as no change, not removal.
        let mut remove = Vec::new();
        for (value, field) in [
            (&self.display_name, "DisplayName"),
            (&self.pronouns, "Pronouns"),
            (&self.status, "StatusText"),
            (&self.about, "ProfileContent"),
        ] {
            if value.trim().is_empty() {
                remove.push(field);
            }
        }
        if self.remove_background {
            remove.push("ProfileBackground");
        }
        if !remove.is_empty() {
            body["remove"] = json!(remove);
        }
        Ok(body)
    }
}

#[derive(Clone, Copy)]
pub enum AccountManagement {
    Disable,
    Delete,
}
impl AccountManagement {
    fn endpoint(self, confirmation: &str) -> Result<&'static str, CommandError> {
        match self {
            Self::Disable => Ok("disable"),
            Self::Delete if confirmation == "DELETE" => Ok("delete"),
            Self::Delete => Err(input("Enter DELETE to confirm.")),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct AccountInfo {
    pub email: String,
    pub authenticator_enabled: bool,
    pub recovery_enabled: bool,
}
impl AccountInfo {
    pub fn methods(&self) -> &'static [&'static str] {
        match (self.authenticator_enabled, self.recovery_enabled) {
            (true, true) => &["totp", "recovery"],
            (true, false) => &["totp"],
            _ => &["password"],
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct Session {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub last_seen: Option<String>,
    #[serde(skip)]
    pub current: Option<bool>,
}
fn sessions(value: Value, current: Option<&str>) -> Result<Vec<Session>, CommandError> {
    let mut sessions: Vec<Session> = parse(value)?;
    if sessions
        .iter()
        .any(|s| !id(&s.id) || s.name.trim().is_empty())
    {
        return Err(invalid());
    }
    let current = current.filter(|current| sessions.iter().any(|s| &s.id == current));
    for s in &mut sessions {
        s.current = current.map(|current| s.id == current);
    }
    Ok(sessions)
}

impl AccountSettingsService {
    /// Call off the UI thread: acquiring the context may read secure storage.
    /// `current_session` must come from the host's authenticated session, never the server list.
    pub fn connect(
        policy: Arc<HttpPolicy>,
        current_session: Option<String>,
    ) -> Result<Self, CommandError> {
        let context = policy.authenticated_context().map_err(|_| {
            CommandError::new(
                ErrorCode::AuthenticationFailed,
                "Log in again to access account settings.",
            )
        })?;
        Ok(Self {
            policy,
            context,
            current_session,
        })
    }
    /// Bind to the same credential and epoch as the host's gateway session.
    pub fn for_context(policy: Arc<HttpPolicy>, context: AuthenticatedContext) -> Self {
        let current_session =
            crate::auth::credentials::session_id_for_token(&context.server, &context.token);
        Self {
            policy,
            context,
            current_session,
        }
    }

    pub fn is_current(&self) -> bool {
        self.policy.is_authenticated_context_current(&self.context)
    }

    async fn request(
        &self,
        method: Method,
        path: &[&str],
        body: Option<Value>,
        ticket: Option<&str>,
        others: bool,
    ) -> Result<Value, CommandError> {
        let mut url = Url::parse(&self.context.server.endpoints.api).map_err(|_| invalid())?;
        url.path_segments_mut()
            .map_err(|_| invalid())?
            .pop_if_empty()
            .extend(path);
        if others {
            url.query_pairs_mut().append_pair("revoke_self", "false");
        }
        let mut request = self
            .policy
            .client()
            .request(method, url)
            .header("x-session-token", &self.context.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(ticket) = ticket {
            request = request.header("x-mfa-ticket", ticket);
        }
        let unavailable = || {
            CommandError::new(
                ErrorCode::Unavailable,
                "The account settings request could not be completed.",
            )
        };
        self.policy
            .execute_authenticated(
                &self.context,
                request,
                RequestClass::Standard,
                MAX_JSON_RESPONSE_BYTES,
            )
            .await
            .map_err(|_| unavailable())?
            .consume(
                |status, _, bytes| {
                    if !status.is_success() {
                        let code = match status.as_u16() {
                            401 | 403 => ErrorCode::AuthenticationFailed,
                            404 => ErrorCode::NotFound,
                            400 | 422 => ErrorCode::InvalidInput,
                            _ => ErrorCode::Unavailable,
                        };
                        return Err(CommandError::new(
                            code,
                            "The server rejected the account settings request.",
                        ));
                    }
                    if bytes.is_empty() {
                        Ok(Value::Null)
                    } else {
                        serde_json::from_slice(bytes).map_err(|_| invalid())
                    }
                },
                unavailable,
            )
    }
    async fn get<T: DeserializeOwned>(&self, path: &[&str]) -> Result<T, CommandError> {
        parse(self.request(Method::GET, path, None, None, false).await?)
    }
    pub async fn me(&self) -> Result<ClientUser, CommandError> {
        self.get(&["users", "@me"]).await
    }
    pub async fn profile(&self, user: &str) -> Result<Profile, CommandError> {
        if !id(user) {
            return Err(input("The user ID is invalid."));
        }
        self.get(&["users", user, "profile"]).await
    }
    pub async fn profile_image(&self, file: &ClientFileReference) -> Option<Vec<u8>> {
        let url = crate::media::file_url(
            self.context.server.endpoints.autumn.as_deref(),
            &file.tag,
            &file.id,
        )?;
        crate::media::fetch_image(&self.policy, &url).await.ok()
    }

    async fn upload_profile_image(
        &self,
        tag: &str,
        file: &crate::uploads::UploadFile,
    ) -> Result<String, CommandError> {
        use reqwest::multipart::{Form, Part};
        file.validate(8_000_000)?;
        let server = &self.context.server;
        crate::server::validate_selection(server)?;
        let mut url = Url::parse(
            server
                .endpoints
                .autumn
                .as_deref()
                .ok_or_else(|| input("This server does not advertise a file service."))?,
        )
        .map_err(|_| invalid())?;
        url.path_segments_mut()
            .map_err(|_| invalid())?
            .pop_if_empty()
            .push(tag);
        let part = Part::bytes(file.bytes.clone())
            .file_name(file.filename.clone())
            .mime_str(&file.content_type)
            .map_err(|_| input("The image type is invalid."))?;
        let request = self
            .policy
            .client()
            .post(url)
            .header("x-session-token", &self.context.token)
            .multipart(Form::new().part("file", part));
        let failed = || {
            CommandError::new(
                ErrorCode::Unavailable,
                "The profile image could not be uploaded.",
            )
        };
        self.policy
            .execute_authenticated(
                &self.context,
                request,
                RequestClass::Transfer,
                MAX_JSON_RESPONSE_BYTES,
            )
            .await
            .map_err(|_| failed())?
            .consume(
                |status, _, bytes| {
                    if !status.is_success() {
                        return Err(failed());
                    }
                    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
                    value
                        .as_str()
                        .or_else(|| value["id"].as_str())
                        .or_else(|| value["_id"].as_str())
                        .or_else(|| value["file"]["id"].as_str())
                        .or_else(|| value["file"]["_id"].as_str())
                        .filter(|id| !id.trim().is_empty())
                        .map(str::to_owned)
                        .ok_or_else(invalid)
                },
                failed,
            )
    }

    pub async fn save_profile(&self, draft: &ProfileDraft) -> Result<(), CommandError> {
        self.save_profile_with_images(draft, None, None).await
    }

    pub async fn save_profile_with_images(
        &self,
        draft: &ProfileDraft,
        avatar: Option<&crate::uploads::UploadFile>,
        background: Option<&crate::uploads::UploadFile>,
    ) -> Result<(), CommandError> {
        let mut body = draft.body()?;
        if background.is_some() && draft.remove_background {
            return Err(input("Choose whether to replace or remove the background."));
        }
        for file in [avatar, background].into_iter().flatten() {
            file.validate(8_000_000)?;
        }
        if let Some(file) = avatar {
            body["avatar"] = json!(self.upload_profile_image("avatars", file).await?);
        }
        if let Some(file) = background {
            body["profile"]["background"] =
                json!(self.upload_profile_image("backgrounds", file).await?);
        }
        self.request(Method::PATCH, &["users", "@me"], Some(body), None, false)
            .await?;
        Ok(())
    }
    pub async fn account(&self) -> Result<AccountInfo, CommandError> {
        let account: Value = self.get(&["auth", "account"]).await?;
        let mfa: Value = self.get(&["auth", "mfa"]).await?;
        let email = account["email"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(invalid)?
            .to_owned();
        if !mfa.is_object() {
            return Err(invalid());
        }
        let flag = |name| match mfa.get(name) {
            None | Some(Value::Null) => Ok(false),
            Some(v) => v.as_bool().ok_or_else(invalid),
        };
        Ok(AccountInfo {
            email,
            authenticator_enabled: flag("totp_mfa")?,
            recovery_enabled: flag("recovery_active")?,
        })
    }
    async fn ticket(&self, method: &str, value: &str) -> Result<String, CommandError> {
        let body = crate::member_actions::mfa_body(method, value)?;
        let result = self
            .request(
                Method::PUT,
                &["auth", "mfa", "ticket"],
                Some(body),
                None,
                false,
            )
            .await?;
        result["token"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
    pub async fn change_username(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(), CommandError> {
        change_values(username, password)?;
        self.request(
            Method::PATCH,
            &["users", "@me", "username"],
            Some(json!({"username":username.trim(),"password":password})),
            None,
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn change_password(
        &self,
        password: &str,
        new_password: &str,
    ) -> Result<(), CommandError> {
        change_values(new_password, password)?;
        self.request(
            Method::PATCH,
            &["auth", "account", "change", "password"],
            Some(json!({"password":new_password,"current_password":password})),
            None,
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn change_email(
        &self,
        email: &str,
        password: &str,
        verification: Option<(&str, &str)>,
    ) -> Result<(), CommandError> {
        change_values(email, password)?;
        let ticket = match verification {
            Some((method, value)) => Some(self.ticket(method, value).await?),
            None => None,
        };
        self.request(
            Method::PATCH,
            &["auth", "account", "change", "email"],
            Some(json!({"email":email.trim(),"current_password":password})),
            ticket.as_deref(),
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn sessions(&self) -> Result<Vec<Session>, CommandError> {
        sessions(
            self.get(&["auth", "session", "all"]).await?,
            self.current_session.as_deref(),
        )
    }
    pub async fn rename_session(&self, session: &str, name: &str) -> Result<(), CommandError> {
        if !id(session) || session == "all" || name.trim().is_empty() {
            return Err(input("Enter a valid session name."));
        }
        self.request(
            Method::PATCH,
            &["auth", "session", session],
            Some(json!({"friendly_name":name.trim()})),
            None,
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn revoke_session(&self, session: &str) -> Result<(), CommandError> {
        if !id(session)
            || session == "all"
            || !matches!(self.current_session.as_deref(), Some(current) if current != session)
        {
            return Err(input("Only an identified other session can be revoked."));
        }
        self.request(
            Method::DELETE,
            &["auth", "session", session],
            None,
            None,
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn manage_account(
        &self,
        action: AccountManagement,
        confirmation: &str,
        method: &str,
        value: &str,
    ) -> Result<(), CommandError> {
        let endpoint = action.endpoint(confirmation)?;
        let ticket = self.ticket(method, value).await?;
        self.request(
            Method::POST,
            &["auth", "account", endpoint],
            None,
            Some(&ticket),
            false,
        )
        .await?;
        Ok(())
    }

    pub async fn revoke_other_sessions(
        &self,
        method: &str,
        value: &str,
    ) -> Result<(), CommandError> {
        let ticket = self.ticket(method, value).await?;
        self.request(
            Method::DELETE,
            &["auth", "session", "all"],
            None,
            Some(&ticket),
            true,
        )
        .await?;
        Ok(())
    }
    pub async fn blocked_users(&self) -> Result<Vec<ClientUser>, CommandError> {
        let me: Value = self.get(&["users", "@me"]).await?;
        let mut users = Vec::new();
        for user_id in blocked_ids(&me)? {
            let mut user: ClientUser = self.get(&["users", user_id]).await?;
            if user.id != user_id {
                return Err(invalid());
            }
            user.relationship = Some("Blocked".into());
            users.push(user);
        }
        Ok(users)
    }
    pub async fn unblock(&self, user_id: &str) -> Result<ClientUser, CommandError> {
        if !id(user_id) {
            return Err(input("The user ID is invalid."));
        }
        let user: ClientUser = parse(
            self.request(
                Method::DELETE,
                &["users", user_id, "block"],
                None,
                None,
                false,
            )
            .await?,
        )?;
        if user.id != user_id {
            return Err(invalid());
        }
        Ok(user)
    }
}
fn change_values(value: &str, password: &str) -> Result<(), CommandError> {
    if value.trim().is_empty() || password.trim().is_empty() {
        return Err(input("Enter the new value and your current password."));
    }
    Ok(())
}
fn blocked_ids(me: &Value) -> Result<Vec<&str>, CommandError> {
    if !me.is_object() {
        return Err(invalid());
    }
    let mut ids = Vec::new();
    if let Some(relations) = me.get("relations").filter(|v| !v.is_null()) {
        for relation in relations.as_array().ok_or_else(invalid)? {
            if relation["status"].as_str().ok_or_else(invalid)? == "Blocked" {
                let user = relation["_id"]
                    .as_str()
                    .filter(|s| id(s))
                    .ok_or_else(invalid)?;
                if !ids.contains(&user) {
                    ids.push(user);
                }
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_identity_is_not_taken_from_response() {
        let value =
            json!([{"_id":"one","name":"Desktop","current":true},{"_id":"two","name":"Other"}]);
        assert_eq!(
            sessions(value.clone(), Some("two"))
                .unwrap()
                .iter()
                .map(|s| s.current)
                .collect::<Vec<_>>(),
            [Some(false), Some(true)]
        );
        assert!(sessions(value, Some("missing"))
            .unwrap()
            .iter()
            .all(|s| s.current.is_none()));
    }
    #[test]
    fn blocked_relations_are_deduplicated_and_validated() {
        assert_eq!(blocked_ids(&json!({"relations":[{"_id":"A","status":"Blocked"},{"_id":"A","status":"Blocked"},{"_id":"B","status":"Friend"}]})).unwrap(), ["A"]);
        assert!(blocked_ids(&json!({"relations":[{"_id":"../A","status":"Blocked"}]})).is_err());
    }
    #[test]
    fn profile_clears_blank_text_and_only_explicitly_removes_background() {
        let mut draft = ProfileDraft {
            display_name: " ".into(),
            pronouns: " they ".into(),
            status: "".into(),
            presence: "Focus".into(),
            about: "".into(),
            remove_background: false,
        };
        let body = draft.body().unwrap();
        assert!(body["display_name"].is_null());
        assert_eq!(body["pronouns"], "they");
        assert_eq!(
            body["remove"],
            json!(["DisplayName", "StatusText", "ProfileContent"])
        );
        draft.remove_background = true;
        assert_eq!(
            draft.body().unwrap()["remove"],
            json!([
                "DisplayName",
                "StatusText",
                "ProfileContent",
                "ProfileBackground"
            ])
        );
    }
}

#[cfg(test)]
#[test]
fn deletion_requires_exact_confirmation() {
    assert_eq!(
        AccountManagement::Delete.endpoint("DELETE").unwrap(),
        "delete"
    );
    for value in ["", "delete", " DELETE", "DELETE "] {
        assert!(AccountManagement::Delete.endpoint(value).is_err());
    }
    assert_eq!(AccountManagement::Disable.endpoint("").unwrap(), "disable");
}
