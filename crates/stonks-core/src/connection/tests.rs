use std::cell::Cell;

use serde_json::{json, Value};

use super::state::ConnectionManager;

const KICK_MEMBERS: u64 = 1 << 6;
const VIEW_CHANNEL: u64 = 1 << 20;

fn authorization_snapshot() -> Value {
    json!({
        "currentUser": { "_id": "u1", "privileged": true },
        "ready": { "servers": [], "channels": [], "members": [] }
    })
}

fn moderation_snapshot(
    admin_permissions: Value,
    member_permissions: Value,
    timeout: Option<&str>,
    channel_permissions: Value,
) -> Value {
    json!({
        "currentUser": { "_id": "u1", "privileged": false },
        "ready": {
            "servers": [{
                "_id": "server",
                "owner": "owner",
                "default_permissions": VIEW_CHANNEL,
                "roles": {
                    "admin": { "rank": 1, "permissions": admin_permissions },
                    "member": { "rank": 100, "permissions": member_permissions }
                }
            }],
            "channels": [{
                "_id": "channel",
                "server": "server",
                "default_permissions": { "a": 0, "d": 0 },
                "role_permissions": channel_permissions
            }],
            "members": [{
                "_id": { "server": "server", "user": "u1" },
                "roles": ["admin", "member"],
                "timeout": timeout
            }]
        }
    })
}

fn lifecycle_snapshot(owner: &str) -> Value {
    json!({
        "currentUser": { "id": "u1", "privileged": false },
        "ready": {
            "users": [],
            "servers": [{
                "_id": "server",
                "name": "Server",
                "owner": owner,
                "channels": ["channel"],
                "default_permissions": VIEW_CHANNEL | KICK_MEMBERS,
                "roles": {}
            }],
            "channels": [{
                "_id": "channel",
                "channel_type": "TextChannel",
                "server": "server",
                "default_permissions": { "a": 0, "d": 0 },
                "role_permissions": {}
            }],
            "members": [{
                "_id": { "server": "server", "user": "u1" },
                "roles": []
            }],
            "channel_unreads": [{ "_id": { "channel": "channel", "user": "u1" } }],
            "voice_states": [{ "id": "channel", "participants": [] }]
        }
    })
}

fn manager_with(snapshot: Value) -> ConnectionManager {
    let manager = ConnectionManager::default();
    let generation = manager.begin(None).unwrap();
    manager
        .publish_ready(generation, snapshot, || Ok(()))
        .unwrap();
    manager
}

#[test]
fn replacement_clears_ready_and_rejects_stale_publication() {
    let manager = ConnectionManager::default();
    let first = manager.begin(None).unwrap();
    assert!(manager
        .publish_ready(first, authorization_snapshot(), || Ok(()))
        .unwrap());
    assert!(manager.require_server_permission("server", 1).is_ok());

    let second = manager.begin(None).unwrap();
    assert!(manager.require_server_permission("server", 1).is_err());
    let stale_published = Cell::new(false);
    assert!(!manager
        .publish(first, || {
            stale_published.set(true);
            Ok(())
        })
        .unwrap());
    assert!(!manager
        .publish_ready(first, authorization_snapshot(), || {
            stale_published.set(true);
            Ok(())
        })
        .unwrap());
    assert!(!stale_published.get());

    assert!(manager.publish(second, || Ok(())).unwrap());
}

#[test]
fn stop_logout_and_terminal_exit_clear_ready_ownership() {
    let manager = ConnectionManager::default();

    let logout = manager.begin(None).unwrap();
    manager
        .publish_ready(logout, authorization_snapshot(), || Ok(()))
        .unwrap();
    manager.clear_ready(logout);
    assert!(manager.require_server_permission("server", 1).is_err());

    manager
        .publish_ready(logout, authorization_snapshot(), || Ok(()))
        .unwrap();
    manager.finish(logout);
    assert!(manager.require_server_permission("server", 1).is_err());
    assert!(!manager.publish(logout, || Ok(())).unwrap());

    let stopped = manager.begin(None).unwrap();
    manager
        .publish_ready(stopped, authorization_snapshot(), || Ok(()))
        .unwrap();
    manager.stop(None).unwrap();
    assert!(manager.require_server_permission("server", 1).is_err());
    assert!(!manager.publish(stopped, || Ok(())).unwrap());
}

