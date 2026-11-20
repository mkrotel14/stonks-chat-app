//! Server settings mutations, with validated targets and canonical snapshots.
use super::*;
use crate::connection::{ClientRole, PermissionOverride};

fn input(message: &'static str) -> CommandError {
    error(ErrorCode::InvalidInput, message)
}
fn ids(server: &str, target: Option<&str>) -> Result<(), CommandError> {
    if !valid_id(server) || target.is_some_and(|id| !valid_id(id)) {
        return Err(input("The settings target is invalid."));
    }
    Ok(())
}
fn name(value: &str) -> Result<&str, CommandError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 32 {
        return Err(input("Enter a name between 1 and 32 characters."));
    }
    Ok(value)
}
fn file_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub async fn refresh_server(
    policy: &HttpPolicy,
    server_id: &str,
) -> Result<ClientServer, CommandError> {
    ids(server_id, None)?;
    canonical(policy, server_id, None).await
}
async fn canonical(
    policy: &HttpPolicy,
    server_id: &str,
    value: Option<Value>,
) -> Result<ClientServer, CommandError> {
    if let Some(value) = value {
        if let Ok(server) = serde_json::from_value::<ClientServer>(value) {
            if server.id != server_id {
                return Err(invalid());
            }
            return Ok(server);
        }
    }
    let value = request(policy, Method::GET, &["servers", server_id], None)
        .await?
        .ok_or_else(invalid)?;
    let server: ClientServer = serde_json::from_value(value).map_err(|_| invalid())?;
    if server.id != server_id {
        return Err(invalid());
    }
    Ok(server)
}
pub async fn set_icon(
    policy: &HttpPolicy,
    server_id: &str,
    icon: &str,
) -> Result<ClientServer, CommandError> {
    ids(server_id, None)?;
    if !file_id(icon) {
        return Err(input("The icon is invalid."));
    }
    let value = request(
        policy,
        Method::PATCH,
        &["servers", server_id],
        Some(json!({"icon":icon})),
    )
    .await?;
    canonical(policy, server_id, value).await
}

#[derive(Clone, Debug)]
pub struct RolePatch {
    pub name: String,
    pub colour: Option<String>,
    pub hoist: bool,
    pub icon: Option<String>,
}
impl RolePatch {
    pub fn body(&self) -> Result<Value, CommandError> {
        let name = name(&self.name)?;
        let mut value = json!({"name":name,"hoist":self.hoist});
        if let Some(colour) = &self.colour {
            if colour.len() != 7
                || !colour.starts_with('#')
                || !colour[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(input("Choose a valid role colour."));
            }
            value["colour"] = json!(colour);
        }
        if let Some(icon) = &self.icon {
            if !file_id(icon) {
                return Err(input("The icon is invalid."));
            }
            value["icon"] = json!(icon);
        }
        Ok(value)
    }
}
fn project_role(value: Value) -> Result<ClientRole, CommandError> {
    let mut role = value.get("role").cloned().unwrap_or(value);
    if role.get("colour").is_none() {
        if let Some(colour) = role.get("color").cloned() {
            role["colour"] = colour;
        }
    }
    if let Some(permissions) = role.get_mut("permissions") {
        if permissions.get("a").is_none() {
            permissions["a"] = permissions.get("allow").cloned().unwrap_or(json!(0));
        }
        if permissions.get("d").is_none() {
            permissions["d"] = permissions.get("deny").cloned().unwrap_or(json!(0));
        }
    }
    serde_json::from_value(role).map_err(|_| invalid())
}
pub async fn create_role(
    policy: &HttpPolicy,
    server_id: &str,
    role_name: &str,
) -> Result<(String, ClientRole), CommandError> {
    ids(server_id, None)?;
    let role_name = name(role_name)?;
    let value = request(
        policy,
        Method::POST,
        &["servers", server_id, "roles"],
        Some(json!({"name":role_name})),
    )
    .await?
    .ok_or_else(invalid)?;
    let id = ["id", "_id", "role_id", "roleId"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|id| valid_id(id))
        .ok_or_else(invalid)?
        .to_owned();
    let role = project_role(value.get("role").cloned().ok_or_else(invalid)?)?;
    Ok((id, role))
}
pub async fn edit_role(
    policy: &HttpPolicy,
    server_id: &str,
    role_id: &str,
    patch: &RolePatch,
) -> Result<ClientRole, CommandError> {
    ids(server_id, Some(role_id))?;
    let body = patch.body()?;
    project_role(
        request(
            policy,
            Method::PATCH,
            &["servers", server_id, "roles", role_id],
            Some(body),
        )
        .await?
        .ok_or_else(invalid)?,
    )
}
pub async fn delete_role(
    policy: &HttpPolicy,
    server_id: &str,
    role_id: &str,
) -> Result<(), CommandError> {
    ids(server_id, Some(role_id))?;
    request(
        policy,
        Method::DELETE,
        &["servers", server_id, "roles", role_id],
        None,
    )
    .await?;
    Ok(())
}
pub async fn set_permissions(
    policy: &HttpPolicy,
    server_id: &str,
    role_id: Option<&str>,
    permissions: &PermissionOverride,
) -> Result<ClientServer, CommandError> {
    ids(server_id, role_id)?;
    if permissions.a & permissions.d != 0 {
        return Err(input("A permission cannot be both allowed and denied."));
    }
    if role_id.is_none() && permissions.d != 0 {
        return Err(input("Default permissions cannot contain overrides."));
    }
    let body = if role_id.is_some() {
        json!({"permissions":{"allow":permissions.a,"deny":permissions.d}})
    } else {
        json!({"permissions":permissions.a})
    };
    let value = request(
        policy,
        Method::PUT,
        &[
            "servers",
            server_id,
            "permissions",
            role_id.unwrap_or("default"),
        ],
        Some(body),
    )
    .await?;
    canonical(policy, server_id, value).await
}

