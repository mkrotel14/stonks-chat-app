use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::models::{AppSnapshot, ClientUser};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEvent {
    pub event_type: String,
    pub data: Value,
}

const MAX_GATEWAY_EVENT_BYTES: usize = 256 * 1024;
const MAX_GATEWAY_EVENT_DEPTH: usize = 12;
const MAX_GATEWAY_COLLECTION_ITEMS: usize = 2_048;
const MAX_GATEWAY_STRING_BYTES: usize = 32 * 1024;

fn gateway_fields(event_type: &str) -> Option<&'static [&'static str]> {
    Some(match event_type {
        "Message" => &[
            "_id",
            "id",
            "channel",
            "author",
            "content",
            "system",
            "user",
            "member",
            "masquerade",
            "webhook",
            "attachments",
            "embeds",
            "replies",
            "reactions",
            "pinned",
            "edited",
            "mentions",
            "role_mentions",
        ],
        "MessageDelete" => &["id", "channel"],
        "MessageUpdate" => &["id", "channel", "data", "clear"],
        "MessageAppend" => &["id", "channel", "append"],
        "MessageReact" | "MessageUnreact" | "MessageRemoveReaction" => &[
            "id",
            "channel",
            "channel_id",
            "channelId",
            "emoji",
            "emoji_id",
            "emojiId",
            "user",
            "user_id",
            "userId",
        ],
        "ChannelAck" => &["id", "user", "message_id", "messageId"],
        "ChannelStartTyping" | "ChannelStopTyping" | "ChannelDelete" => &["id", "user"],
        "ChannelCreate" => &[
            "_id",
            "id",
            "channel_type",
            "channelType",
            "name",
            "description",
            "voice",
            "server",
            "recipients",
            "last_message_id",
            "lastMessageId",
            "owner",
            "permissions",
            "default_permissions",
            "defaultPermissions",
            "role_permissions",
            "rolePermissions",
            "slowmode",
            "nsfw",
        ],
        "ChannelUpdate" => &["id", "data", "clear"],
        "ChannelGroupJoin" | "ChannelGroupLeave" => &["id", "user"],
        "ServerCreate" => &[
            "_id",
            "id",
            "server",
            "name",
            "description",
            "icon",
            "banner",
            "system_messages",
            "systemMessages",
            "owner",
            "channels",
            "categories",
            "default_permissions",
            "defaultPermissions",
            "roles",
            "voice_states",
            "voiceStates",
        ],
        "ServerUpdate" | "UserUpdate" => &["id", "data", "clear"],
        "UserRelationship" => &["id", "user"],
        "ServerDelete" => &["id"],
        "ServerRoleUpdate" => &["id", "role_id", "roleId", "data", "clear"],
        "ServerRoleDelete" => &["id", "role_id", "roleId"],
        "ServerMemberJoin" => &["id", "user", "member"],
        "ServerMemberUpdate" => &["id", "data", "clear"],
        "ServerMemberLeave" => &["id", "user"],
        "UserPlatformWipe" => &["user_id", "userId"],
        "VoiceChannelJoin" => &["id", "state"],
        "VoiceChannelLeave" => &["id", "user"],
        "VoiceChannelMove" => &["user", "from", "to", "state"],
        "UserVoiceStateUpdate" => &["id", "channel_id", "channelId", "data"],
        _ => return None,
    })
}

fn sensitive_gateway_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "token"
            | "accesstoken"
            | "refreshtoken"
            | "sessiontoken"
            | "secret"
            | "password"
            | "email"
            | "internalurl"
            | "backendurl"
            | "diagnostics"
            | "diagnostic"
            | "path"
            | "filepath"
    )
}

fn gateway_value_is_safe(value: &Value, depth: usize) -> bool {
    if depth > MAX_GATEWAY_EVENT_DEPTH {
        return false;
    }
    match value {
        Value::String(value) => value.len() <= MAX_GATEWAY_STRING_BYTES,
        Value::Array(values) => {
            values.len() <= MAX_GATEWAY_COLLECTION_ITEMS
                && values
                    .iter()
                    .all(|value| gateway_value_is_safe(value, depth + 1))
        }
        Value::Object(values) => {
            values.len() <= MAX_GATEWAY_COLLECTION_ITEMS
                && values.iter().all(|(key, value)| {
                    key.len() <= 128
                        && !sensitive_gateway_key(key)
                        && gateway_value_is_safe(value, depth + 1)
                })
        }
        _ => true,
    }
}

