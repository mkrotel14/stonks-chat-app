use icu_normalizer::ComposingNormalizer;
use std::{collections::HashMap, ops::Range};
use stonks_core::connection::{AppSnapshot, ClientChannel, ClientFileReference};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    User,
    Role,
    Mass,
    Music,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub kind: Kind,
    pub label: String,
    pub detail: Option<String>,
    pub token: String,
    pub image: Option<String>,
    pub colour: Option<String>,
}

impl Candidate {
    fn new(
        kind: Kind,
        label: impl Into<String>,
        token: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            token: token.into(),
            detail,
            image: None,
            colour: None,
        }
    }
    fn music() -> Self {
        Self::new(
            Kind::Music,
            "@Music",
            "@Music",
            Some("Play an audio link or absolute local file path in your voice channel".into()),
        )
    }
    pub fn display(&self) -> String {
        if self.kind == Kind::Role {
            format!("@{}", self.label)
        } else {
            self.label.clone()
        }
    }
}

pub(super) fn candidates(
    channel: &ClientChannel,
    snapshot: &AppSnapshot,
    autumn: Option<&str>,
    everyone: bool,
    roles: bool,
) -> Vec<Candidate> {
    let server = channel
        .server
        .as_ref()
        .and_then(|id| snapshot.servers.iter().find(|server| &server.id == id));
    let current = &snapshot.current_user;
    let users: HashMap<_, _> = snapshot
        .users
        .iter()
        .chain([current])
        .map(|user| (user.id.as_str(), user))
        .collect();
    let members: HashMap<_, _> = snapshot
        .members
        .iter()
        .filter(|member| server.is_some_and(|server| member.id.server == server.id))
        .map(|member| (member.id.user.as_str(), member))
        .collect();
    let ids = if channel.server.is_some() {
        members
            .keys()
            .copied()
            .chain([current.id.as_str()])
            .collect()
    } else if channel.channel_type == "SavedMessages" {
        vec![current.id.as_str()]
    } else {
        channel
            .recipients
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    };
    let image = |file: Option<&ClientFileReference>| {
        file.and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id))
    };
    let mut result = vec![Candidate::music()];
    if server.is_some() && everyone {
        for (label, detail) in [
            ("@everyone", "Notify everyone who can see this channel"),
            ("@online", "Notify online members who can see this channel"),
        ] {
            result.push(Candidate::new(
                Kind::Mass,
                label,
                label,
                Some(detail.into()),
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids.into_iter().filter(|id| seen.insert(*id)) {
        let Some(user) = users.get(id) else { continue };
        let display = members
            .get(id)
            .and_then(|member| member.nickname.as_deref())
            .or(user.display_name.as_deref())
            .unwrap_or(&user.username);
        let detail = (display != user.username).then(|| {
            user.discriminator
                .as_deref()
                .filter(|value| !value.is_empty())
                .map_or_else(
                    || user.username.clone(),
                    |suffix| format!("{}#{suffix}", user.username),
                )
        });
        let mut candidate = Candidate::new(
            Kind::User,
            format!("@{display}"),
            format!("<@{id}>"),
            detail,
        );
        candidate.image = image(user.avatar.as_ref());
        result.push(candidate);
    }
    if let Some(server) = server.filter(|_| roles) {
        for (id, role) in &server.roles {
            let mut candidate = Candidate::new(
                Kind::Role,
                &role.name,
                format!("<%{id}>"),
                Some("Server role".into()),
            );
            candidate.image = role
                .icon
                .as_ref()
                .and_then(|file| stonks_core::media::file_url(autumn, &file.tag, &file.id));
            candidate.colour = role.colour.clone();
            result.push(candidate);
        }
    }
    result.sort_unstable_by(|a, b| a.token.cmp(&b.token));
    result
}

pub(super) fn complete<'a>(
    text: &str,
    caret: usize,
    candidates: &'a [Candidate],
) -> Option<(Range<usize>, Vec<&'a Candidate>)> {
    let prefix = text.get(..caret)?;
    let (at, trigger) = prefix.rmatch_indices(['@', '%']).next()?;
    let left = &prefix[..at];
    if !left.is_empty() && !left.ends_with(char::is_whitespace) {
        return None;
    }
    let query = &prefix[at + 1..];
    if query.chars().count() > 64 || query.contains(char::is_whitespace) {
        return None;
    }
    let normalize = |text: &str| {
        ComposingNormalizer::new_nfkc()
            .normalize(text)
            .to_lowercase()
    };
    let query = normalize(query);
    let mut matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| trigger != "%" || candidate.kind == Kind::Role)
        .filter_map(|candidate| {
            let label = normalize(&candidate.label);
            let name = if candidate.kind == Kind::Role {
                label.as_str()
            } else {
                label.strip_prefix('@').unwrap_or(&label)
            };
            let detail = candidate.detail.as_deref().unwrap_or_default();
            format!("{label} {}", normalize(detail))
                .contains(&query)
                .then_some((
                    (
                        candidate.kind != Kind::Music,
                        !name.starts_with(&query),
                        label,
                    ),
                    candidate,
                ))
        })
        .collect();
    matches.sort_by(|(a, _), (b, _)| a.cmp(b));
    (!matches.is_empty()).then(|| {
        (
            at..caret,
            matches
                .into_iter()
                .take(8)
                .map(|(_, candidate)| candidate)
                .collect(),
        )
    })
}

#[derive(Debug)]
pub(in super::super) struct DraftMention {
    pub range: Range<usize>,
    pub label: String,
    pub token: String,
}

pub(in super::super) fn reconcile(previous: &str, next: &str, mentions: &mut Vec<DraftMention>) {
    if previous == next || mentions.is_empty() {
        return;
    }
    let prefix: usize = previous
        .chars()
        .zip(next.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = previous[prefix..]
        .chars()
        .rev()
        .zip(next[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    replace_ranges(
        mentions,
        prefix..previous.len() - suffix,
        next.len() - prefix - suffix,
    );
}

pub(in super::super) fn replace_ranges(
    mentions: &mut Vec<DraftMention>,
    replaced: Range<usize>,
    inserted_len: usize,
) {
    mentions.retain_mut(|mention| {
        if mention.range.end <= replaced.start {
            return true;
        }
        if mention.range.start < replaced.end {
            return false;
        }
        let new_end = replaced.start + inserted_len;
        mention.range = (mention.range.start - replaced.end + new_end)
            ..(mention.range.end - replaced.end + new_end);
        true
    });
}

pub(in super::super) fn serialize(
    text: &str,
    mentions: &[DraftMention],
    mut plain: impl FnMut(&str) -> String,
) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    for mention in mentions {
        if mention.range.start >= cursor && text.get(mention.range.clone()) == Some(&mention.label)
        {
            output.push_str(&plain(&text[cursor..mention.range.start]));
            output.push_str(&mention.token);
            cursor = mention.range.end;
        }
    }
    output.push_str(&plain(&text[cursor..]));
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> AppSnapshot {
        AppSnapshot {
            current_user: serde_json::from_value(json!({"_id": "self", "username": "self"})).unwrap(),
            users: serde_json::from_value(json!([
                {"_id": "person", "username": "alice", "display_name": "Al", "discriminator": "1234"},
                {"_id": "outsider", "username": "other"}
            ])).unwrap(),
            servers: serde_json::from_value(json!([
                {"_id": "server", "name": "Server", "owner": "self", "roles": {"role": {"name": "Moderators"}}}
            ])).unwrap(),
            members: serde_json::from_value(json!([
                {"_id": {"server": "server", "user": "person"}, "nickname": "Álice"},
                {"_id": {"server": "elsewhere", "user": "outsider"}}
            ])).unwrap(),
            channels: Vec::new(),
            unreads: Vec::new(),
            voice_states: Vec::new(),
        }
    }

    #[test]
    fn scope_and_permissions_match_server_dm_and_saved_messages() {
        let snapshot = snapshot();
        let mut channel: ClientChannel = serde_json::from_value(
            json!({"_id": "channel", "channel_type": "TextChannel", "server": "server"}),
        )
        .unwrap();
        let tokens = |channel: &ClientChannel, everyone, roles| {
            candidates(channel, &snapshot, None, everyone, roles)
                .into_iter()
                .map(|candidate| candidate.token)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            tokens(&channel, false, false),
            ["<@person>", "<@self>", "@Music"]
        );
        assert_eq!(
            tokens(&channel, true, false),
            ["<@person>", "<@self>", "@Music", "@everyone", "@online"]
        );
        assert_eq!(
            tokens(&channel, false, true),
            ["<%role>", "<@person>", "<@self>", "@Music"]
        );
        channel.server = None;
        channel.channel_type = "DirectMessage".into();
        channel.recipients = vec!["person".into()];
        assert_eq!(tokens(&channel, true, true), ["<@person>", "@Music"]);
        channel.channel_type = "SavedMessages".into();
        assert_eq!(tokens(&channel, true, true), ["<@self>", "@Music"]);
    }

    #[test]
    fn completion_uses_the_caret_and_correct_trigger() {
        let snapshot = snapshot();
        let channel = serde_json::from_value(
            json!({"_id": "channel", "channel_type": "TextChannel", "server": "server"}),
        )
        .unwrap();
        let candidates = candidates(&channel, &snapshot, None, true, true);
        let text = "🦀 @ａｌｉｃｅ suffix";
        let caret = text.find(" suffix").unwrap();
        let (range, matches) = complete(text, caret, &candidates).unwrap();
        assert_eq!(&text[range], "@ａｌｉｃｅ");
        assert_eq!(matches[0].display(), "@Álice");
        assert!(complete(text, text.len(), &candidates).is_none());
        assert!(complete("hello@alice", 11, &candidates).is_none());
        for trigger in ['@', '%'] {
            let text = format!("{trigger}mod");
            let (range, matches) = complete(&text, text.len(), &candidates).unwrap();
            assert_eq!(range, 0..text.len());
            assert_eq!(matches[0].display(), "@Moderators");
            assert_eq!(matches[0].token, "<%role>");
        }
        assert!(complete("%alice", 6, &candidates).is_none());
        assert_eq!(
            complete("@", 1, &candidates).unwrap().1[0].display(),
            "@Music"
        );
    }

    #[test]
    fn wire_spans_shift_with_unicode_edits_and_only_serialize_selected_labels() {
        let previous = "@Álice @Moderators @everyone @Music @Álice";
        let span = |label: &str, token: &str| {
            let start = previous.find(label).unwrap();
            DraftMention {
                range: start..start + label.len(),
                label: label.into(),
                token: token.into(),
            }
        };
        let mut spans = vec![
            span("@Álice", "<@person>"),
            span("@Moderators", "<%role>"),
            span("@everyone", "@everyone"),
        ];
        let next = format!(" 🦀 {previous}");
        reconcile(previous, &next, &mut spans);
        assert_eq!(
            serialize(&next, &spans, str::to_owned),
            " 🦀 <@person> <%role> @everyone @Music @Álice"
        );
        let edited = next.replacen("@Álice", "@Alice", 1);
        reconcile(&next, &edited, &mut spans);
        assert_eq!(
            serialize(&edited, &spans, str::to_owned),
            " 🦀 @Alice <%role> @everyone @Music @Álice"
        );
    }
}
