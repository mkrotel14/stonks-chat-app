//! Reaction identities, optimistic membership and authoritative gateway reconciliation.
use crate::{
    session::{Session, SessionEvent},
    shared::images::Images,
};
use crate::runtime::spawn_core;
use futures_util::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use gpui_kit::*;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use stonks_core::{
    connection::{AppSnapshot, GatewayEvent},
    http::HttpPolicy,
    messages::{
        models::{ReactionRequest, TimelineMessage, TimelineReaction},
        reactions as service,
    },
    server_settings::ServerEmoji,
};
use stonks_ui::{
    components::{
        emoji::{emoji_shortcode, ReactionPerson},
        ReactionPill,
    },
    i18n::t,
    Theme,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub avatar: Option<String>,
}
pub fn person(
    snapshot: &AppSnapshot,
    server: Option<&str>,
    id: &str,
    autumn: Option<&str>,
) -> Person {
    let member = snapshot
        .members
        .iter()
        .find(|member| Some(member.id.server.as_str()) == server && member.id.user == id);
    let user = if snapshot.current_user.id == id {
        Some(&snapshot.current_user)
    } else {
        snapshot.users.iter().find(|user| user.id == id)
    };
    let name = if snapshot.current_user.id == id {
        t("messages.you")
    } else {
        member
            .and_then(|member| member.nickname.clone())
            .or_else(|| user.and_then(|user| user.display_name.clone()))
            .or_else(|| user.map(|user| user.username.clone()))
            .unwrap_or_else(|| t("messages.unknownUser"))
    };
    let avatar = member
        .and_then(|member| member.avatar.as_ref())
        .or_else(|| user.and_then(|user| user.avatar.as_ref()))
        .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id));
    Person { name, avatar }
}
pub fn set_membership(
    reactions: &mut Vec<TimelineReaction>,
    emoji: &str,
    user: &str,
    present: bool,
) {
    if let Some(reaction) = reactions
        .iter_mut()
        .find(|reaction| reaction.emoji == emoji)
    {
        if present {
            if !reaction.user_ids.iter().any(|id| id == user) {
                reaction.user_ids.push(user.to_owned());
            }
        } else {
            reaction.user_ids.retain(|id| id != user);
        }
    } else if present {
        reactions.push(TimelineReaction {
            emoji: emoji.into(),
            user_ids: vec![user.into()],
        });
    }
    reactions.retain(|reaction| !reaction.user_ids.is_empty());
}
fn field<'a>(data: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| data.get(key).and_then(Value::as_str))
}
pub fn event_channel(event: &GatewayEvent) -> Option<&str> {
    field(&event.data, &["channel", "channel_id", "channelId"])
}
pub fn apply_event(message: &mut TimelineMessage, event: &GatewayEvent) -> bool {
    if event_channel(event) != Some(message.channel_id.as_str())
        || field(&event.data, &["id"]) != Some(message.id.as_str())
    {
        return false;
    }
    match event.event_type.as_str() {
        "MessageReact" | "MessageUnreact" => {
            let (Some(emoji), Some(user)) = (
                field(&event.data, &["emoji", "emoji_id", "emojiId"]),
                field(&event.data, &["user", "user_id", "userId"]),
            ) else {
                return false;
            };
            set_membership(
                &mut message.reactions,
                emoji,
                user,
                event.event_type == "MessageReact",
            );
            true
        }
        "MessageRemoveReaction" => {
            let Some(emoji) = field(&event.data, &["emoji", "emoji_id", "emojiId"]) else {
                return false;
            };
            message.reactions.retain(|reaction| reaction.emoji != emoji);
            true
        }
        "MessageUpdate" => {
            if let Some(reactions) = event
                .data
                .get("data")
                .and_then(|data| data.get("reactions"))
                .and_then(Value::as_object)
            {
                message.reactions = reactions
                    .iter()
                    .map(|(emoji, users)| TimelineReaction {
                        emoji: emoji.clone(),
                        user_ids: users
                            .as_array()
                            .map(|users| {
                                users
                                    .iter()
                                    .filter_map(|user| user.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .filter(|reaction| !reaction.user_ids.is_empty())
                    .collect();
                return true;
            }
            if event
                .data
                .get("clear")
                .and_then(Value::as_array)
                .is_some_and(|clear| {
                    clear
                        .iter()
                        .any(|value| value.as_str() == Some("Reactions"))
                })
            {
                message.reactions.clear();
                return true;
            }
            false
        }
        _ => false,
    }
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    message: String,
    emoji: String,
}
struct Work {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
pub enum ReactionEvent {
    MetadataChanged,
    Membership {
        channel: String,
        message: String,
        emoji: String,
        user: String,
        present: bool,
    },
    Failed {
        channel: String,
        error: String,
    },
}
pub struct Reactions {
    session: Entity<Session>,
    policy: Arc<HttpPolicy>,
    channel: Option<String>,
    identity: Option<(String, String)>,
    generation: u64,
    pending: HashMap<Key, Work>,
    revisions: HashMap<(String, String, String), u64>,
    emojis: HashMap<String, (String, String)>,
    emoji_order: Vec<String>,
    emoji_request: Option<Work>,
    emoji_scope: Option<(Vec<String>, Option<String>, bool)>,
    metadata_generation: u64,
    _gateway: Subscription,
    _session: Subscription,
}
impl EventEmitter<ReactionEvent> for Reactions {}
fn identity(session: &Session) -> Option<(String, String)> {
    session
        .snapshot
        .as_ref()
        .zip(session.server.as_ref())
        .map(|(snapshot, server)| {
            (
                snapshot.current_user.id.clone(),
                server.deployment_url.clone(),
            )
        })
}
impl Reactions {
    pub fn new(session: Entity<Session>, policy: Arc<HttpPolicy>, cx: &mut Context<Self>) -> Self {
        let initial = identity(session.read(cx));
        let gateway = cx.subscribe(&session, |this, _, event, cx| {
            let SessionEvent::Gateway(event) = event;
            this.authoritative(event);
            if matches!(
                event.event_type.as_str(),
                "ServerEmojiCreate" | "ServerEmojiDelete"
            ) {
                this.fetch_emojis(cx);
            }
        });
        let observer = cx.observe(&session, |this, session, cx| {
            let next = identity(session.read(cx));
            if next != this.identity {
                this.identity = next;
                this.reset();
                this.channel = None;
            }
            if this.metadata_scope(cx) != this.emoji_scope {
                this.fetch_emojis(cx);
            }
            cx.notify();
        });
        Self {
            session,
            policy,
            channel: None,
            identity: initial,
            generation: 0,
            pending: HashMap::new(),
            revisions: HashMap::new(),
            emojis: HashMap::new(),
            emoji_order: Vec::new(),
            emoji_request: None,
            emoji_scope: None,
            metadata_generation: 0,
            _gateway: gateway,
            _session: observer,
        }
    }
    fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending.clear();
        self.revisions.clear();
        self.emoji_request = None;
        self.metadata_generation = self.metadata_generation.wrapping_add(1);
        self.emoji_scope = None;
        self.emojis.clear();
        self.emoji_order.clear();
    }
    pub fn set_channel(&mut self, channel: Option<String>, cx: &mut Context<Self>) {
        if self.channel != channel {
            self.reset();
            self.channel = channel;
            self.fetch_emojis(cx);
            cx.notify();
        }
    }
    fn server(&self, cx: &App) -> Option<String> {
        let session = self.session.read(cx);
        session
            .snapshot
            .as_ref()?
            .channels
            .iter()
            .find(|channel| Some(&channel.id) == self.channel.as_ref())?
            .server
            .clone()
    }
    fn metadata_scope(&self, cx: &App) -> Option<(Vec<String>, Option<String>, bool)> {
        let channel = self.channel.as_deref()?;
        let session = self.session.read(cx);
        let snapshot = session.snapshot.as_ref()?;
        let server = self.server(cx);
        let external = session.can_channel_permission(channel, service::USE_EXTERNAL_EMOJIS);
        let mut servers = server.iter().cloned().collect::<Vec<_>>();
        if server.is_none() || external {
            for entry in &snapshot.servers {
                if !servers.contains(&entry.id) {
                    servers.push(entry.id.clone());
                }
            }
        }
        Some((servers, server, external))
    }
    pub fn invalidate_emojis(&mut self, cx: &mut Context<Self>) {
        self.fetch_emojis(cx);
    }
    fn fetch_emojis(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.metadata_scope(cx) else {
            return;
        };
        let servers = scope.0.clone();
        self.emoji_scope = Some(scope);
        self.emojis.clear();
        self.emoji_order.clear();
        cx.emit(ReactionEvent::MetadataChanged);
        cx.notify();
        self.metadata_generation = self.metadata_generation.wrapping_add(1);
        let metadata_generation = self.metadata_generation;
        let policy = self.policy.clone();
        let generation = self.generation;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let Ok(mut groups) = spawn_core(Abortable::new(
                async move {
                    futures_util::stream::iter(servers.into_iter().enumerate())
                        .map(|(order, server)| {
                            let policy = policy.clone();
                            async move {
                                stonks_core::server_settings::fetch_emojis(&policy, &server)
                                    .await
                                    .ok()
                                    .map(|emojis| (order, server, emojis))
                            }
                        })
                        .buffer_unordered(4)
                        .filter_map(|group| async move { group })
                        .collect::<Vec<_>>()
                        .await
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation || this.metadata_generation != metadata_generation
                {
                    return;
                }
                groups.sort_by_key(|(order, _, _)| *order);
                this.emojis.clear();
                this.emoji_order.clear();
                for (_, server, emojis) in groups {
                    for ServerEmoji { id, name, .. } in emojis {
                        this.emoji_order.push(id.clone());
                        this.emojis.insert(id, (name, server.clone()));
                    }
                }
                cx.emit(ReactionEvent::MetadataChanged);
                cx.notify();
            })
            .ok();
        });
        self.emoji_request = Some(Work { abort, _task: task });
    }
    fn authoritative(&mut self, event: &GatewayEvent) {
        if event_channel(event) != self.channel.as_deref() {
            return;
        }
        let Some(message) = field(&event.data, &["id"]) else {
            return;
        };
        if !self.pending.keys().any(|key| key.message == message) {
            return;
        }
        let key = match event.event_type.as_str() {
            "MessageReact" | "MessageUnreact" => {
                let (Some(emoji), Some(user)) = (
                    field(&event.data, &["emoji", "emoji_id", "emojiId"]),
                    field(&event.data, &["user", "user_id", "userId"]),
                ) else {
                    return;
                };
                if self.identity.as_ref().map(|(id, _)| id.as_str()) != Some(user)
                    || !self.pending.contains_key(&Key {
                        message: message.into(),
                        emoji: emoji.into(),
                    })
                {
                    return;
                }
                (message.to_owned(), emoji.to_owned(), user.to_owned())
            }
            "MessageRemoveReaction" => {
                let Some(emoji) = field(&event.data, &["emoji", "emoji_id", "emojiId"]) else {
                    return;
                };
                (message.to_owned(), emoji.to_owned(), "*".into())
            }
            "MessageDelete" => (message.to_owned(), "*".into(), "*".into()),
            "MessageUpdate"
                if event
                    .data
                    .get("data")
                    .is_some_and(|data| data.get("reactions").is_some())
                    || event
                        .data
                        .get("clear")
                        .and_then(Value::as_array)
                        .is_some_and(|clear| {
                            clear
                                .iter()
                                .any(|value| value.as_str() == Some("Reactions"))
                        }) =>
            {
                (message.to_owned(), "*".into(), "*".into())
            }
            _ => return,
        };
        let revision = self.revisions.entry(key).or_default();
        *revision = revision.wrapping_add(1);
    }
    fn revision(&self, message: &str, emoji: &str, user: &str) -> [u64; 3] {
        [
            self.revisions
                .get(&(message.into(), emoji.into(), user.into()))
                .copied()
                .unwrap_or(0),
            self.revisions
                .get(&(message.into(), emoji.into(), "*".into()))
                .copied()
                .unwrap_or(0),
            self.revisions
                .get(&(message.into(), "*".into(), "*".into()))
                .copied()
                .unwrap_or(0),
        ]
    }
    fn can_use(&self, emoji: &str, removing: bool, cx: &App) -> bool {
        let Some(channel) = self.channel.as_deref() else {
            return false;
        };
        let session = self.session.read(cx);
        session.can_channel_permission(channel, service::REACT)
            && (removing
                || !stonks_core::messages::normalize::valid_id(emoji)
                || self.server(cx).is_none()
                || session.can_channel_permission(channel, service::USE_EXTERNAL_EMOJIS)
                || self
                    .emojis
                    .get(emoji)
                    .is_some_and(|(_, server)| Some(server) == self.server(cx).as_ref()))
    }
    pub fn can_add(&self, emoji: &str, cx: &App) -> bool {
        self.can_use(emoji, false, cx)
    }

    pub fn picker_catalog(&self, cx: &App) -> Vec<stonks_ui::components::CustomEmoji> {
        let session = self.session.read(cx);
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        self.emoji_order
            .iter()
            .filter(|id| self.can_use(id, false, cx))
            .filter_map(|id| {
                let (name, server) = self.emojis.get(id)?;
                Some(stonks_ui::components::CustomEmoji {
                    id: id.clone(),
                    name: name.clone(),
                    server_name: session
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot.servers.iter().find(|entry| &entry.id == server)
                        })
                        .map(|entry| entry.name.clone()),
                    url: stonks_core::media::file_url(autumn, "emojis", id),
                })
            })
            .collect()
    }

    /// Add a picker-selected emoji using the same pending and permission checks.
    #[allow(dead_code)] // Entry point for the later M4 add-reaction picker.
    pub fn add(&mut self, message: String, emoji: String, cx: &mut Context<Self>) {
        self.toggle(message, emoji, false, cx);
    }
    pub fn resolve_custom(&self, id: Option<&str>, name: Option<&str>) -> Option<(String, String)> {
        if let Some(id) = id {
            return Some((
                id.to_owned(),
                name.map(str::to_owned)
                    .or_else(|| self.emojis.get(id).map(|(name, _)| name.clone()))
                    .unwrap_or_else(|| "custom emoji".into()),
            ));
        }
        let name = name?;
        self.emoji_order.iter().find_map(|id| {
            self.emojis
                .get(id)
                .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                .map(|(name, _)| (id.clone(), name.clone()))
        })
    }
    fn toggle(&mut self, message: String, emoji: String, removing: bool, cx: &mut Context<Self>) {
        let key = Key {
            message: message.clone(),
            emoji: emoji.clone(),
        };
        if self.pending.contains_key(&key) || !self.can_use(&emoji, removing, cx) {
            return;
        }
        let Some(channel) = self.channel.clone() else {
            return;
        };
        let Some((user, _)) = self.identity.clone() else {
            return;
        };
        let revision = self.revision(&message, &emoji, &user);
        let generation = self.generation;
        cx.emit(ReactionEvent::Membership {
            channel: channel.clone(),
            message: message.clone(),
            emoji: emoji.clone(),
            user: user.clone(),
            present: !removing,
        });
        let policy = self.policy.clone();
        let authorization = self.session.read(cx).authorization();
        let (abort, registration) = AbortHandle::new_pair();
        let task_key = key.clone();
        let task = cx.spawn(async move |this, cx| {
            let request = ReactionRequest {
                channel_id: channel.clone(),
                message_id: message.clone(),
                emoji: emoji.clone(),
            };
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    service::set_membership(&policy, &authorization, &request, !removing).await
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending.remove(&task_key);
                if let Err(error) = result {
                    if this.revision(&message, &emoji, &user) == revision {
                        cx.emit(ReactionEvent::Membership {
                            channel: channel.clone(),
                            message,
                            emoji,
                            user,
                            present: removing,
                        });
                        cx.emit(ReactionEvent::Failed {
                            channel,
                            error: error.to_string(),
                        });
                    }
                }
                if !this
                    .pending
                    .keys()
                    .any(|pending| pending.message == task_key.message)
                {
                    this.revisions
                        .retain(|(message, _, _), _| message != &task_key.message);
                }
                cx.notify();
            })
            .ok();
        });
        self.pending.insert(key, Work { abort, _task: task });
        cx.notify();
    }
    pub fn avatar_sources(
        &self,
        ids: impl Iterator<Item = String>,
        cx: &App,
    ) -> Vec<(String, u32)> {
        let session = self.session.read(cx);
        let Some(snapshot) = session.snapshot.as_ref() else {
            return vec![];
        };
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let server = self.server(cx);
        ids.filter_map(|id| {
            person(snapshot, server.as_deref(), &id, autumn)
                .avatar
                .map(|url| (url, 48))
        })
        .collect()
    }
    pub fn pill(
        this: &Entity<Self>,
        message: &TimelineMessage,
        index: usize,
        images: &Entity<Images>,
        theme: Theme,
        cx: &mut App,
    ) -> ReactionPill {
        let reaction = &message.reactions[index];
        let state = this.read(cx);
        let session = state.session.read(cx);
        let snapshot = session.snapshot.as_ref();
        let server = state.server(cx);
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let people = reaction
            .user_ids
            .iter()
            .map(|id| {
                snapshot
                    .map(|snapshot| person(snapshot, server.as_deref(), id, autumn))
                    .unwrap_or(Person {
                        name: t("messages.unknownUser"),
                        avatar: None,
                    })
            })
            .collect::<Vec<_>>();
        let user = state
            .identity
            .as_ref()
            .map(|(id, _)| id.as_str())
            .unwrap_or("");
        let removing = reaction.user_ids.iter().any(|id| id == user);
        let disabled = state.pending.contains_key(&Key {
            message: message.id.clone(),
            emoji: reaction.emoji.clone(),
        }) || !state.can_use(&reaction.emoji, removing, cx);
        let code = state
            .emojis
            .get(&reaction.emoji)
            .map(|(name, _)| format!(":{name}:"))
            .unwrap_or_else(|| emoji_shortcode(&reaction.emoji));
        let label = format!(
            "{code}, reacted by {}",
            people
                .iter()
                .map(|person| person.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let custom = stonks_core::messages::normalize::valid_id(&reaction.emoji)
            .then(|| stonks_core::media::file_url(autumn, "emojis", &reaction.emoji))
            .flatten();
        let custom = images.update(cx, |images, cx| images.get(custom.as_deref(), 40, cx));
        let avatar_urls = people
            .iter()
            .map(|person| person.avatar.clone())
            .collect::<Vec<_>>();
        let avatar_images = images.clone();
        let people = people
            .into_iter()
            .map(|person| ReactionPerson {
                name: person.name.into(),
                avatar: None,
            })
            .collect();
        let target = this.downgrade();
        let message_id = message.id.clone();
        let emoji = reaction.emoji.clone();
        ReactionPill::new(
            ElementId::Name(format!("reaction-{}-{}", message.id, reaction.emoji).into()),
            theme,
            reaction.emoji.clone(),
            reaction.user_ids.len(),
        )
        .selected(removing)
        .custom_image(custom)
        .tooltip(label)
        .shortcode(code)
        .people(people)
        .avatar_resolver(move |index, cx| {
            avatar_images.update(cx, |images, cx| {
                images.get(avatar_urls.get(index).and_then(Option::as_deref), 48, cx)
            })
        })
        .disabled(disabled)
        .on_click(move |_, _, cx| {
            target
                .update(cx, |this, cx| {
                    this.toggle(message_id.clone(), emoji.clone(), removing, cx)
                })
                .ok();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_event, person, set_membership};
    use stonks_core::{
        connection::{AppSnapshot, GatewayEvent},
        messages::models::TimelineReaction,
    };
    #[test]
    fn membership_is_idempotent_and_peer_reactions_survive_rollback() {
        let mut values = vec![TimelineReaction {
            emoji: "💀".into(),
            user_ids: vec!["peer".into()],
        }];
        set_membership(&mut values, "💀", "me", true);
        set_membership(&mut values, "💀", "me", true);
        set_membership(&mut values, "💀", "other", true);
        set_membership(&mut values, "💀", "me", false);
        assert_eq!(values[0].user_ids, ["peer", "other"]);
        set_membership(&mut values, "💀", "peer", false);
        set_membership(&mut values, "💀", "other", false);
        assert!(values.is_empty());
    }
    #[test]
    fn reactor_identity_uses_server_nickname_avatar_and_current_user_label() {
        let snapshot=AppSnapshot{
            current_user:serde_json::from_value(serde_json::json!({"_id":"me","username":"me"})).unwrap(),
            users:serde_json::from_value(serde_json::json!([{"_id":"peer","username":"global","display_name":"Global","avatar":{"_id":"global","tag":"avatars"}}])).unwrap(),
            members:serde_json::from_value(serde_json::json!([{"_id":{"server":"elsewhere","user":"peer"},"nickname":"Wrong"},{"_id":{"server":"here","user":"peer"},"nickname":"Local","avatar":{"_id":"member","tag":"avatars"}}])).unwrap(),
            channels:vec![],servers:vec![],unreads:vec![],voice_states:vec![],
        };
        let person = person(&snapshot, Some("here"), "peer", Some("https://cdn.example"));
        assert_eq!(person.name, "Local");
        assert_eq!(
            person.avatar.as_deref(),
            Some("https://cdn.example/avatars/member/original")
        );
        assert_eq!(super::person(&snapshot, None, "peer", None).name, "Global");
        assert_eq!(
            super::person(&snapshot, Some("here"), "me", None).name,
            stonks_ui::i18n::t("messages.you")
        );
    }
    #[test]
    fn gateway_aliases_scope_membership_and_remove_empty_pills() {
        let mut message=stonks_core::messages::normalize::normalize_message(&serde_json::json!({"_id":"01M0P54M56DA883E6ZYDK00BKC","channel":"01M0P54M56DA883E6ZYDK00BKD","author":"01M0P54M56DA883E6ZYDK00BKE"}),&Default::default(),&Default::default(),None).unwrap();
        let mut event = GatewayEvent {
            event_type: "MessageReact".into(),
            data: serde_json::json!({"id":message.id,"channelId":message.channel_id,"emojiId":"💀","userId":"peer"}),
        };
        assert!(apply_event(&mut message, &event));
        assert_eq!(message.reactions[0].user_ids, ["peer"]);
        event.event_type = "MessageUnreact".into();
        assert!(apply_event(&mut message, &event));
        assert!(message.reactions.is_empty());
    }
}