fn gateway_shape_is_valid(event_type: &str, data: &serde_json::Map<String, Value>) -> bool {
    let string = |key: &str| data.get(key).is_some_and(Value::is_string);
    let object = |key: &str| data.get(key).is_some_and(Value::is_object);
    match event_type {
        "Message" => string("_id") && string("channel") && (string("author") || object("system")),
        "MessageDelete" => string("id") && string("channel"),
        "MessageUpdate" => string("id") && string("channel") && object("data"),
        "MessageAppend" => string("id") && string("channel") && object("append"),
        "MessageReact" | "MessageUnreact" | "MessageRemoveReaction" => {
            string("id")
                && (string("channel") || string("channel_id") || string("channelId"))
                && (string("emoji") || string("emoji_id") || string("emojiId"))
        }
        "ChannelAck" => {
            string("id") && string("user") && (string("message_id") || string("messageId"))
        }
        "ChannelStartTyping" | "ChannelStopTyping" | "ChannelGroupJoin" | "ChannelGroupLeave"
        | "VoiceChannelLeave" | "ServerMemberLeave" => string("id") && string("user"),
        "ChannelCreate" => {
            (string("_id") || string("id")) && (string("channel_type") || string("channelType"))
        }
        "ChannelUpdate" | "ServerUpdate" | "UserUpdate" => string("id") && object("data"),
        "ChannelDelete" | "ServerDelete" => string("id"),
        "ServerCreate" => {
            (string("id") && object("server") && data.get("channels").is_some_and(Value::is_array))
                || ((string("_id") || string("id")) && string("name"))
        }
        "UserRelationship" => string("id") && object("user"),
        "ServerRoleUpdate" => {
            string("id") && (string("role_id") || string("roleId")) && object("data")
        }
        "ServerRoleDelete" => string("id") && (string("role_id") || string("roleId")),
        "ServerMemberJoin" => string("id") && data.contains_key("user") && object("member"),
        "ServerMemberUpdate" => object("id") && object("data"),
        "UserPlatformWipe" => string("user_id") || string("userId"),
        "VoiceChannelJoin" => string("id") && object("state"),
        "VoiceChannelMove" => string("user") && string("from") && string("to") && object("state"),
        "UserVoiceStateUpdate" => {
            string("id") && (string("channel_id") || string("channelId")) && object("data")
        }
        _ => false,
    }
}

pub(crate) fn gateway_event(value: Value) -> Option<GatewayEvent> {
    let values = value.as_object()?;
    let event_type = values.get("type")?.as_str()?;
    let fields = gateway_fields(event_type)?;
    if !gateway_value_is_safe(&value, 0) {
        return None;
    }
    let data = fields
        .iter()
        .filter_map(|field| {
            values
                .get(*field)
                .cloned()
                .map(|value| ((*field).into(), value))
        })
        .collect::<serde_json::Map<String, Value>>();
    if !gateway_shape_is_valid(event_type, &data) {
        return None;
    }
    let data = Value::Object(data);
    if serde_json::to_vec(&data).ok()?.len() > MAX_GATEWAY_EVENT_BYTES {
        return None;
    }
    Some(GatewayEvent {
        event_type: event_type.to_owned(),
        data,
    })
}

pub(crate) fn ready_snapshot(
    value: &Value,
    current_user: ClientUser,
) -> Result<AppSnapshot, String> {
    let parse = |field: &str| -> Result<Vec<Value>, String> {
        value
            .get(field)
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| format!("The gateway sent a malformed {field} array."))
    };
    let users = decode_items(parse("users")?, "users")?;
    let servers = decode_items(parse("servers")?, "servers")?;
    let channels = decode_items(parse("channels")?, "channels")?;
    let members = decode_items(parse("members")?, "members")?;
    let unreads = decode_items(parse("channel_unreads")?, "channel unreads")?;
    let voice_states = decode_items(parse("voice_states")?, "voice states")?;
    Ok(AppSnapshot {
        current_user,
        users,
        servers,
        channels,
        members,
        unreads,
        voice_states,
    })
}