#[test]
fn server_permissions_apply_lower_priority_roles_before_higher_priority_roles() {
    let allowed = manager_with(moderation_snapshot(
        json!({ "a": KICK_MEMBERS, "d": 0 }),
        json!({ "a": 0, "d": KICK_MEMBERS }),
        None,
        json!({}),
    ));
    assert!(allowed
        .require_server_permission("server", KICK_MEMBERS)
        .is_ok());

    let denied = manager_with(moderation_snapshot(
        json!({ "a": 0, "d": KICK_MEMBERS }),
        json!({ "a": KICK_MEMBERS, "d": 0 }),
        None,
        json!({}),
    ));
    assert!(denied
        .require_server_permission("server", KICK_MEMBERS)
        .is_err());
}

#[test]
fn channel_permissions_apply_ranked_server_then_ranked_channel_overrides() {
    let ranked_channel = manager_with(moderation_snapshot(
        json!({ "a": 0, "d": 0 }),
        json!({ "a": 0, "d": 0 }),
        None,
        json!({
            "admin": { "a": KICK_MEMBERS, "d": 0 },
            "member": { "a": 0, "d": KICK_MEMBERS }
        }),
    ));
    assert!(ranked_channel
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_ok());

    let staged = manager_with(moderation_snapshot(
        json!({ "a": KICK_MEMBERS, "d": 0 }),
        json!({ "a": 0, "d": 0 }),
        None,
        json!({ "member": { "a": 0, "d": KICK_MEMBERS } }),
    ));
    assert!(staged
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_err());
}

#[test]
fn only_future_timeout_timestamps_restrict_permissions() {
    let expired = manager_with(moderation_snapshot(
        json!({ "a": KICK_MEMBERS, "d": 0 }),
        json!({ "a": 0, "d": 0 }),
        Some("2000-01-01T00:00:00.000Z"),
        json!({}),
    ));
    assert!(expired
        .require_server_permission("server", KICK_MEMBERS)
        .is_ok());
    assert!(expired
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_ok());

    let active = manager_with(moderation_snapshot(
        json!({ "a": KICK_MEMBERS, "d": 0 }),
        json!({ "a": 0, "d": 0 }),
        Some("2999-01-01T00:00:00.000Z"),
        json!({}),
    ));
    assert!(active
        .require_server_permission("server", KICK_MEMBERS)
        .is_err());
    assert!(active
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_err());
}

