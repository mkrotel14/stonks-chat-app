use std::{
    collections::HashSet,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use tokio::sync::mpsc;

pub struct ConnectionManager {
    state: Mutex<ConnectionState>,
}

#[derive(Default)]
struct ConnectionState {
    generation: u64,
    active: bool,
    owner_id: Option<String>,
    runtime: Option<ConnectionRuntime>,
    ready: Option<Value>,
}

struct ConnectionRuntime {
    task: tokio::task::JoinHandle<()>,
    outgoing: mpsc::UnboundedSender<Value>,
}

fn parse_digits(value: &[u8]) -> Option<i64> {
    value.iter().try_fold(0_i64, |number, byte| {
        byte.is_ascii_digit()
            .then(|| number * 10 + i64::from(byte - b'0'))
    })
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn parse_rfc3339(value: &str) -> Option<(i64, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = parse_digits(&bytes[0..4])?;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        28 + i64::from(leap_year),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || !(1..=month_days[(month - 1) as usize]).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let mut index = 19;
    let mut nanos = 0_u32;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
        for (place, byte) in bytes[fraction_start..index].iter().take(9).enumerate() {
            nanos += u32::from(byte - b'0') * 10_u32.pow(8 - place as u32);
        }
    }

    let offset = match bytes.get(index)? {
        b'Z' | b'z' if index + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if index + 6 == bytes.len() && bytes.get(index + 3) == Some(&b':') => {
            let hours = parse_digits(&bytes[index + 1..index + 3])?;
            let minutes = parse_digits(&bytes[index + 4..index + 6])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = hours * 3_600 + minutes * 60;
            if *sign == b'+' {
                seconds
            } else {
                -seconds
            }
        }
        _ => return None,
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset;
    Some((seconds, nanos))
}

fn timeout_is_active(timeout: Option<&str>) -> bool {
    let Some((seconds, nanos)) = timeout.and_then(parse_rfc3339) else {
        return false;
    };
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return false;
    };
    seconds > now.as_secs() as i64
        || (seconds == now.as_secs() as i64 && nanos > now.subsec_nanos())
}

fn patch_object(target: &mut Value, data: &Value, clear: Option<&Value>) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    if let Some(fields) = clear.and_then(Value::as_array) {
        for field in fields.iter().filter_map(Value::as_str) {
            target.remove(field);
            match field {
                "defaultPermissions" => {
                    target.remove("default_permissions");
                }
                "rolePermissions" => {
                    target.remove("role_permissions");
                }
                _ => {}
            }
        }
    }
    let Some(data) = data.as_object() else {
        return;
    };
    for (key, value) in data {
        target.insert(key.clone(), value.clone());
        match key.as_str() {
            "defaultPermissions" => {
                target.insert("default_permissions".into(), value.clone());
            }
            "rolePermissions" => {
                target.insert("role_permissions".into(), value.clone());
            }
            _ => {}
        }
    }
}

fn ready_collection_mut<'a>(
    authorization: &'a mut Value,
    field: &str,
) -> Option<&'a mut Vec<Value>> {
    authorization
        .get_mut("ready")?
        .get_mut(field)?
        .as_array_mut()
}