pub(crate) fn decode_items<T: DeserializeOwned>(
    items: Vec<Value>,
    name: &str,
) -> Result<Vec<T>, String> {
    items
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("The gateway sent invalid {name}: {error}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn test_user() -> ClientUser {
        ClientUser {
            id: "u1".into(),
            username: "me".into(),
            discriminator: None,
            display_name: None,
            pronouns: None,
            avatar: None,
            status: None,
            online: true,
            privileged: false,
            relationship: None,
            bot: None,
        }
    }

    #[test]
    fn maps_ready_payload_to_safe_snapshot() {
        let ready = json!({
            "type": "Ready",
            "users": [{"_id":"u2","username":"friend","discriminator":"0001","relationship":"Friend","online":true,"bot":{"owner":"u1"}}],
            "servers": [{"_id":"s1","name":"Stoat","owner":"u1","channels":["c1"],"default_permissions":1048576,"roles":{"r1":{"name":"Member","rank":1,"permissions":{"a":4194304,"d":0}}},"approximate_member_count":2}],
            "channels": [{"_id":"c1","channel_type":"TextChannel","server":"s1","name":"general","voice":{},"last_message_id":"m2","default_permissions":{"a":2097152,"d":0},"role_permissions":{"r1":{"a":134217728,"d":0}}}],
            "members": [{"_id":{"server":"s1","user":"u1"},"pronouns":"they/them","roles":["r1"],"timeout":"2026-08-25T00:00:00.000Z"}],
            "channel_unreads": [{"_id":{"channel":"c1","user":"u1"},"last_id":"m1","mentions":[]}],
            "voice_states": [{"id":"c1","participants":[{"id":"u2","joined_at":"2026-08-25T12:34:56Z","is_receiving":true,"is_publishing":false,"screensharing":false,"camera":true}]}]
        });
        let snapshot = ready_snapshot(&ready, test_user()).unwrap();
        let voice = &snapshot.voice_states[0];
        assert_eq!(voice.id, "c1");
        let participant = &voice.participants[0];
        assert_eq!(participant.id, "u2");
        assert_eq!(participant.joined_at, "2026-08-25T12:34:56Z");
        assert_eq!(participant.is_receiving, Some(true));
        assert_eq!(participant.is_publishing, Some(false));
        assert_eq!(snapshot.servers.len(), 1);
        assert!(snapshot.users[0].bot.is_some());
        assert_eq!(snapshot.channels.len(), 1);
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.unreads.len(), 1);
        assert_eq!(snapshot.channels[0].last_message_id.as_deref(), Some("m2"));
        assert_eq!(snapshot.unreads[0].last_id.as_deref(), Some("m1"));
        assert_eq!(snapshot.servers[0].default_permissions, 1_048_576);
        assert_eq!(snapshot.servers[0].roles["r1"].permissions.a, 4_194_304);
        assert_eq!(snapshot.channels[0].role_permissions["r1"].a, 134_217_728);
        assert_eq!(
            snapshot.members[0].timeout.as_deref(),
            Some("2026-08-25T00:00:00.000Z")
        );
        assert_eq!(snapshot.members[0].pronouns.as_deref(), Some("they/them"));
    }

    #[test]
    fn projects_stoat_live_voice_events_without_dropping_roster_fields() {
        let state = json!({"id":"u2","joined_at":"2026-08-25T12:34:56Z","is_receiving":true,"is_publishing":false,"screensharing":false,"camera":true});
        for mut wire in [
            json!({"type":"VoiceChannelJoin","id":"c1","state":state}),
            json!({"type":"VoiceChannelMove","user":"u2","from":"c1","to":"c2","state":state}),
            json!({"type":"UserVoiceStateUpdate","id":"u2","channel_id":"c2","data":{"is_publishing":true}}),
            json!({"type":"VoiceChannelLeave","id":"c2","user":"u2"}),
        ] {
            let event = gateway_event(wire.clone()).expect("valid Stoat voice event");
            let kind = wire.as_object_mut().unwrap().remove("type").unwrap();
            assert_eq!(event.event_type, kind.as_str().unwrap());
            assert_eq!(event.data, wire);
        }
    }

    #[test]
    fn rejects_missing_or_non_array_ready_collections() {
        let valid = json!({
            "users": [],
            "servers": [],
            "channels": [],
            "members": [],
            "channel_unreads": [],
            "voice_states": []
        });
        for field in [
            "users",
            "servers",
            "channels",
            "members",
            "channel_unreads",
            "voice_states",
        ] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(ready_snapshot(&missing, test_user()).is_err(), "{field}");

            let mut malformed = valid.clone();
            malformed[field] = json!({});
            assert!(ready_snapshot(&malformed, test_user()).is_err(), "{field}");
        }
    }

    #[test]
    fn gateway_events_allow_only_consumed_kinds_and_project_fields() {
        assert!(gateway_event(json!({"type": "Unknown", "value": 1})).is_none());
        assert!(gateway_event(json!({"type": "FutureEvent", "value": 1})).is_none());
        assert!(gateway_event(json!({"value": 1})).is_none());
        assert!(gateway_event(json!({"type": "Message", "content": "missing identity"})).is_none());
        assert!(gateway_event(json!({
            "type": "Message",
            "_id": "m1",
            "channel": "c1",
            "author": "u1",
            "content": "x".repeat(MAX_GATEWAY_STRING_BYTES + 1)
        }))
        .is_none());

        let event = gateway_event(json!({
            "type": "Message",
            "_id": "m1",
            "channel": "c1",
            "author": "u1",
            "content": "safe",
            "backend_only": "not emitted"
        }))
        .unwrap();
        assert_eq!(event.event_type, "Message");
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "eventType": "Message",
                "data": {"_id": "m1", "channel": "c1", "author": "u1", "content": "safe"}
            })
        );

        let relationship = gateway_event(json!({
            "type": "UserRelationship",
            "id": "u1",
            "user": { "_id": "u2", "username": "friend", "relationship": "Incoming" }
        }))
        .unwrap();
        assert_eq!(relationship.event_type, "UserRelationship");
        assert_eq!(relationship.data["user"]["relationship"], "Incoming");

        let created = gateway_event(json!({
            "type": "ServerCreate",
            "id": "s1",
            "server": { "_id": "s1", "name": "Server", "owner": "u1", "channels": ["c1"] },
            "channels": [{ "_id": "c1", "channel_type": "TextChannel", "server": "s1" }],
            "emojis": [{ "_id": "not-projected" }],
            "voice_states": []
        }))
        .unwrap();
        assert_eq!(created.event_type, "ServerCreate");
        assert_eq!(created.data["server"]["_id"], "s1");
        assert_eq!(created.data["channels"][0]["_id"], "c1");
        assert!(created.data.get("emojis").is_none());

        let compatible = gateway_event(json!({
            "type": "ServerCreate",
            "_id": "s2",
            "name": "Compatible",
            "owner": "u1",
            "channels": ["c2"]
        }))
        .unwrap();
        assert_eq!(compatible.event_type, "ServerCreate");
        assert_eq!(compatible.data["_id"], "s2");
        assert_eq!(compatible.data["name"], "Compatible");
    }

    #[test]
    fn gateway_events_reject_sensitive_keys_at_any_depth() {
        for sensitive in [
            json!({"token": "session"}),
            json!({"path": "/srv/private"}),
            json!({"internal_url": "http://backend.internal"}),
            json!({"nested": {"diagnostics": "stack trace"}}),
            json!({"nested": [{"email": "person@example.test"}]}),
        ] {
            let mut value = json!({
                "type": "Message",
                "_id": "m1",
                "channel": "c1",
                "author": "u1"
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(sensitive.as_object().unwrap().clone());
            assert!(gateway_event(value).is_none());
        }
    }
}
