//! Snapshot updates used by the member projection. Preserve typed identity
//! while merging partial gateway patches and clearing optional fields.
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use stonks_core::connection::{AppSnapshot, ClientRole, GatewayEvent};

pub fn deduplicate(snapshot: &mut AppSnapshot) {
    let mut seen = std::collections::HashSet::new();
    snapshot.members.reverse();
    snapshot
        .members
        .retain(|member| seen.insert((member.id.server.clone(), member.id.user.clone())));
    snapshot.members.reverse();
    let mut seen = std::collections::HashSet::new();
    snapshot.users.reverse();
    snapshot.users.retain(|user| seen.insert(user.id.clone()));
    snapshot.users.reverse();
}

fn merge(target: &mut Value, data: &Value) {
    if data.is_object() && !target.is_object() {
        *target = serde_json::json!({});
    }
    if let (Some(target), Some(data)) = (target.as_object_mut(), data.as_object()) {
        for (key, value) in data {
            if value.is_object() {
                merge(
                    target.entry(key).or_insert_with(|| serde_json::json!({})),
                    value,
                );
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn wire(value: &mut Value) {
    if let Some(array) = value.as_array_mut() {
        for item in array {
            wire(item);
        }
    }
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for value in object.values_mut() {
        wire(value);
    }
    for (projected, wire) in [
        ("id", "_id"),
        ("displayName", "display_name"),
        ("contentType", "content_type"),
        ("joinedAt", "joined_at"),
        ("defaultPermissions", "default_permissions"),
        ("systemMessages", "system_messages"),
    ] {
        if let Some(value) = object.remove(projected) {
            object.insert(wire.into(), value);
        }
    }
}

fn patch<T: Serialize + DeserializeOwned>(target: &mut T, data: &Value, clear: &Value) {
    let Ok(mut value) = serde_json::to_value(&*target) else {
        return;
    };
    wire(&mut value);
    let mut data = data.clone();
    wire(&mut data);
    merge(&mut value, &data);
    for field in clear
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if matches!(field, "StatusText" | "StatusPresence") {
            if let Some(status) = value.get_mut("status").and_then(Value::as_object_mut) {
                status.remove(if field == "StatusText" {
                    "text"
                } else {
                    "presence"
                });
            }
            continue;
        }
        let key = match field {
            "Avatar" => "avatar",
            "Nickname" => "nickname",
            "DisplayName" => "display_name",
            "Pronouns" => "pronouns",
            "Status" => "status",
            "Timeout" => "timeout",
            "Colour" => "colour",
            "Icon" => "icon",
            _ => continue,
        };
        value[key] = Value::Null;
    }
    if let Ok(updated) = serde_json::from_value(value) {
        *target = updated;
    }
}

pub fn apply(snapshot: &mut AppSnapshot, event: &GatewayEvent) {
    let data = &event.data["data"];
    let clear = &event.data["clear"];
    match event.event_type.as_str() {
        "UserUpdate" => {
            let Some(id) = event.data["id"].as_str() else {
                return;
            };
            for user in &mut snapshot.users {
                if user.id == id {
                    patch(user, data, clear);
                }
            }
            if snapshot.current_user.id == id {
                patch(&mut snapshot.current_user, data, clear);
            }
        }
        "ServerMemberUpdate" => {
            let (Some(server), Some(user)) = (
                event.data["id"]["server"].as_str(),
                event.data["id"]["user"].as_str(),
            ) else {
                return;
            };
            for member in &mut snapshot.members {
                if member.id.server == server && member.id.user == user {
                    patch(member, data, clear);
                }
            }
        }
        "ServerMemberLeave" => {
            let (Some(server), Some(user)) =
                (event.data["id"].as_str(), event.data["user"].as_str())
            else {
                return;
            };
            snapshot
                .members
                .retain(|member| member.id.server != server || member.id.user != user);
        }
        "ServerRoleUpdate" | "ServerRoleDelete" => {
            let Some(server) = snapshot
                .servers
                .iter_mut()
                .find(|server| Some(server.id.as_str()) == event.data["id"].as_str())
            else {
                return;
            };
            let Some(id) = event
                .data
                .get("role_id")
                .or_else(|| event.data.get("roleId"))
                .and_then(Value::as_str)
            else {
                return;
            };
            if event.event_type == "ServerRoleDelete" {
                server.roles.remove(id);
            } else if let Some(role) = server.roles.get_mut(id) {
                patch(role, data, clear);
            } else if let Ok(role) = serde_json::from_value::<ClientRole>(data.clone()) {
                server.roles.insert(id.into(), role);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> AppSnapshot {
        let user = serde_json::from_value(serde_json::json!({"_id":"a","username":"Alice","online":true,"avatar":{"_id":"gif","tag":"avatars"},"status":{"presence":"Online","text":"custom"}})).unwrap();
        AppSnapshot {
            current_user: user,
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: vec![],
        }
    }
    #[test]
    fn partial_presence_updates_keep_avatar_identity_and_other_status_fields() {
        let mut snapshot = snapshot();
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "UserUpdate".into(),
                data: serde_json::json!({"id":"a","data":{"online":false,"status":{"presence":"Busy"}}}),
            },
        );
        assert!(!snapshot.current_user.online);
        assert_eq!(snapshot.current_user.avatar.as_ref().unwrap().id, "gif");
        assert_eq!(
            snapshot
                .current_user
                .status
                .as_ref()
                .unwrap()
                .text
                .as_deref(),
            Some("custom")
        );
        assert_eq!(
            snapshot
                .current_user
                .status
                .as_ref()
                .unwrap()
                .presence
                .as_deref(),
            Some("Busy")
        );
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "UserUpdate".into(),
                data: serde_json::json!({"id":"a","data":{},"clear":["Avatar"]}),
            },
        );
        assert!(snapshot.current_user.avatar.is_none());
        snapshot.current_user.status = None;
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "UserUpdate".into(),
                data: serde_json::json!({"id":"a","data":{"status":{"presence":"Away"}}}),
            },
        );
        assert_eq!(
            snapshot
                .current_user
                .status
                .as_ref()
                .unwrap()
                .presence
                .as_deref(),
            Some("Away")
        );
    }
    #[test]
    fn member_patch_keeps_composite_identity_and_ready_deduplicates_users() {
        let mut snapshot = snapshot();
        let member = serde_json::from_value(
            serde_json::json!({"_id":{"server":"s","user":"a"},"nickname":"first"}),
        )
        .unwrap();
        snapshot.members = vec![member];
        snapshot.members.push(snapshot.members[0].clone());
        snapshot.users = vec![snapshot.current_user.clone(), snapshot.current_user.clone()];
        deduplicate(&mut snapshot);
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.users.len(), 1);
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "ServerMemberUpdate".into(),
                data: serde_json::json!({"id":{"server":"s","user":"a"},"data":{"roles":["r"]},"clear":["Nickname"]}),
            },
        );
        assert_eq!(snapshot.members[0].roles, ["r"]);
        assert!(snapshot.members[0].nickname.is_none());
        assert_eq!(snapshot.members[0].id.user, "a");
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "ServerMemberLeave".into(),
                data: serde_json::json!({"id":"s","user":"a"}),
            },
        );
        assert!(snapshot.members.is_empty());
    }
}
