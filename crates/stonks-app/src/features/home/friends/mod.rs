//! Mount once per authenticated session. Render `body` in Home and `overlay`
//! at the window overlay layer. Desktop sidebar owners can call `controls`, or
//! use `set_filter` and `open_add` to wire their own navigation. Observe the
//! Friends entity to redraw the host and subscribe to `FriendsEvent` for profile
//! and conversation navigation. The DM channel is already projected into Session.
//! Drop the entity on logout or deployment changes to cancel in-flight requests.

mod view;
pub(crate) use view::Layout;

use std::{collections::HashMap, sync::Arc};

use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::{
    component::input::{InputEvent, InputState},
    *,
};
use stonks_core::{
    connection::{ClientChannel, ClientUser},
    http::HttpPolicy,
    member_actions::{
        self as service, Action, RelationshipKind, relationship_actions, relationship_kind,
    },
};
use stonks_ui::i18n::t;

use crate::{runtime::spawn_core, session::Session, shared::images::Images};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Filter {
    #[default]
    All,
    Online,
    Pending,
    Blocked,
}

impl Filter {
    fn matches(self, user: &ClientUser) -> bool {
        let kind = relationship_kind(user.relationship.as_deref());
        match self {
            Self::All => kind == RelationshipKind::Friend,
            Self::Online => kind == RelationshipKind::Friend && user.online,
            Self::Pending => matches!(
                kind,
                RelationshipKind::Incoming | RelationshipKind::Outgoing | RelationshipKind::Pending
            ),
            Self::Blocked => kind == RelationshipKind::Blocked,
        }
    }
}

pub(crate) enum FriendsEvent {
    OpenProfile {
        user_id: String,
        anchor: Point<Pixels>,
    },
    OpenConversation(ClientChannel),
}

struct Request {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub(crate) struct Friends {
    policy: Arc<HttpPolicy>,
    session: Entity<Session>,
    images: Entity<Images>,
    filter: Filter,
    pending: HashMap<String, (Action, Request)>,
    errors: HashMap<String, String>,
    add_open: bool,
    add_input: Entity<InputState>,
    add_request: Option<Request>,
    add_error: Option<String>,
    previous_focus: Option<FocusHandle>,
    _subscriptions: [Subscription; 3],
}

impl EventEmitter<FriendsEvent> for Friends {}

impl Friends {
    pub(crate) fn new(
        policy: Arc<HttpPolicy>,
        session: Entity<Session>,
        images: Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let add_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t("relationships.usernamePlaceholder"))
                .validate(|value, _| value.encode_utf16().count() <= 64)
        });
        let subscriptions = [
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.observe(&images, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &add_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit_add(window, cx);
                    }
                    cx.notify();
                },
            ),
        ];
        Self {
            policy,
            session,
            images,
            filter: Filter::All,
            pending: HashMap::new(),
            errors: HashMap::new(),
            add_open: false,
            add_input,
            add_request: None,
            add_error: None,
            previous_focus: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn filter(&self) -> Filter {
        self.filter
    }

    pub(crate) fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }

    pub(crate) fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_open {
            return;
        }
        self.add_open = true;
        self.add_error = None;
        self.previous_focus = window.focused(cx);
        self.add_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn add_is_open(&self) -> bool {
        self.add_open
    }

    pub(crate) fn close_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_request.is_some() {
            return;
        }
        self.add_open = false;
        self.add_error = None;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn submit_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.add_open || self.add_request.is_some() {
            return;
        }
        let policy = self.policy.clone();
        let value = self.add_input.read(cx).value();
        let name = value.trim().to_owned();
        if name.is_empty() || name.encode_utf16().count() > 64 {
            return;
        }
        self.add_error = None;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                async move { service::send_friend_request(&policy, &name).await },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let _finished = this.add_request.take();
                match result {
                    Ok(user) => {
                        this.session
                            .update(cx, |session, cx| session.update_user(user, cx));
                        this.close_add(window, cx);
                    }
                    Err(error) => this.add_error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        });
        self.add_request = Some(Request { abort, _task: task });
        cx.notify();
    }

    fn perform(&mut self, id: &str, action: Action, cx: &mut Context<Self>) {
        if self.pending.contains_key(id) {
            return;
        }
        let Some(user) = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .users
                    .iter()
                    .find(|user| user.id == id && user.id != snapshot.current_user.id)
            })
            .cloned()
        else {
            return;
        };
        if !relationship_actions(user.relationship.as_deref(), user.bot.is_some()).contains(&action)
        {
            return;
        }
        self.errors.remove(id);
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let id = id.to_owned();
        let target = id.clone();
        let starting_kind = relationship_kind(user.relationship.as_deref());
        let task = cx.spawn(async move |this, cx| {
            let operation = async move {
                if action == Action::Message {
                    service::open_dm(&policy, &user.id)
                        .await
                        .map(Outcome::Message)
                } else {
                    service::relationship(&policy, &user, action)
                        .await
                        .map(Outcome::Relationship)
                }
            };
            let Ok(result) = spawn_core(Abortable::new(operation, registration)).await else {
                return;
            };
            this.update(cx, |this, cx| {
                let _finished = this.pending.remove(&target);
                match result {
                    Ok(Outcome::Message(channel)) => {
                        this.session.update(cx, |session, cx| {
                            session.upsert_channel(channel.clone(), cx)
                        });
                        cx.emit(FriendsEvent::OpenConversation(channel));
                    }
                    Ok(Outcome::Relationship(changed)) => {
                        this.session.update(cx, |session, cx| {
                            // Keep fresher gateway relationship and presence updates.
                            let Some(mut current) = session
                                .snapshot
                                .as_ref()
                                .and_then(|snapshot| {
                                    snapshot.users.iter().find(|user| user.id == target)
                                })
                                .cloned()
                            else {
                                return;
                            };
                            if relationship_kind(current.relationship.as_deref()) == starting_kind {
                                current.relationship = changed.relationship;
                                session.update_user(current, cx);
                            }
                        });
                    }
                    Err(error) => {
                        this.errors.insert(target, error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.pending
            .insert(id, (action, Request { abort, _task: task }));
        cx.notify();
    }
}

enum Outcome {
    Relationship(ClientUser),
    Message(ClientChannel),
}

#[cfg(test)]
mod tests {
    use super::{ClientUser, Filter};

    #[test]
    fn filters_only_include_the_corresponding_relationships() {
        for (relationship, online, expected) in [
            (" Friend ", true, vec![Filter::All, Filter::Online]),
            ("Friend", false, vec![Filter::All]),
            ("pending_incoming", true, vec![Filter::Pending]),
            ("request-sent", false, vec![Filter::Pending]),
            ("Pending", false, vec![Filter::Pending]),
            ("Blocked", true, vec![Filter::Blocked]),
            ("BestFriend", true, vec![]),
        ] {
            let user: ClientUser = serde_json::from_value(serde_json::json!({"_id":"user", "username":"test", "relationship":relationship, "online":online})).unwrap();
            assert_eq!(
                [
                    Filter::All,
                    Filter::Online,
                    Filter::Pending,
                    Filter::Blocked
                ]
                .into_iter()
                .filter(|filter| filter.matches(&user))
                .collect::<Vec<_>>(),
                expected,
                "{relationship}"
            );
        }
    }
}