pub async fn register_emoji(
    policy: &HttpPolicy,
    server_id: &str,
    image_id: &str,
    emoji_name: &str,
) -> Result<ServerEmoji, CommandError> {
    ids(server_id, Some(image_id))?;
    let emoji_name = name(emoji_name)?;
    let value = request(
        policy,
        Method::PUT,
        &["custom", "emoji", image_id],
        Some(json!({"parent":{"type":"Server","id":server_id},"name":emoji_name})),
    )
    .await?
    .ok_or_else(invalid)?;
    let emoji = project_emoji(value)?;
    if emoji.id != image_id {
        return Err(invalid());
    }
    Ok(emoji)
}
fn project_emoji(value: Value) -> Result<ServerEmoji, CommandError> {
    serde_json::from_value(value.get("emoji").cloned().unwrap_or(value)).map_err(|_| invalid())
}
async fn own_emoji(
    policy: &HttpPolicy,
    server_id: &str,
    emoji_id: &str,
) -> Result<(), CommandError> {
    ids(server_id, Some(emoji_id))?;
    let value = request(policy, Method::GET, &["custom", "emoji", emoji_id], None)
        .await?
        .ok_or_else(invalid)?;
    if value.pointer("/parent/type").and_then(Value::as_str) != Some("Server")
        || value.pointer("/parent/id").and_then(Value::as_str) != Some(server_id)
    {
        return Err(input("The emoji does not belong to this server."));
    }
    Ok(())
}
pub async fn rename_emoji(
    policy: &HttpPolicy,
    server_id: &str,
    emoji_id: &str,
    emoji_name: &str,
) -> Result<ServerEmoji, CommandError> {
    let emoji_name = name(emoji_name)?;
    own_emoji(policy, server_id, emoji_id).await?;
    let emoji = project_emoji(
        request(
            policy,
            Method::PATCH,
            &["custom", "emoji", emoji_id],
            Some(json!({"name":emoji_name})),
        )
        .await?
        .ok_or_else(invalid)?,
    )?;
    if emoji.id != emoji_id {
        return Err(invalid());
    }
    Ok(emoji)
}
pub async fn delete_emoji(
    policy: &HttpPolicy,
    server_id: &str,
    emoji_id: &str,
) -> Result<(), CommandError> {
    own_emoji(policy, server_id, emoji_id).await?;
    request(policy, Method::DELETE, &["custom", "emoji", emoji_id], None).await?;
    Ok(())
}

