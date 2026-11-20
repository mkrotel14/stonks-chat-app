//! Match the Solid member projection: unique users, channel visibility,
//! connectivity-based presence, hoisted roles and highest-priority colors.
use std::collections::{BTreeMap, HashMap};
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientMember, ClientServer};

pub struct MemberEntry {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub bot: bool,
    pub presence: Option<String>,
    pub avatar_url: Option<String>,
    pub colour: Option<u32>,
}
pub struct MemberGroup {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub members: Vec<MemberEntry>,
}

fn can_view(
    server: &ClientServer,
    channel: Option<&ClientChannel>,
    user: &stonks_core::connection::ClientUser,
    member: &ClientMember,
) -> bool {
    if user.privileged || server.owner == user.id {
        return true;
    }
    let apply = |value: u64, overrides: &stonks_core::connection::PermissionOverride| {
        (value | overrides.a) & !overrides.d
    };
    let mut value = server.default_permissions;
    if let Some(overrides) = channel.and_then(|channel| channel.default_permissions.as_ref()) {
        value = apply(value, overrides);
    }
    let mut roles: Vec<_> = member
        .roles
        .iter()
        .filter_map(|id| server.roles.get(id).map(|role| (id, role)))
        .collect();
    roles.sort_by_key(|(_, role)| std::cmp::Reverse(role.rank.unwrap_or(0)));
    for (_, role) in &roles {
        value = apply(value, &role.permissions);
    }
    if let Some(channel) = channel {
        for (id, _) in roles {
            if let Some(overrides) = channel.role_permissions.get(id) {
                value = apply(value, overrides);
            }
        }
    }
    value & (1 << 20) != 0
}

pub fn groups(
    server: &ClientServer,
    channel: Option<&ClientChannel>,
    snapshot: &AppSnapshot,
    autumn: Option<&str>,
    api: Option<&str>,
) -> Vec<MemberGroup> {
    let members: BTreeMap<_, _> = snapshot
        .members
        .iter()
        .filter(|member| member.id.server == server.id)
        .map(|member| (member.id.user.as_str(), member))
        .collect();
    let users: HashMap<_, _> = snapshot
        .users
        .iter()
        .chain(std::iter::once(&snapshot.current_user))
        .map(|user| (user.id.as_str(), user))
        .collect();
    let mut result: BTreeMap<(u8, i64, String), MemberGroup> = BTreeMap::new();
    for (id, member) in members {
        let Some(user) = users.get(id) else {
            continue;
        };
        if !can_view(server, channel, user, member) {
            continue;
        }
        let name = member
            .nickname
            .clone()
            .or_else(|| user.display_name.clone())
            .unwrap_or_else(|| user.username.clone());
        let avatar_url = member
            .avatar
            .as_ref()
            .or(user.avatar.as_ref())
            .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id))
            .or_else(|| {
                stonks_core::media::file_url(api, "users", id)
                    .map(|url| format!("{url}/default_avatar"))
            });
        let mut roles: Vec<_> = member
            .roles
            .iter()
            .filter_map(|id| server.roles.get(id).map(|role| (id, role)))
            .collect();
        roles.sort_by_key(|(id, role)| (role.rank.unwrap_or(0), id.as_str()));
        let colour = roles.iter().find_map(|(_, role)| {
            role.colour.as_deref().and_then(|colour| {
                let value = colour.strip_prefix('#').unwrap_or(colour);
                (value.len() == 6)
                    .then(|| u32::from_str_radix(value, 16).ok())
                    .flatten()
            })
        });
        let hoisted = roles.iter().find(|(_, role)| role.hoist);
        let (key, label, icon_url) = match hoisted {
            Some((id, role)) => (
                (0, role.rank.unwrap_or(0), format!("role:{id}")),
                role.name.clone(),
                role.icon
                    .as_ref()
                    .and_then(|file| stonks_core::media::file_url(autumn, &file.tag, &file.id)),
            ),
            None if user.online => (
                (1, 0, "online".into()),
                stonks_ui::i18n::t("navigation.online"),
                None,
            ),
            None => (
                (2, 0, "offline".into()),
                stonks_ui::i18n::t("navigation.offline"),
                None,
            ),
        };
        let group = result.entry(key.clone()).or_insert_with(|| MemberGroup {
            id: key.2,
            name: label,
            icon_url,
            members: Vec::new(),
        });
        group.members.push(MemberEntry {
            id: id.into(),
            name,
            avatar_url,
            online: user.online,
            bot: user.bot.is_some(),
            presence: user
                .status
                .as_ref()
                .and_then(|status| status.presence.clone()),
            colour,
        });
    }
    result
        .into_values()
        .map(|mut group| {
            group.members.sort_by_key(|member| {
                (
                    !member.online,
                    member.name.to_lowercase(),
                    member.id.clone(),
                )
            });
            group
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (ClientServer, AppSnapshot) {
        let users: Vec<stonks_core::connection::ClientUser> = [
            serde_json::json!({"_id":"a","username":"Alice","online":true,"avatar":{"_id":"gif","tag":"avatars"}}),
            serde_json::json!({"_id":"b","username":"Bob","online":false,"status":{"presence":"Online"}}),
            serde_json::json!({"_id":"c","username":"Carol","online":true}),
        ].into_iter().map(|user|serde_json::from_value(user).unwrap()).collect();
        let members = [("a", "icl"), ("b", "sus"), ("a", "icl"), ("c", "icl")]
            .into_iter()
            .map(|(user, role)| {
                serde_json::from_value(
                    serde_json::json!({"_id":{"server":"s","user":user},"roles":[role]}),
                )
                .unwrap()
            })
            .collect();
        let server = serde_json::from_value(serde_json::json!({"_id":"s","name":"Server","owner":"a","default_permissions":1<<20,"roles":{
            "sus":{"name":"SUS","rank":1,"hoist":true},"icl":{"name":"ICL","rank":2,"hoist":true,"colour":"#3498db"}
        }})).unwrap();
        (
            server,
            AppSnapshot {
                current_user: users[0].clone(),
                users,
                servers: vec![],
                channels: vec![],
                members,
                unreads: vec![],
                voice_states: vec![],
            },
        )
    }
    #[test]
    fn roster_is_unique_role_grouped_and_offline_is_authoritative() {
        let (server, snapshot) = fixture();
        let result = groups(
            &server,
            None,
            &snapshot,
            Some("https://cdn.example.com"),
            None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "SUS");
        assert_eq!(result[0].members.len(), 1);
        assert!(!result[0].members[0].online);
        assert_eq!(result[1].name, "ICL");
        assert_eq!(result[1].members.len(), 2);
        assert_eq!(result[1].members[0].colour, Some(0x3498db));
        assert!(result[1].members[0]
            .avatar_url
            .as_ref()
            .unwrap()
            .ends_with("/original"));
    }
    #[test]
    fn channel_permission_overrides_filter_hidden_members() {
        let (server, snapshot) = fixture();
        let channel=serde_json::from_value(serde_json::json!({"_id":"channel","channel_type":"TextChannel","server":"s","default_permissions":{"a":0,"d":1<<20},"role_permissions":{"icl":{"a":1<<20,"d":0}}})).unwrap();
        let result = groups(&server, Some(&channel), &snapshot, None, None);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0]
                .members
                .iter()
                .map(|member| member.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "c"]
        );
    }
}
