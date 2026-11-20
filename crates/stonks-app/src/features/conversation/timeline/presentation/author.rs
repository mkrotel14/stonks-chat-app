//! Source MessageRow header metadata. Member pronouns override global values;
//! the highest-ranked role with an icon supplies the17px role image.
use gpui_kit::*;
use std::sync::Arc;
use stonks_core::connection::AppSnapshot;
use stonks_ui::Theme;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Header {
    pub pronouns: Option<String>,
    pub role_icon_url: Option<String>,
    pub bot: bool,
}
pub(crate) fn view(
    theme: Theme,
    name: String,
    header: Header,
    role_icon: Option<Arc<RenderImage>>,
    timestamp: String,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .min_h(px(23.))
        .items_baseline()
        .gap(px(9.))
        .child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(15.))
                        .text_color(theme.text)
                        .text_ellipsis()
                        .child(name),
                )
                .children(
                    header
                        .bot
                        .then(|| stonks_ui::components::BotBadge::new(theme)),
                ),
        )
        .children(header.role_icon_url.is_some().then(|| {
            div()
                .size(px(17.))
                .flex_shrink_0()
                .self_center()
                .rounded(px(theme.radius_pill))
                .children(role_icon.map(|image| {
                    img(image)
                        .size_full()
                        .rounded(px(theme.radius_pill))
                        .object_fit(ObjectFit::Cover)
                }))
        }))
        .children(header.pronouns.map(|pronouns| {
            div()
                .text_size(px(theme.font_size_xs))
                .text_color(theme.text_muted)
                .child(pronouns)
        }))
        .child(
            div()
                .text_size(px(theme.font_size_xs))
                .text_color(theme.text_muted)
                .child(timestamp),
        )
}
pub(crate) fn header(
    snapshot: &AppSnapshot,
    channel: &str,
    author: &str,
    autumn: Option<&str>,
) -> Header {
    let user = snapshot
        .users
        .iter()
        .chain(std::iter::once(&snapshot.current_user))
        .find(|user| user.id == author);
    let server_id = snapshot
        .channels
        .iter()
        .find(|entry| entry.id == channel)
        .and_then(|entry| entry.server.as_deref());
    let member = server_id.and_then(|server| {
        snapshot
            .members
            .iter()
            .find(|member| member.id.server == server && member.id.user == author)
    });
    let server = server_id.and_then(|id| snapshot.servers.iter().find(|server| server.id == id));
    let role = server.zip(member).and_then(|(server, member)| {
        member
            .roles
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                server
                    .roles
                    .get(id)
                    .filter(|role| role.icon.is_some())
                    .map(|role| (index, role))
            })
            .min_by_key(|(index, role)| (role.rank.unwrap_or(0), *index))
            .map(|(_, role)| role)
    });
    Header {
        pronouns: member
            .and_then(|member| member.pronouns.clone())
            .or_else(|| user.and_then(|user| user.pronouns.clone()))
            .filter(|value| !value.is_empty()),
        role_icon_url: role
            .and_then(|role| role.icon.as_ref())
            .and_then(|icon| stonks_core::media::file_url(autumn, &icon.tag, &icon.id)),
        bot: user
            .and_then(|user| user.bot.as_ref())
            .is_some_and(|value| !value.is_null() && value != &serde_json::Value::Bool(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::header;
    #[test]
    fn headers_use_member_pronouns_highest_icon_role_and_global_bot_identity() {
        let mut snapshot=stonks_core::connection::AppSnapshot {
            current_user:serde_json::from_value(serde_json::json!({"_id":"self","username":"Self"})).unwrap(),
            users:vec![serde_json::from_value(serde_json::json!({"_id":"author","username":"Author","pronouns":"global","bot":{"owner":"self"}})).unwrap()],
            servers:vec![serde_json::from_value(serde_json::json!({"_id":"server","name":"Server","owner":"self","roles":{"no-icon":{"name":"No icon","rank":-1},"low":{"name":"Low","rank":5,"icon":{"_id":"low-icon","tag":"icons"}},"high":{"name":"High","rank":1,"icon":{"_id":"high-icon","tag":"icons"}}}})).unwrap()],
            channels:vec![serde_json::from_value(serde_json::json!({"_id":"channel","channel_type":"TextChannel","server":"server"})).unwrap()],
            members:vec![serde_json::from_value(serde_json::json!({"_id":{"server":"server","user":"author"},"pronouns":"member","roles":["low","no-icon","high"]})).unwrap()],
            unreads:vec![],voice_states:vec![],
        };
        let shown = header(
            &snapshot,
            "channel",
            "author",
            Some("https://cdn.example.test/files"),
        );
        assert_eq!(shown.pronouns.as_deref(), Some("member"));
        assert_eq!(
            shown.role_icon_url.as_deref(),
            Some("https://cdn.example.test/files/icons/high-icon")
        );
        assert!(shown.bot);
        snapshot.members[0].pronouns = None;
        assert_eq!(
            header(&snapshot, "channel", "author", None)
                .pronouns
                .as_deref(),
            Some("global")
        );
        snapshot.members[0].pronouns = Some("".into());
        assert_eq!(header(&snapshot, "channel", "author", None).pronouns, None);
        assert_eq!(
            header(&snapshot, "channel", "missing", None),
            super::Header::default()
        );
    }
}
