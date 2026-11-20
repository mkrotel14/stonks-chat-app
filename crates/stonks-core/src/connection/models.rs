use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub state: &'static str,
    pub reason: Option<ConnectionStatusReason>,
    pub message: Option<&'static str>,
    pub attempt: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatusReason {
    AuthenticationFailed,
    ConnectionLost,
    SignedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientFileReference {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: String,
    pub tag: String,
    #[serde(
        default,
        rename(deserialize = "content_type", serialize = "contentType")
    )]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientStatus {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub presence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientUser {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(
        default,
        rename(deserialize = "display_name", serialize = "displayName")
    )]
    pub display_name: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub avatar: Option<ClientFileReference>,
    #[serde(default)]
    pub status: Option<ClientStatus>,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub privileged: bool,
    #[serde(default)]
    pub relationship: Option<String>,
    #[serde(default)]
    pub bot: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCategory {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientServer {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<ClientFileReference>,
    #[serde(default)]
    pub banner: Option<ClientFileReference>,
    #[serde(
        default,
        rename(deserialize = "system_messages", serialize = "systemMessages")
    )]
    pub system_messages: Option<std::collections::HashMap<String, String>>,
    pub owner: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default, deserialize_with = "null_vec")]
    pub categories: Vec<ClientCategory>,
    #[serde(
        default,
        rename(deserialize = "default_permissions", serialize = "defaultPermissions")
    )]
    pub default_permissions: u64,
    #[serde(default)]
    pub roles: std::collections::HashMap<String, ClientRole>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionOverride {
    #[serde(default)]
    pub a: u64,
    #[serde(default)]
    pub d: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRole {
    pub name: String,
    #[serde(default)]
    pub rank: Option<i64>,
    #[serde(default)]
    pub colour: Option<String>,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub icon: Option<ClientFileReference>,
    #[serde(default)]
    pub permissions: PermissionOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientChannel {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: String,
    #[serde(rename(deserialize = "channel_type", serialize = "channelType"))]
    pub channel_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub voice: Option<Value>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(
        default,
        rename(deserialize = "last_message_id", serialize = "lastMessageId")
    )]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub permissions: Option<u64>,
    #[serde(
        default,
        rename(deserialize = "default_permissions", serialize = "defaultPermissions")
    )]
    pub default_permissions: Option<PermissionOverride>,
    #[serde(
        default,
        rename(deserialize = "role_permissions", serialize = "rolePermissions")
    )]
    pub role_permissions: std::collections::HashMap<String, PermissionOverride>,
    #[serde(default)]
    pub slowmode: Option<u64>,
    #[serde(default)]
    pub nsfw: bool,
}

impl ClientChannel {
    pub fn is_voice(&self) -> bool {
        self.channel_type == "VoiceChannel" || self.voice.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMemberId {
    pub server: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientMember {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: ClientMemberId,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub avatar: Option<ClientFileReference>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default, rename(deserialize = "joined_at", serialize = "joinedAt"))]
    pub joined_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUnreadId {
    pub channel: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientUnread {
    #[serde(rename(deserialize = "_id", serialize = "id"))]
    pub id: ClientUnreadId,
    #[serde(default, rename(deserialize = "last_id", serialize = "lastId"))]
    pub last_id: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientVoiceParticipant {
    pub id: String,
    #[serde(default, alias = "joined_at")]
    // Current Stoat sends an ISO-8601 timestamp while older protocol typings
    // documented a numeric timestamp. Preserve either representation.
    pub joined_at: Value,
    // Omitted flags are unknown, not evidence of mute or deafen.
    #[serde(
        default,
        alias = "is_receiving",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_receiving: Option<bool>,
    #[serde(
        default,
        alias = "is_publishing",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_publishing: Option<bool>,
    #[serde(default)]
    pub screensharing: bool,
    #[serde(default)]
    pub camera: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientVoiceState {
    pub id: String,
    #[serde(default)]
    pub participants: Vec<ClientVoiceParticipant>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub current_user: ClientUser,
    pub users: Vec<ClientUser>,
    pub servers: Vec<ClientServer>,
    pub channels: Vec<ClientChannel>,
    pub members: Vec<ClientMember>,
    pub unreads: Vec<ClientUnread>,
    pub voice_states: Vec<ClientVoiceState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerMemberSnapshot {
    pub users: Vec<ClientUser>,
    pub members: Vec<ClientMember>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupRequest {
    pub name: String,
    pub recipients: Vec<String>,
}

fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}
