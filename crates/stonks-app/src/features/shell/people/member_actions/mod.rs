//! Native member interactions, following the Tauri member and relationship controllers.
use crate::{session::Session, shared::images::Images};
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{collections::HashSet, future::Future, sync::Arc};
use stonks_core::member_actions::Action;
use stonks_core::{
    connection::{ClientChannel, ClientMember, ClientServer, ClientUser},
    error::CommandError,
    http::HttpPolicy,
    media,
    member_actions::{self as service, MemberPatch},
};
use stonks_ui::{
    components::{
        avatar, input_field, Button, ButtonVariant, Checkbox, Feedback, FeedbackTone,
        SettingsInput, UiDialog,
    },
    icons::Icon,
    Theme,
};
mod editor;
mod profile;
mod requests;
mod state;
use state::{field_length, Gate, IdentityDraft, RelationshipRequest};

pub enum MemberActionsEvent {
    Navigate(ClientChannel),
    UpdatedUser(ClientUser),
    UpdatedMember {
        server: String,
        user: String,
        patch: MemberPatch,
    },
    RemovedMember {
        server: String,
        user: String,
    },
    UpdatedServer(ClientServer),
    Close,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    server: String,
    user: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Profile,
    Identity,
    Roles,
    Confirm(Action),
    Report,
    Mfa,
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

pub struct MemberActions {
    policy: Arc<HttpPolicy>,
    session: Entity<Session>,
    images: Entity<Images>,
    target: Option<Target>,
    stage: Stage,
    position: Option<Point<Pixels>>,
    return_to_profile: bool,
    focus: FocusHandle,
    gate: Gate,
    error: Option<String>,
    profile_pending: Option<Action>,
    profile: service::Profile,
    nickname: Entity<InputState>,
    pronouns: Entity<InputState>,
    initial_identity: IdentityDraft,
    context: Entity<TextareaState>,
    secret: Entity<InputState>,
    roles: HashSet<String>,
    avatar_file: Option<stonks_core::uploads::UploadFile>,
    avatar_preview: Option<Arc<RenderImage>>,
    preview_task: Option<Task<()>>,
    preview_revision: u64,
    avatar_url: Option<String>,
    mfa_methods: Vec<String>,
    mfa_method: String,
    mfa_focus_pending: bool,
    request: Option<Request>,
    transfer_request: Option<Request>,
    profile_request: Option<Request>,
    picker: Option<Task<()>>,
    identity: Option<(String, String)>,
    _observe: Subscription,
    _images: Subscription,
    _inputs: Vec<Subscription>,
}
fn identity(session: &Session) -> Option<(String, String)> {
    session
        .snapshot
        .as_ref()
        .zip(session.server.as_ref())
        .map(|(snapshot, server)| {
            (
                server.deployment_url.clone(),
                snapshot.current_user.id.clone(),
            )
        })
}

impl MemberActions {
    pub fn new(
        policy: Arc<HttpPolicy>,
        session: Entity<Session>,
        images: Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let nickname = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Use account display name")
                .validate(|v, _| field_length(v) <= 32)
        });
        let pronouns = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Use account pronouns")
                .validate(|v, _| field_length(v) <= 24)
        });
        let context = cx.new(|cx| TextareaState::new(window, cx));
        let secret = cx.new(|cx| InputState::new(window, cx));
        let mut input_subscriptions =
            vec![cx.subscribe(&nickname, |_, _, _: &InputEvent, cx| cx.notify())];
        input_subscriptions.push(cx.subscribe(&pronouns, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. })
                && this.stage == Stage::Identity
                && !this.busy()
            {
                this.submit_edit(cx);
            }
            cx.notify();
        }));
        input_subscriptions.push(cx.subscribe_in(
            &secret,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.stage == Stage::Mfa
                    && !this.busy()
                {
                    this.submit_transfer(window, cx);
                }
                cx.notify();
            },
        ));
        let current_identity = identity(session.read(cx));
        let observe = cx.observe_in(&session, window, |this, session, window, cx| {
            let next = identity(session.read(cx));
            if this.identity != next {
                this.reset(window, cx);
                this.identity = next;
            }
            cx.notify();
        });
        let observe_images = cx.observe(&images, |_, _, cx| cx.notify());
        cx.on_release(|this, cx| {
            this.images.update(cx, |images, _| {
                images.set_sources("member-profile", Vec::<(String, u32)>::new())
            });
        })
        .detach();
        Self {
            policy,
            session,
            images,
            target: None,
            stage: Stage::Profile,
            position: None,
            return_to_profile: false,
            focus: cx.focus_handle(),
            gate: Gate::default(),
            error: None,
            profile_pending: None,
            profile: Default::default(),
            nickname,
            pronouns,
            initial_identity: Default::default(),
            context,
            secret,
            roles: Default::default(),
            avatar_file: None,
            avatar_preview: None,
            preview_task: None,
            preview_revision: 0,
            avatar_url: None,
            mfa_methods: vec![],
            mfa_method: String::new(),
            mfa_focus_pending: false,
            request: None,
            transfer_request: None,
            profile_request: None,
            picker: None,
            identity: current_identity,
            _observe: observe,
            _images: observe_images,
            _inputs: input_subscriptions,
        }
    }
    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }
    pub fn busy(&self) -> bool {
        self.gate.busy()
    }
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy() {
            self.reset(window, cx);
            cx.emit(MemberActionsEvent::Close);
        }
    }
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.gate.reset();
        self.request = None;
        self.transfer_request = None;
        self.profile_request = None;
        self.picker = None;
        self.target = None;
        self.return_to_profile = false;
        self.profile = Default::default();
        self.error = None;
        self.profile_pending = None;
        self.roles.clear();
        self.initial_identity = Default::default();
        self.avatar_file = None;
        self.avatar_preview = None;
        self.preview_task = None;
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.avatar_url = None;
        self.mfa_methods.clear();
        self.mfa_method.clear();
        self.mfa_focus_pending = false;
        self.nickname
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.pronouns
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.context
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.images.update(cx, |images, _| {
            images.set_sources("member-profile", Vec::<(String, u32)>::new())
        });
        cx.notify();
    }
    fn finish_close(&mut self, cx: &mut Context<Self>) {
        self.gate.reset();
        self.profile_request = None;
        self.picker = None;
        self.target = None;
        self.profile = Default::default();
        self.profile_pending = None;
        self.roles.clear();
        self.initial_identity = Default::default();
        self.avatar_file = None;
        self.avatar_preview = None;
        self.preview_task = None;
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.avatar_url = None;
        self.images.update(cx, |images, _| {
            images.set_sources("member-profile", Vec::<(String, u32)>::new())
        });
        cx.emit(MemberActionsEvent::Close);
    }
    fn user(&self, cx: &App) -> Option<ClientUser> {
        let id = &self.target.as_ref()?.user;
        self.session
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                std::iter::once(&snapshot.current_user)
                    .chain(snapshot.users.iter())
                    .find(|user| &user.id == id)
            })
            .cloned()
    }
    fn member(&self, cx: &App) -> Option<ClientMember> {
        let target = self.target.as_ref()?;
        self.session
            .read(cx)
            .snapshot
            .as_ref()?
            .members
            .iter()
            .find(|member| member.id.server == target.server && member.id.user == target.user)
            .cloned()
    }
    fn server(&self, cx: &App) -> Option<ClientServer> {
        let target = self.target.as_ref()?;
        self.session
            .read(cx)
            .snapshot
            .as_ref()?
            .servers
            .iter()
            .find(|server| server.id == target.server)
            .cloned()
    }
    fn self_target(&self, cx: &App) -> bool {
        self.target
            .as_ref()
            .zip(self.session.read(cx).snapshot.as_ref())
            .is_some_and(|(target, snapshot)| target.user == snapshot.current_user.id)
    }
    fn can(&self, mask: u64, cx: &App) -> bool {
        self.target.as_ref().is_some_and(|target| {
            self.session
                .read(cx)
                .can_server_permission(&target.server, mask)
        })
    }
    pub fn context_actions(&self, server: &str, user: &str, cx: &App) -> Vec<Action> {
        let session = self.session.read(cx);
        let Some(snapshot) = &session.snapshot else {
            return vec![];
        };
        let masks = [
            service::KICK_MEMBERS,
            service::BAN_MEMBERS,
            service::ASSIGN_ROLES,
            service::CHANGE_NICKNAME,
            service::MANAGE_NICKNAMES,
            service::CHANGE_AVATAR,
        ];
        let permissions = masks
            .into_iter()
            .filter(|mask| session.can_server_permission(server, *mask))
            .fold(0, |value, mask| value | mask);
        service::context_actions(snapshot.current_user.id == user, permissions)
    }

    fn profile_actions(&self, cx: &App) -> Vec<Action> {
        if self.self_target(cx) {
            return vec![];
        }
        let Some(user) = self.user(cx) else {
            return vec![];
        };
        let mut actions = service::relationship_actions(
            user.relationship.as_deref(),
            user.bot
                .as_ref()
                .is_some_and(|bot| !bot.is_null() && bot != &serde_json::Value::Bool(false)),
        );
        if self
            .server(cx)
            .zip(self.session.read(cx).snapshot.as_ref())
            .is_some_and(|(server, snapshot)| server.owner == snapshot.current_user.id)
            && self.member(cx).is_some()
            && user
                .bot
                .as_ref()
                .is_none_or(|bot| bot.is_null() || bot == &serde_json::Value::Bool(false))
        {
            actions.push(Action::Transfer);
        }
        actions
    }
    pub fn open_profile(
        &mut self,
        server: String,
        user: String,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        self.reset(window, cx);
        self.target = Some(Target {
            server,
            user: user.clone(),
        });
        self.stage = Stage::Profile;
        self.return_to_profile = true;
        self.position = position;
        self.focus.focus(window, cx);
        if self.user(cx).is_none() {
            self.target = None;
            return;
        }
        let generation = self.gate.generation();
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let result = spawn_core(Abortable::new(
                async move { service::profile(&policy, &user).await },
                registration,
            ))
            .await;
            this.update(cx, |this, cx| {
                if this.gate.generation() != generation {
                    return;
                }
                if let Ok(Ok(profile)) = result {
                    this.profile = profile;
                    cx.notify();
                }
            })
            .ok();
        });
        self.profile_request = Some(Request { abort, _task: task });
        cx.notify();
    }
    pub fn perform(
        &mut self,
        server: String,
        user: String,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        if action == Action::Profile {
            self.open_profile(server, user, None, window, cx);
            return;
        }
        let context_allowed = self.context_actions(&server, &user, cx).contains(&action);
        let had_profile = self.stage == Stage::Profile
            && self.target.as_ref()
                == Some(&Target {
                    server: server.clone(),
                    user: user.clone(),
                });
        if self.target.as_ref()
            != Some(&Target {
                server: server.clone(),
                user: user.clone(),
            })
        {
            self.open_profile(server.clone(), user.clone(), None, window, cx);
        }
        if !(context_allowed || self.profile_actions(cx).contains(&action)) {
            return;
        }
        self.return_to_profile = had_profile;
        self.error = None;
        match action {
            Action::Identity | Action::Roles => self.start_edit(action, window, cx),
            Action::Kick | Action::Ban | Action::Block | Action::Transfer => {
                self.stage = Stage::Confirm(action);
                self.focus.focus(window, cx);
            }
            Action::Report => {
                self.stage = Stage::Report;
                self.context.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.focus(window, cx);
                });
            }
            _ => self.execute(action, cx),
        }
        cx.notify();
    }
    fn run<T: Send + 'static>(
        &mut self,
        future: impl Future<Output = Result<T, CommandError>> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(ticket) = self.gate.start() else {
            return;
        };
        self.error = None;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let result = spawn_core(Abortable::new(future, registration)).await;
            this.update(cx, |this, cx| {
                if !this.gate.complete(ticket) {
                    return;
                }
                this.profile_pending = None;
                match result {
                    Ok(result) => done(this, result.map_err(|error| error.to_string()), cx),
                    Err(_) => {
                        this.error = Some("The member request was cancelled.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.request = Some(Request { abort, _task: task });
        cx.notify();
    }
    fn execute(&mut self, action: Action, cx: &mut Context<Self>) {
        let (Some(target), Some(user)) = (self.target.clone(), self.user(cx)) else {
            return;
        };
        if action == Action::Message {
            let existing = self
                .session
                .read(cx)
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot.channels.iter().find(|channel| {
                        channel.channel_type == "DirectMessage"
                            && channel.recipients.contains(&user.id)
                    })
                })
                .cloned();
            if let Some(channel) = existing {
                cx.emit(MemberActionsEvent::Navigate(channel));
                self.finish_close(cx);
                return;
            }
            let policy = self.policy.clone();
            self.run(
                async move { service::open_dm(&policy, &user.id).await },
                |this, result, cx| match result {
                    Ok(channel) => {
                        cx.emit(MemberActionsEvent::Navigate(channel));
                        this.finish_close(cx);
                    }
                    Err(error) => this.error = Some(error),
                },
                cx,
            );
        } else if matches!(action, Action::Kick | Action::Ban) {
            if self.self_target(cx) {
                return;
            }
            let policy = self.policy.clone();
            let authorization = self.session.read(cx).authorization();
            let for_request = target.clone();
            self.run(
                async move {
                    service::moderate(
                        &policy,
                        &authorization,
                        &for_request.server,
                        &for_request.user,
                        action == Action::Ban,
                    )
                    .await
                },
                move |this, result, cx| match result {
                    Ok(()) => {
                        cx.emit(MemberActionsEvent::RemovedMember {
                            server: target.server,
                            user: target.user,
                        });
                        this.finish_close(cx);
                    }
                    Err(error) => this.error = Some(error),
                },
                cx,
            );
        } else {
            let allowed = action == Action::Block || self.profile_actions(cx).contains(&action);
            if !allowed {
                return;
            }
            let policy = self.policy.clone();
            let relationship_request = RelationshipRequest::new(&user);
            self.run(
                async move { service::relationship(&policy, &user, action).await },
                move |this, result, cx| match result {
                    Ok(user) => {
                        // A gateway update from another client may have changed
                        // the relationship while this request was in flight.
                        if this
                            .user(cx)
                            .is_some_and(|current| relationship_request.accepts(&current))
                        {
                            cx.emit(MemberActionsEvent::UpdatedUser(user));
                        }
                        if this.return_to_profile {
                            this.stage = Stage::Profile;
                        } else {
                            this.finish_close(cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                },
                cx,
            );
        }
    }
    fn perform_profile_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || !self.profile_actions(cx).contains(&action) {
            return;
        }
        self.error = None;
        self.profile_pending = Some(action);
        self.return_to_profile = true;
        if action == Action::Transfer {
            self.stage = Stage::Confirm(action);
            self.focus.focus(window, cx);
            cx.notify();
        } else {
            self.execute(action, cx);
        }
    }
    fn confirm(&mut self, action: Action, cx: &mut Context<Self>) {
        if action != Action::Transfer {
            self.execute(action, cx);
            return;
        }
        if !self.profile_actions(cx).contains(&Action::Transfer) {
            return;
        }
        let policy = self.policy.clone();
        self.run(
            async move { service::mfa_methods(&policy).await },
            |this, result, _| match result {
                Ok(methods) => {
                    this.mfa_method = methods.first().cloned().unwrap_or_default();
                    this.mfa_methods = methods;
                    this.stage = Stage::Mfa;
                    this.mfa_focus_pending = true;
                    this.return_to_profile = false;
                }
                Err(error) => this.error = Some(error),
            },
            cx,
        );
    }
    fn submit_report(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        if self.self_target(cx) {
            return;
        }
        let policy = self.policy.clone();
        let context = self.context.read(cx).value().to_string();
        self.run(
            async move { service::report(&policy, &target.user, &context).await },
            |this, result, cx| match result {
                Ok(()) => {
                    if this.return_to_profile {
                        this.stage = Stage::Profile;
                    } else {
                        this.finish_close(cx);
                    }
                }
                Err(error) => this.error = Some(error),
            },
            cx,
        );
    }
    fn submit_transfer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        if !self.profile_actions(cx).contains(&Action::Transfer) {
            return;
        }
        let value = self.secret.read(cx).value().to_string();
        let method = self.mfa_method.clone();
        let policy = self.policy.clone();
        self.run_in(
            window,
            cx,
            async move { service::mfa_ticket(&policy, &method, &value).await },
            move |this, result, window, cx| match result {
                Ok(ticket) => {
                    this.secret
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    this.stage = Stage::Profile;
                    this.return_to_profile = true;
                    this.profile_pending = Some(Action::Transfer);
                    this.begin_transfer(target, ticket, cx);
                }
                Err(error) => this.error = Some(error),
            },
        );
    }
    fn cancel_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        if self.return_to_profile {
            self.stage = Stage::Profile;
            self.error = None;
            cx.notify();
        } else {
            self.close(window, cx);
        }
    }
}
impl EventEmitter<MemberActionsEvent> for MemberActions {}
impl Render for MemberActions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.target.is_none() {
            return div().into_any_element();
        }
        let theme = Theme::current(cx);
        match self.stage {
            Stage::Profile => self.render_profile(theme, window, cx),
            Stage::Identity | Stage::Roles => self.render_editor(theme, window, cx),
            _ => self.render_prompt(theme, window, cx),
        }
    }
}