fn object_id(value: &Value) -> Option<&str> {
    value
        .get("_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

fn current_user_id(authorization: &Value) -> Option<&str> {
    authorization.get("currentUser").and_then(object_id)
}

fn upsert_ready_value(authorization: &mut Value, field: &str, value: Value) {
    let Some(id) = object_id(&value).map(str::to_owned) else {
        return;
    };
    let Some(values) = ready_collection_mut(authorization, field) else {
        return;
    };
    if let Some(existing) = values
        .iter_mut()
        .find(|candidate| object_id(candidate) == Some(id.as_str()))
    {
        *existing = value;
    } else {
        values.push(value);
    }
}

fn remove_channel_from_authorization(authorization: &mut Value, channel_id: &str) {
    if let Some(channels) = ready_collection_mut(authorization, "channels") {
        channels.retain(|channel| object_id(channel) != Some(channel_id));
    }
    if let Some(servers) = ready_collection_mut(authorization, "servers") {
        for server in servers {
            if let Some(channels) = server.get_mut("channels").and_then(Value::as_array_mut) {
                channels.retain(|id| id.as_str() != Some(channel_id));
            }
            if let Some(categories) = server.get_mut("categories").and_then(Value::as_array_mut) {
                for category in categories {
                    if let Some(channels) =
                        category.get_mut("channels").and_then(Value::as_array_mut)
                    {
                        channels.retain(|id| id.as_str() != Some(channel_id));
                    }
                }
            }
        }
    }
    if let Some(unreads) = ready_collection_mut(authorization, "channel_unreads") {
        unreads.retain(|unread| {
            unread
                .get("_id")
                .and_then(|id| id.get("channel"))
                .and_then(Value::as_str)
                != Some(channel_id)
        });
    }
    if let Some(voice_states) = ready_collection_mut(authorization, "voice_states") {
        voice_states.retain(|voice| object_id(voice) != Some(channel_id));
    }
}

fn remove_server_from_authorization(authorization: &mut Value, server_id: &str) {
    let mut channel_ids = HashSet::new();
    if let Some(servers) = ready_collection_mut(authorization, "servers") {
        for server in servers
            .iter()
            .filter(|server| object_id(server) == Some(server_id))
        {
            if let Some(channels) = server.get("channels").and_then(Value::as_array) {
                channel_ids.extend(channels.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
        servers.retain(|server| object_id(server) != Some(server_id));
    }
    if let Some(channels) = ready_collection_mut(authorization, "channels") {
        for channel in channels
            .iter()
            .filter(|channel| channel.get("server").and_then(Value::as_str) == Some(server_id))
        {
            if let Some(id) = object_id(channel) {
                channel_ids.insert(id.to_owned());
            }
        }
        channels.retain(|channel| channel.get("server").and_then(Value::as_str) != Some(server_id));
    }
    if let Some(members) = ready_collection_mut(authorization, "members") {
        members.retain(|member| {
            member
                .get("_id")
                .and_then(|id| id.get("server"))
                .and_then(Value::as_str)
                != Some(server_id)
        });
    }
    if let Some(unreads) = ready_collection_mut(authorization, "channel_unreads") {
        unreads.retain(|unread| {
            unread
                .get("_id")
                .and_then(|id| id.get("channel"))
                .and_then(Value::as_str)
                .is_none_or(|id| !channel_ids.contains(id))
        });
    }
    if let Some(voice_states) = ready_collection_mut(authorization, "voice_states") {
        voice_states.retain(|voice| object_id(voice).is_none_or(|id| !channel_ids.contains(id)));
    }
}

fn apply_permission_gateway_event(authorization: &mut Value, event_type: &str, event: &Value) {
    let clear = event.get("clear");
    match event_type {
        "UserRelationship" => {
            if let Some(user) = event.get("user").filter(|user| user.is_object()) {
                upsert_ready_value(authorization, "users", user.clone());
            }
        }
        "ServerCreate" => {
            let current_user = current_user_id(authorization).map(str::to_owned);
            let mut server = event
                .get("server")
                .cloned()
                .unwrap_or_else(|| event.clone());
            if object_id(&server).is_none() {
                if let (Some(id), Some(server)) = (
                    event.get("id").and_then(Value::as_str),
                    server.as_object_mut(),
                ) {
                    server.insert("_id".into(), Value::String(id.to_owned()));
                }
            }
            let server_id = object_id(&server).map(str::to_owned);
            upsert_ready_value(authorization, "servers", server);
            if let Some(channels) = event.get("channels").and_then(Value::as_array) {
                for channel in channels.iter().filter(|channel| channel.is_object()) {
                    upsert_ready_value(authorization, "channels", channel.clone());
                }
            }
            if let (Some(server_id), Some(user_id)) = (server_id, current_user) {
                let has_member = authorization
                    .get("ready")
                    .and_then(|ready| ready.get("members"))
                    .and_then(Value::as_array)
                    .is_some_and(|members| {
                        members.iter().any(|member| {
                            member
                                .get("_id")
                                .and_then(Value::as_object)
                                .is_some_and(|id| {
                                    id.get("server").and_then(Value::as_str)
                                        == Some(server_id.as_str())
                                        && id.get("user").and_then(Value::as_str)
                                            == Some(user_id.as_str())
                                })
                        })
                    });
                if !has_member {
                    if let Some(members) = ready_collection_mut(authorization, "members") {
                        members.push(serde_json::json!({
                            "_id": { "server": server_id, "user": user_id },
                            "roles": []
                        }));
                    }
                }
            }
        }
        "ServerDelete" => {
            if let Some(id) = event.get("id").and_then(Value::as_str) {
                remove_server_from_authorization(authorization, id);
            }
        }
        "ChannelCreate" => {
            upsert_ready_value(authorization, "channels", event.clone());
        }
        "ChannelDelete" => {
            if let Some(id) = event.get("id").and_then(Value::as_str) {
                remove_channel_from_authorization(authorization, id);
            }
        }
        "ServerMemberLeave" => {
            let (Some(server_id), Some(user_id)) = (
                event.get("id").and_then(Value::as_str),
                event.get("user").and_then(Value::as_str),
            ) else {
                return;
            };
            if current_user_id(authorization) == Some(user_id) {
                remove_server_from_authorization(authorization, server_id);
            } else if let Some(members) = ready_collection_mut(authorization, "members") {
                members.retain(|member| {
                    !member
                        .get("_id")
                        .and_then(Value::as_object)
                        .is_some_and(|id| {
                            id.get("server").and_then(Value::as_str) == Some(server_id)
                                && id.get("user").and_then(Value::as_str) == Some(user_id)
                        })
                });
            }
        }
        "ServerUpdate" => {
            let Some(id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(server) =
                ready_collection_mut(authorization, "servers").and_then(|servers| {
                    servers
                        .iter_mut()
                        .find(|server| object_id(server) == Some(id))
                })
            {
                patch_object(server, &event["data"], clear);
            }
        }
        "ChannelUpdate" => {
            let Some(id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(channel) =
                ready_collection_mut(authorization, "channels").and_then(|channels| {
                    channels
                        .iter_mut()
                        .find(|channel| object_id(channel) == Some(id))
                })
            {
                patch_object(channel, &event["data"], clear);
            }
        }
        "ServerRoleUpdate" => {
            let Some(server_id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            let Some(role_id) = event
                .get("role_id")
                .or_else(|| event.get("roleId"))
                .and_then(Value::as_str)
            else {
                return;
            };
            let Some(server) = ready_collection_mut(authorization, "servers").and_then(|servers| {
                servers
                    .iter_mut()
                    .find(|server| object_id(server) == Some(server_id))
            }) else {
                return;
            };
            let Some(roles) = server.get_mut("roles").and_then(Value::as_object_mut) else {
                return;
            };
            let role = roles
                .entry(role_id.to_owned())
                .or_insert_with(|| Value::Object(Default::default()));
            patch_object(role, &event["data"], clear);
        }
        "ServerRoleDelete" => {
            let Some(server_id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            let Some(role_id) = event
                .get("role_id")
                .or_else(|| event.get("roleId"))
                .and_then(Value::as_str)
            else {
                return;
            };
            if let Some(roles) = ready_collection_mut(authorization, "servers")
                .and_then(|servers| {
                    servers
                        .iter_mut()
                        .find(|server| object_id(server) == Some(server_id))
                })
                .and_then(|server| server.get_mut("roles"))
                .and_then(Value::as_object_mut)
            {
                roles.remove(role_id);
            }
        }
        "ServerMemberUpdate" => {
            let Some(id) = event.get("id").and_then(Value::as_object) else {
                return;
            };
            let (server_id, user_id) = (
                id.get("server").and_then(Value::as_str),
                id.get("user").and_then(Value::as_str),
            );
            if let Some(member) =
                ready_collection_mut(authorization, "members").and_then(|members| {
                    members.iter_mut().find(|member| {
                        member
                            .get("_id")
                            .and_then(Value::as_object)
                            .is_some_and(|candidate| {
                                candidate.get("server").and_then(Value::as_str) == server_id
                                    && candidate.get("user").and_then(Value::as_str) == user_id
                            })
                    })
                })
            {
                patch_object(member, &event["data"], clear);
            }
        }
        "UserUpdate" => {
            let Some(id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            if authorization
                .get("currentUser")
                .and_then(object_id)
                .is_some_and(|current| current == id)
            {
                if let Some(user) = authorization.get_mut("currentUser") {
                    patch_object(user, &event["data"], clear);
                }
            }
        }
        _ => {}
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self {
            state: Mutex::new(ConnectionState::default()),
        }
    }
}

impl ConnectionManager {
    pub(crate) fn begin(&self, owner_id: Option<String>) -> Result<u64, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        state.generation = state.generation.wrapping_add(1);
        state.active = true;
        state.owner_id = owner_id;
        state.ready = None;
        if let Some(previous) = state.runtime.take() {
            previous.task.abort();
        }
        Ok(state.generation)
    }

    pub(crate) fn install(
        &self,
        generation: u64,
        task: tokio::task::JoinHandle<()>,
        outgoing: mpsc::UnboundedSender<Value>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        if state.generation != generation || !state.active {
            task.abort();
            return Ok(());
        }
        state.runtime = Some(ConnectionRuntime { task, outgoing });
        Ok(())
    }

    pub(crate) fn stop(&self, owner_id: Option<&str>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        if owner_id.is_some() && state.owner_id.as_deref() != owner_id {
            return Ok(());
        }
        state.generation = state.generation.wrapping_add(1);
        state.active = false;
        state.owner_id = None;
        state.ready = None;
        if let Some(runtime) = state.runtime.take() {
            runtime.task.abort();
        }
        Ok(())
    }

    pub(crate) fn finish(&self, generation: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.generation == generation {
                state.active = false;
                state.owner_id = None;
                state.ready = None;
                state.runtime = None;
            }
        }
    }

    pub(crate) fn clear_ready(&self, generation: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.generation == generation && state.active {
                state.ready = None;
            }
        }
    }

    pub(crate) fn publish(
        &self,
        generation: u64,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        if state.generation != generation || !state.active {
            return Ok(false);
        }
        publish()?;
        Ok(true)
    }

    pub(crate) fn publish_ready(
        &self,
        generation: u64,
        value: Value,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        if state.generation != generation || !state.active {
            return Ok(false);
        }
        state.ready = Some(value);
        publish()?;
        Ok(true)
    }

    pub(crate) fn publish_gateway_event(
        &self,
        generation: u64,
        event_type: &str,
        event: &Value,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        if state.generation != generation || !state.active {
            return Ok(false);
        }
        if let Some(ready) = state.ready.as_mut() {
            apply_permission_gateway_event(ready, event_type, event);
        }
        publish()?;
        Ok(true)
    }

    pub fn require_server_owner(&self, server_id: &str) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "The authorization state is unavailable.")?;
        let ready = state
            .ready
            .as_ref()
            .ok_or("Ownership is unavailable until the gateway is ready.")?;
        let user_id = current_user_id(ready).ok_or("The authenticated user is invalid.")?;
        let server = ready
            .get("ready")
            .and_then(|payload| payload.get("servers"))
            .and_then(Value::as_array)
            .and_then(|servers| {
                servers
                    .iter()
                    .find(|server| object_id(server) == Some(server_id))
            })
            .ok_or("The server is not available in the current ownership snapshot.")?;
        if server.get("owner").and_then(Value::as_str) == Some(user_id) {
            Ok(())
        } else {
            Err("Only the server owner can delete this server.".into())
        }
    }

    pub fn require_server_non_owner_member(&self, server_id: &str) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "The authorization state is unavailable.")?;
        let ready = state
            .ready
            .as_ref()
            .ok_or("Membership is unavailable until the gateway is ready.")?;
        let user_id = current_user_id(ready).ok_or("The authenticated user is invalid.")?;
        let payload = ready
            .get("ready")
            .ok_or("The gateway membership snapshot is unavailable.")?;
        let server = payload
            .get("servers")
            .and_then(Value::as_array)
            .and_then(|servers| {
                servers
                    .iter()
                    .find(|server| object_id(server) == Some(server_id))
            })
            .ok_or("The server is not available in the current membership snapshot.")?;
        if server.get("owner").and_then(Value::as_str) == Some(user_id) {
            return Err("The server owner must delete the server instead of leaving it.".into());
        }
        let is_member = payload
            .get("members")
            .and_then(Value::as_array)
            .is_some_and(|members| {
                members.iter().any(|member| {
                    member
                        .get("_id")
                        .and_then(Value::as_object)
                        .is_some_and(|id| {
                            id.get("server").and_then(Value::as_str) == Some(server_id)
                                && id.get("user").and_then(Value::as_str) == Some(user_id)
                        })
                })
            });
        if is_member {
            Ok(())
        } else {
            Err("You are not a member of this server.".into())
        }
    }

    pub fn require_server_permission(
        &self,
        server_id: &str,
        required: u64,
    ) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "The authorization state is unavailable.")?;
        let ready = state
            .ready
            .as_ref()
            .ok_or("Permissions are unavailable until the gateway is ready.")?;
        let user = ready
            .get("currentUser")
            .ok_or("The authenticated user is unavailable.")?;
        if user
            .get("privileged")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let user_id = user
            .get("_id")
            .or_else(|| user.get("id"))
            .and_then(Value::as_str)
            .ok_or("The authenticated user is invalid.")?;
        let payload = ready
            .get("ready")
            .ok_or("The gateway permissions are unavailable.")?;
        let server = payload
            .get("servers")
            .and_then(Value::as_array)
            .and_then(|servers| {
                servers
                    .iter()
                    .find(|server| object_id(server) == Some(server_id))
            })
            .ok_or("The server is not available in the current permission snapshot.")?;
        if server.get("owner").and_then(Value::as_str) == Some(user_id) {
            return Ok(());
        }
        let member = payload
            .get("members")
            .and_then(Value::as_array)
            .and_then(|members| {
                members.iter().find(|member| {
                    member
                        .get("_id")
                        .and_then(Value::as_object)
                        .is_some_and(|id| {
                            id.get("server").and_then(Value::as_str) == Some(server_id)
                                && id.get("user").and_then(Value::as_str) == Some(user_id)
                        })
                })
            })
            .ok_or("You are not a member of this server.")?;
        if timeout_is_active(member.get("timeout").and_then(Value::as_str)) {
            return Err("Timed-out members cannot perform this action.".into());
        }
        let mut permissions = server
            .get("default_permissions")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let roles = server
            .get("roles")
            .and_then(Value::as_object)
            .ok_or("The server roles are unavailable.")?;
        let mut assigned = member
            .get("roles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|id| roles.get(id).map(|role| (id, role)))
            .collect::<Vec<_>>();
        assigned.sort_by_key(|(_, role)| {
            std::cmp::Reverse(role.get("rank").and_then(Value::as_i64).unwrap_or_default())
        });
        for (_, role) in assigned {
            if let Some(permission) = role.get("permissions") {
                permissions |= permission
                    .get("a")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                permissions &= !permission
                    .get("d")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
            }
        }
        if permissions & required == required {
            Ok(())
        } else {
            Err("You do not have permission to perform this action.".into())
        }
    }

    pub fn require_channel_permission(
        &self,
        channel_id: &str,
        required: u64,
    ) -> Result<(), String> {
        let (server_id, permissions) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "The authorization state is unavailable.")?;
            let payload = state
                .ready
                .as_ref()
                .and_then(|value| value.get("ready"))
                .ok_or("Permissions are unavailable until the gateway is ready.")?;
            let channel = payload
                .get("channels")
                .and_then(Value::as_array)
                .and_then(|channels| {
                    channels
                        .iter()
                        .find(|channel| object_id(channel) == Some(channel_id))
                })
                .ok_or("The channel is not available in the current permission snapshot.")?;
            let Some(server_id) = channel.get("server").and_then(Value::as_str) else {
                return Ok(());
            };
            let user = state
                .ready
                .as_ref()
                .and_then(|value| value.get("currentUser"))
                .ok_or("The authenticated user is unavailable.")?;
            if user
                .get("privileged")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(());
            }
            let user_id = user
                .get("_id")
                .or_else(|| user.get("id"))
                .and_then(Value::as_str)
                .ok_or("The authenticated user is invalid.")?;
            let server = payload
                .get("servers")
                .and_then(Value::as_array)
                .and_then(|servers| {
                    servers
                        .iter()
                        .find(|server| object_id(server) == Some(server_id))
                })
                .ok_or("The server is not available in the current permission snapshot.")?;
            if server.get("owner").and_then(Value::as_str) == Some(user_id) {
                return Ok(());
            }
            let member = payload
                .get("members")
                .and_then(Value::as_array)
                .and_then(|members| {
                    members.iter().find(|member| {
                        member
                            .get("_id")
                            .and_then(Value::as_object)
                            .is_some_and(|id| {
                                id.get("server").and_then(Value::as_str) == Some(server_id)
                                    && id.get("user").and_then(Value::as_str) == Some(user_id)
                            })
                    })
                })
                .ok_or("You are not a member of this server.")?;
            let mut permissions = server
                .get("default_permissions")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let apply = |permissions: &mut u64, value: Option<&Value>| {
                if let Some(value) = value {
                    *permissions |= value.get("a").and_then(Value::as_u64).unwrap_or_default();
                    *permissions &= !value.get("d").and_then(Value::as_u64).unwrap_or_default();
                }
            };
            apply(&mut permissions, channel.get("default_permissions"));
            let roles = server
                .get("roles")
                .and_then(Value::as_object)
                .ok_or("The server roles are unavailable.")?;
            let overrides = channel.get("role_permissions").and_then(Value::as_object);
            let mut assigned = member
                .get("roles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|id| roles.get(id).map(|role| (id, role)))
                .collect::<Vec<_>>();
            assigned.sort_by_key(|(_, role)| {
                std::cmp::Reverse(role.get("rank").and_then(Value::as_i64).unwrap_or_default())
            });
            for (_, role) in &assigned {
                apply(&mut permissions, role.get("permissions"));
            }
            for (id, _) in &assigned {
                apply(&mut permissions, overrides.and_then(|map| map.get(*id)));
            }
            if timeout_is_active(member.get("timeout").and_then(Value::as_str)) {
                permissions &= (1 << 20) | (1 << 21);
            }
            (server_id.to_owned(), permissions)
        };
        let _ = server_id;
        if permissions & (1 << 20) == 0 || permissions & required != required {
            return Err("You do not have permission to perform this action.".into());
        }
        Ok(())
    }

    pub(crate) fn send(&self, value: Value) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "The connection manager is unavailable.")?;
        state
            .runtime
            .as_ref()
            .ok_or("The gateway is not connected.")?
            .outgoing
            .send(value)
            .map_err(|_| "The gateway connection is unavailable.".into())
    }
}