#[derive(Clone, Default, Debug)]
pub struct InviteOptions {
    pub max_uses: Option<u64>,
    pub expires: Option<String>,
}
impl InviteOptions {
    pub fn body(&self, now: u64) -> Result<Value, CommandError> {
        if self.max_uses.is_some_and(|uses| !(1..=100).contains(&uses)) {
            return Err(input("Invite use limits must be between 1 and 100."));
        }
        let mut body = json!({});
        if let Some(uses) = self.max_uses {
            body["max_uses"] = json!(uses);
        }
        if let Some(expires) = &self.expires {
            if expires.len() > 64
                || crate::connection::parse_rfc3339(expires).is_none_or(|(seconds, _)| {
                    seconds <= now as i64 || seconds - now as i64 > 366 * 86_400
                })
            {
                return Err(input(
                    "Choose a valid future invite expiration within one year.",
                ));
            }
            body["expires"] = json!(expires);
        }
        Ok(body)
    }
}
pub async fn create_invite(
    policy: &HttpPolicy,
    server_id: &str,
    channel_id: &str,
    options: &InviteOptions,
) -> Result<ServerInvite, CommandError> {
    ids(server_id, Some(channel_id))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| input("The system clock is unavailable."))?
        .as_secs();
    let body = options.body(now)?;
    let invite: ServerInvite = serde_json::from_value(
        request(
            policy,
            Method::POST,
            &["channels", channel_id, "invites"],
            Some(body),
        )
        .await?
        .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    if invite.server_id != server_id || invite.channel_id != channel_id {
        return Err(invalid());
    }
    Ok(invite)
}
pub async fn revoke_invite(policy: &HttpPolicy, invite_code: &str) -> Result<(), CommandError> {
    if !code(invite_code) {
        return Err(input("The invite code is invalid."));
    }
    request(policy, Method::DELETE, &["invites", invite_code], None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_mutations_before_network() {
        assert!(RolePatch {
            name: "role".into(),
            colour: Some("#abcdef".into()),
            hoist: false,
            icon: None
        }
        .body()
        .is_ok());
        assert!(RolePatch {
            name: "role".into(),
            colour: Some("red".into()),
            hoist: false,
            icon: None
        }
        .body()
        .is_err());
        assert!(RolePatch {
            name: " ".into(),
            colour: Some("#abcdef".into()),
            hoist: false,
            icon: None
        }
        .body()
        .is_err());
        assert!(InviteOptions {
            max_uses: Some(0),
            expires: None
        }
        .body(1_700_000_000)
        .is_err());
        assert!(InviteOptions {
            max_uses: None,
            expires: Some("2099-01-01T00:00:00Z".into())
        }
        .body(1_700_000_000)
        .is_err());
        assert_eq!(InviteOptions::default().body(0).unwrap(), json!({}));
        assert!(!code("../unsafe"));
        assert!(!file_id("bad\nfile"));
    }
    #[test]
    fn metadata_patch_can_preserve_existing_custom_colour() {
        let body = RolePatch {
            name: "Renamed".into(),
            colour: None,
            hoist: true,
            icon: None,
        }
        .body()
        .unwrap();
        assert!(body.get("colour").is_none());
        assert!(body.get("icon").is_none());
    }
    #[test]
    fn role_projection_preserves_wire_permission_masks() {
        let role=project_role(json!({"role":{"name":"test","color":"#abcdef","permissions":{"allow":1u64<<41,"deny":1u64<<38}}})).unwrap();
        assert_eq!(role.permissions.a, 1u64 << 41);
        assert_eq!(role.permissions.d, 1u64 << 38);
        assert_eq!(role.colour.as_deref(), Some("#abcdef"));
    }
    async fn mock(
        responses: Vec<Value>,
    ) -> (HttpPolicy, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/api/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = vec![];
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                loop {
                    header.push(stream.read_u8().await.unwrap());
                    if header.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let header = String::from_utf8(header).unwrap();
                assert!(header
                    .to_ascii_lowercase()
                    .contains("x-session-token: test-session"));
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                let body = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                requests.push((header.lines().next().unwrap().to_owned(), body));
                let body = response.to_string();
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            requests
        });
        (super::super::tests::policy(api), task)
    }
    #[tokio::test]
    async fn role_endpoints_preserve_names_icons_and_permission_high_bits() {
        let role = json!({"name":"Test","permissions":{"a":0,"d":0}});
        let (policy, server) = mock(vec![
            json!({"id":"role","role":role}),
            json!({"role":role}),
            json!({"_id":"server","name":"Test","owner":"owner"}),
            Value::Null,
        ])
        .await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            assert_eq!(
                create_role(&policy, "server", " Test ").await.unwrap().0,
                "role"
            );
            edit_role(
                &policy,
                "server",
                "role",
                &RolePatch {
                    name: " Renamed ".into(),
                    colour: Some("#abcdef".into()),
                    hoist: true,
                    icon: Some("file-id".into()),
                },
            )
            .await
            .unwrap();
            set_permissions(
                &policy,
                "server",
                Some("role"),
                &PermissionOverride {
                    a: 1 << 41,
                    d: 1 << 38,
                },
            )
            .await
            .unwrap();
            delete_role(&policy, "server", "role").await.unwrap();
        })
        .await;
        assert!(result.is_ok());
        let requests = server.await.unwrap();
        assert!(requests[0].0.starts_with("POST /api/servers/server/roles "));
        assert_eq!(requests[0].1["name"], "Test");
        assert_eq!(requests[1].1["icon"], "file-id");
        assert_eq!(requests[1].1["hoist"], true);
        assert_eq!(requests[1].1["name"], "Renamed");
        assert_eq!(
            requests[2].1,
            json!({"permissions":{"allow":1u64<<41,"deny":1u64<<38}})
        );
        assert!(requests[3]
            .0
            .starts_with("DELETE /api/servers/server/roles/role "));
    }
    #[tokio::test]
    async fn emoji_mutations_check_server_ownership_before_writing() {
        let (policy, server) = mock(vec![
            json!({"_id":"emoji","name":"wave","parent":{"type":"Server","id":"other"}}),
        ])
        .await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            rename_emoji(&policy, "server", "emoji", "new"),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].0.starts_with("GET /api/custom/emoji/emoji "));
    }
    #[tokio::test]
    async fn invite_creation_and_revocation_use_source_routes_and_bounds() {
        let invite =
            json!({"code":"safe-code","server":"server","channel":"channel","creator":"creator"});
        let (policy, server) = mock(vec![invite, Value::Null]).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let invite = create_invite(
                &policy,
                "server",
                "channel",
                &InviteOptions {
                    max_uses: Some(5),
                    expires: None,
                },
            )
            .await
            .unwrap();
            revoke_invite(&policy, &invite.code).await.unwrap();
        })
        .await;
        assert!(result.is_ok());
        let requests = server.await.unwrap();
        assert_eq!(requests[0].1, json!({"max_uses":5}));
        assert!(requests[0]
            .0
            .starts_with("POST /api/channels/channel/invites "));
        assert!(requests[1].0.starts_with("DELETE /api/invites/safe-code "));
    }
    #[tokio::test]
    async fn icon_and_default_permissions_publish_canonical_server() {
        let canonical = json!({"_id":"server","name":"Test","owner":"owner"});
        let (policy, server) = mock(vec![canonical.clone(), Value::Null, canonical]).await;
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            set_permissions(
                &policy,
                "server",
                None,
                &PermissionOverride { a: 1 << 41, d: 0 },
            )
            .await
            .unwrap();
            set_icon(&policy, "server", "file-id").await.unwrap();
        })
        .await
        .unwrap();
        let requests = server.await.unwrap();
        assert!(requests[0]
            .0
            .starts_with("PUT /api/servers/server/permissions/default "));
        assert_eq!(requests[0].1, json!({"permissions":1u64<<41}));
        assert_eq!(requests[1].1, json!({"icon":"file-id"}));
        assert!(requests[2].0.starts_with("GET /api/servers/server "));
    }
    #[tokio::test]
    async fn emoji_create_rename_delete_use_parent_checked_resource_routes() {
        let emoji = json!({"_id":"emoji","name":"wave","parent":{"type":"Server","id":"server"}});
        let renamed =
            json!({"_id":"emoji","name":"renamed","parent":{"type":"Server","id":"server"}});
        let (policy, server) = mock(vec![
            emoji.clone(),
            emoji,
            renamed.clone(),
            renamed,
            Value::Null,
        ])
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            register_emoji(&policy, "server", "emoji", " wave ")
                .await
                .unwrap();
            rename_emoji(&policy, "server", "emoji", " renamed ")
                .await
                .unwrap();
            delete_emoji(&policy, "server", "emoji").await.unwrap();
        })
        .await
        .unwrap();
        let requests = server.await.unwrap();
        assert!(requests[0].0.starts_with("PUT /api/custom/emoji/emoji "));
        assert_eq!(
            requests[0].1,
            json!({"parent":{"type":"Server","id":"server"},"name":"wave"})
        );
        assert!(requests[1].0.starts_with("GET "));
        assert_eq!(requests[2].1, json!({"name":"renamed"}));
        assert!(requests[3].0.starts_with("GET "));
        assert!(requests[4].0.starts_with("DELETE /api/custom/emoji/emoji "));
    }
}