#[test]
fn permission_relevant_gateway_updates_refresh_native_preflight() {
    let manager = ConnectionManager::default();
    let generation = manager.begin(None).unwrap();
    manager
        .publish_ready(
            generation,
            moderation_snapshot(
                json!({ "a": 0, "d": 0 }),
                json!({ "a": 0, "d": 0 }),
                None,
                json!({}),
            ),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_server_permission("server", KICK_MEMBERS)
        .is_err());

    manager
        .publish_gateway_event(
            generation,
            "ServerRoleUpdate",
            &json!({
                "id": "server",
                "role_id": "admin",
                "data": { "permissions": { "a": KICK_MEMBERS, "d": 0 } }
            }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_server_permission("server", KICK_MEMBERS)
        .is_ok());
    assert!(manager
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_ok());

    manager
        .publish_gateway_event(
            generation,
            "ChannelUpdate",
            &json!({
                "id": "channel",
                "data": { "rolePermissions": { "admin": { "a": 0, "d": KICK_MEMBERS } } }
            }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_err());

    manager
        .publish_gateway_event(
            generation,
            "ServerMemberUpdate",
            &json!({
                "id": { "server": "server", "user": "u1" },
                "data": { "roles": ["member"] }
            }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_server_permission("server", KICK_MEMBERS)
        .is_err());
}

#[test]
fn ownership_intent_checks_distinguish_delete_from_leave() {
    let owner = manager_with(lifecycle_snapshot("u1"));
    assert!(owner.require_server_owner("server").is_ok());
    assert!(owner.require_server_non_owner_member("server").is_err());

    let member = manager_with(lifecycle_snapshot("owner"));
    assert!(member.require_server_owner("server").is_err());
    assert!(member.require_server_non_owner_member("server").is_ok());
}

#[test]
fn server_and_channel_lifecycle_events_refresh_native_authorization() {
    let manager = ConnectionManager::default();
    let generation = manager.begin(None).unwrap();
    manager
        .publish_ready(
            generation,
            json!({
                "currentUser": { "id": "u1", "privileged": false },
                "ready": {
                    "users": [],
                    "servers": [],
                    "channels": [],
                    "members": [],
                    "channel_unreads": [],
                    "voice_states": []
                }
            }),
            || Ok(()),
        )
        .unwrap();

    manager
        .publish_gateway_event(
            generation,
            "ServerCreate",
            &json!({
                "id": "server",
                "server": {
                    "_id": "server",
                    "name": "Server",
                    "owner": "owner",
                    "channels": ["channel"],
                    "default_permissions": VIEW_CHANNEL | KICK_MEMBERS,
                    "roles": {}
                },
                "channels": [{
                    "_id": "channel",
                    "channel_type": "TextChannel",
                    "server": "server",
                    "default_permissions": { "a": 0, "d": 0 },
                    "role_permissions": {}
                }]
            }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager.require_server_non_owner_member("server").is_ok());
    assert!(manager
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_ok());

    manager
        .publish_gateway_event(
            generation,
            "ChannelDelete",
            &json!({ "id": "channel" }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_channel_permission("channel", KICK_MEMBERS)
        .is_err());

    manager
        .publish_gateway_event(
            generation,
            "ChannelCreate",
            &json!({
                "_id": "replacement",
                "channel_type": "TextChannel",
                "server": "server",
                "default_permissions": { "a": 0, "d": 0 },
                "role_permissions": {}
            }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager
        .require_channel_permission("replacement", KICK_MEMBERS)
        .is_ok());

    manager
        .publish_gateway_event(
            generation,
            "ServerMemberLeave",
            &json!({ "id": "server", "user": "u1" }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager.require_server_non_owner_member("server").is_err());
    assert!(manager
        .require_channel_permission("replacement", KICK_MEMBERS)
        .is_err());
}

#[test]
fn server_delete_removes_native_authorization_state() {
    let manager = manager_with(lifecycle_snapshot("u1"));
    assert!(manager.require_server_owner("server").is_ok());
    let generation = 1;
    manager
        .publish_gateway_event(
            generation,
            "ServerDelete",
            &json!({ "id": "server" }),
            || Ok(()),
        )
        .unwrap();
    assert!(manager.require_server_owner("server").is_err());
    assert!(manager
        .require_channel_permission("channel", VIEW_CHANNEL)
        .is_err());
}

#[test]
fn stale_gateway_updates_cannot_change_replacement_permissions() {
    let manager = ConnectionManager::default();
    let stale = manager.begin(None).unwrap();
    let current = manager.begin(None).unwrap();
    manager
        .publish_ready(
            current,
            moderation_snapshot(
                json!({ "a": 0, "d": 0 }),
                json!({ "a": 0, "d": 0 }),
                None,
                json!({}),
            ),
            || Ok(()),
        )
        .unwrap();

    assert!(!manager
        .publish_gateway_event(
            stale,
            "ServerRoleUpdate",
            &json!({
                "id": "server",
                "roleId": "admin",
                "data": { "permissions": { "a": KICK_MEMBERS, "d": 0 } }
            }),
            || Ok(()),
        )
        .unwrap());
    assert!(manager
        .require_server_permission("server", KICK_MEMBERS)
        .is_err());
}

#[test]
fn stale_owner_cannot_stop_a_replacement_connection() {
    let manager = ConnectionManager::default();
    let stale = manager.begin(Some("renderer-a".into())).unwrap();
    let current = manager.begin(Some("renderer-b".into())).unwrap();
    manager
        .publish_ready(current, authorization_snapshot(), || Ok(()))
        .unwrap();

    manager.stop(Some("renderer-a")).unwrap();
    assert!(manager.publish(current, || Ok(())).unwrap());
    assert!(manager.require_server_permission("server", 1).is_ok());
    assert!(!manager.publish(stale, || Ok(())).unwrap());

    manager.stop(Some("renderer-b")).unwrap();
    assert!(!manager.publish(current, || Ok(())).unwrap());
    assert!(manager.require_server_permission("server", 1).is_err());
}
