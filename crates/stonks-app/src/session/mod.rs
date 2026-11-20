//! Live session: owns the gateway connection for the signed-in account and
//! mirrors the ready snapshot (servers, channels, users, members) on the
//! GPUI side. The core worker calls the [`ConnectionEvents`] sink on its
//! tokio thread; a channel forwards updates to this entity on the
//! foreground executor.

use futures_util::future::{AbortHandle, Abortable};
use std::sync::Arc;

use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::StreamExt;
use gpui_kit::*;
use serde_json::Value;
use stonks_core::connection::{
    self, AppSnapshot, ClientChannel, ClientServer, ConnectionEvents, ConnectionManager,
    ConnectionStatus, GatewayEvent,
};
use stonks_core::http::HttpPolicy;
use stonks_core::server::ServerSelection;

use crate::runtime::spawn_core;

mod members;
mod reads;
mod ready_reads;
mod voice;
struct ReadRequest {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for ReadRequest {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
struct MemberRequest {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for MemberRequest {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

enum Update {
    Status(ConnectionStatus),
    Ready(AppSnapshot),
    Gateway(GatewayEvent),
}

/// Forwarded gateway traffic for feature entities (timeline, voice, …).
pub enum SessionEvent {
    Gateway(GatewayEvent),
}

/// Live gateway creates only; never emitted for REST mutation projections or Ready.
pub(crate) struct LiveMessage(pub GatewayEvent);

struct ChannelEvents(UnboundedSender<Update>);

impl ConnectionEvents for ChannelEvents {
    fn status(&self, status: ConnectionStatus) {
        let _ = self.0.unbounded_send(Update::Status(status));
    }

    fn ready(&self, snapshot: AppSnapshot) {
        let _ = self.0.unbounded_send(Update::Ready(snapshot));
    }

    fn gateway_event(&self, event: GatewayEvent) {
        let _ = self.0.unbounded_send(Update::Gateway(event));
    }
}

pub struct Session {
    policy: Arc<HttpPolicy>,
    account_context: Option<stonks_core::http::AuthenticatedContext>,
    account_refresh: Option<ReadRequest>,
    manager: Arc<ConnectionManager>,
    pub status: Option<ConnectionStatus>,
    pub snapshot: Option<AppSnapshot>,
    pub server: Option<ServerSelection>,
    connecting: bool,
    connection_generation: u64,
    member_server: Option<String>,
    member_request: Option<MemberRequest>,
    member_generation: u64,
    member_loading: bool,
    member_events: Vec<GatewayEvent>,
    pub member_error: Option<String>,
    read_state: reads::ReadState,
    read_request: Option<ReadRequest>,
    ready_read_probe: Option<ReadRequest>,
    section_reads: reads::SectionReads,
    pub read_error: Option<String>,
}

impl EventEmitter<SessionEvent> for Session {}
impl EventEmitter<LiveMessage> for Session {}

impl Session {
    pub fn authorization(&self) -> Arc<ConnectionManager> {
        self.manager.clone()
    }

    pub fn can_server_permission(&self, server: &str, permission: u64) -> bool {
        self.manager
            .require_server_permission(server, permission)
            .is_ok()
    }

    pub fn can_channel_permission(&self, channel: &str, permission: u64) -> bool {
        self.manager
            .require_channel_permission(channel, permission)
            .is_ok()
    }
    pub fn can_view_channel(&self, channel: &str) -> bool {
        self.manager
            .require_channel_permission(channel, 1 << 20)
            .is_ok()
            || self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.channels.iter().any(|entry| {
                    entry.id == channel
                        && entry.server.is_none()
                        && (entry.recipients.contains(&snapshot.current_user.id)
                            || entry.owner.as_deref() == Some(&snapshot.current_user.id))
                })
            })
    }

    pub fn can_view_text_channel(&self, channel: &str) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .channels
                .iter()
                .any(|entry| entry.id == channel && !entry.is_voice())
        }) && self.can_view_channel(channel)
    }

    pub fn can_manage_server(&self, server: &str) -> bool {
        self.manager
            .require_server_permission(server, 1 << 1)
            .is_ok()
    }

    pub fn update_server(
        &mut self,
        server: stonks_core::connection::ClientServer,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self.snapshot.as_mut().and_then(|snapshot| {
            snapshot
                .servers
                .iter_mut()
                .find(|entry| entry.id == server.id)
        }) {
            *existing = server;
            cx.notify();
        }
    }
    pub fn new(policy: Arc<HttpPolicy>) -> Self {
        Self {
            policy,
            account_context: None,
            account_refresh: None,
            manager: Arc::new(ConnectionManager::default()),
            status: None,
            snapshot: None,
            server: None,
            connecting: false,
            connection_generation: 0,
            member_server: None,
            member_request: None,
            member_generation: 0,
            member_loading: false,
            member_events: Vec::new(),
            member_error: None,
            read_state: Default::default(),
            read_request: None,
            ready_read_probe: None,
            section_reads: Default::default(),
            read_error: None,
        }
    }

    pub fn connect(&mut self, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let context = match connection::selected_session(&self.policy) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("session: no authenticated context: {error}");
                return;
            }
        };
        self.connection_generation = self.connection_generation.wrapping_add(1);
        let generation = self.connection_generation;
        self.section_reads = Default::default();
        self.snapshot = None;
        self.status = None;
        self.account_refresh = None;
        self.account_context = Some(context.clone());
        self.connecting = true;
        self.server = Some(context.server.clone());
        let (sender, receiver) = unbounded();
        let policy = self.policy.clone();
        let manager = self.manager.clone();
        cx.spawn(async move |this, cx| {
            let started = spawn_core(async move {
                connection::start_connection(
                    policy,
                    manager,
                    Arc::new(ChannelEvents(sender)),
                    context,
                    None,
                )
            })
            .await;
            this.update(cx, |this, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connecting = false;
                if let Err(error) = started {
                    this.status = None;
                    eprintln!("session: could not start the gateway connection: {error}");
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let mut receiver = receiver;
            while let Some(update) = receiver.next().await {
                if !this
                    .update(cx, |this, cx| {
                        if this.connection_generation != generation {
                            return false;
                        }
                        this.apply(update, cx);
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn account_context(
        &self,
    ) -> Option<(Arc<HttpPolicy>, stonks_core::http::AuthenticatedContext)> {
        let context = self.account_context.as_ref()?;
        self.policy
            .is_authenticated_context_current(context)
            .then(|| (self.policy.clone(), context.clone()))
    }

    pub fn can_route_external(&self) -> bool {
        self.snapshot.is_some()
            && self
                .status
                .as_ref()
                .is_some_and(|status| status.state == "ready")
            && self.account_context().is_some()
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.connecting = false;
        self.section_reads = Default::default();
        self.snapshot = None;
        self.status = None;
        self.server = None;
        self.account_context = None;
        self.account_refresh = None;
        self.ready_read_probe = None;
        self.read_request = None;
        self.read_state.reset();
        self.read_error = None;
        self.member_request = None;
        self.member_server = None;
        self.member_loading = false;
        self.member_generation = self.member_generation.wrapping_add(1);
        let manager = self.manager.clone();
        cx.notify();
        cx.spawn(async move |_, _| {
            let _ = spawn_core(async move { connection::stop_connection(&manager, None) }).await;
        })
        .detach();
    }

    fn apply(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Status(status) => self.status = Some(status),
            Update::Ready(mut snapshot) => {
                self.section_reads = Default::default();
                self.read_request = None;
                self.read_state.reset();
                self.read_error = None;
                members::deduplicate(&mut snapshot);
                self.member_request = None;
                self.member_server = None;
                self.member_loading = false;
                self.member_generation = self.member_generation.wrapping_add(1);
                self.snapshot = Some(snapshot);
                self.probe_ready_read_heads(cx);
            }
            Update::Gateway(event) => {
                self.apply_gateway_event(&event);
                if event.event_type == "Message" && self.snapshot.is_some() {
                    cx.emit(LiveMessage(event.clone()));
                }
                cx.emit(SessionEvent::Gateway(event));
            }
        }
        cx.notify();
    }

    /// Structural events the sidebar cares about. Message traffic is
    /// timeline work (M3 continuation) and is ignored here for now.
    pub fn project_message_mutation(&mut self, event: GatewayEvent, cx: &mut Context<Self>) {
        self.apply_gateway_event(&event);
        cx.emit(SessionEvent::Gateway(event));
        cx.notify();
    }

    fn apply_gateway_event(&mut self, event: &GatewayEvent) {
        if self.member_loading
            && matches!(
                event.event_type.as_str(),
                "UserUpdate"
                    | "ServerMemberUpdate"
                    | "ServerMemberLeave"
                    | "ServerRoleUpdate"
                    | "ServerRoleDelete"
            )
        {
            self.member_events.push(event.clone());
        }
        if event.event_type == "ServerMemberJoin" {
            self.member_server = None;
        }
        let Some(snapshot) = self.snapshot.as_mut() else {
            return;
        };
        members::apply(snapshot, event);
        voice::apply(snapshot, event);
        if event.event_type != "ChannelAck"
            || !event.data["id"]
                .as_str()
                .is_some_and(|channel| self.read_state.suppresses(channel))
        {
            stonks_core::messages::read::apply_event(snapshot, event);
        }
        if self.read_state.clear_confirmed_failure(snapshot) {
            self.read_error = None;
        }
        match event.event_type.as_str() {
            "ServerCreate" => {
                if let Some(server) = event
                    .data
                    .get("server")
                    .and_then(|server| serde_json::from_value::<ClientServer>(server.clone()).ok())
                {
                    upsert(&mut snapshot.servers, server, |server| server.id.clone());
                } else if let Ok(server) =
                    serde_json::from_value::<ClientServer>(event.data.clone())
                {
                    upsert(&mut snapshot.servers, server, |server| server.id.clone());
                }
                if let Some(channels) = event.data.get("channels").and_then(Value::as_array) {
                    for channel in channels {
                        if let Ok(channel) =
                            serde_json::from_value::<ClientChannel>(channel.clone())
                        {
                            upsert_channel(snapshot, channel);
                        }
                    }
                }
                voice::apply_created_server(snapshot, &event.data);
            }
            "ServerUpdate" => {
                if let (Some(id), Ok(channels)) = (
                    event.data["id"].as_str(),
                    serde_json::from_value::<Vec<String>>(event.data["data"]["channels"].clone()),
                ) {
                    if let Some(server) = snapshot.servers.iter_mut().find(|server| server.id == id)
                    {
                        server.channels = channels;
                    }
                }
            }
            "ServerDelete" => {
                if let Some(id) = event.data.get("id").and_then(Value::as_str) {
                    snapshot.servers.retain(|server| server.id != id);
                    snapshot.voice_states.retain(|state| {
                        !snapshot.channels.iter().any(|channel| {
                            channel.id == state.id && channel.server.as_deref() == Some(id)
                        })
                    });
                    snapshot.channels.retain(|channel| {
                        channel.server.as_deref().is_none_or(|server| server != id)
                    });
                }
            }
            "ChannelCreate" => {
                if let Ok(channel) = serde_json::from_value::<ClientChannel>(event.data.clone()) {
                    upsert_channel(snapshot, channel);
                }
            }
            "ChannelDelete" => {
                if let Some(id) = event.data.get("id").and_then(Value::as_str) {
                    for server in &mut snapshot.servers {
                        server.channels.retain(|channel| channel != id);
                    }
                    snapshot.channels.retain(|channel| channel.id != id);
                    snapshot.voice_states.retain(|state| state.id != id);
                }
            }
            _ => {}
        }
    }
}

fn upsert_channel(snapshot: &mut AppSnapshot, channel: ClientChannel) {
    if let Some(server) = snapshot
        .servers
        .iter_mut()
        .find(|server| Some(server.id.as_str()) == channel.server.as_deref())
    {
        if !server.channels.contains(&channel.id) {
            server.channels.push(channel.id.clone());
        }
    }
    upsert(&mut snapshot.channels, channel, |channel| {
        channel.id.clone()
    });
}

impl Session {
    pub fn set_typing(&self, channel_id: String, typing: bool) -> Result<(), String> {
        connection::set_typing(&self.manager, channel_id, typing)
    }

    /// Supply the newest painted row intersecting the viewport, including
    /// older history. On blur/hidden navigation use foreground=false or channel=None.
    pub fn observe_read_view(
        &mut self,
        channel: Option<&str>,
        latest_visible: Option<&str>,
        foreground: bool,
        cx: &mut Context<Self>,
    ) {
        let foreground = foreground && !channel.is_some_and(|id| self.section_reads.active_for(id));
        let effect =
            self.read_state
                .observe(self.snapshot.as_ref(), channel, latest_visible, foreground);
        if effect.cancel {
            self.read_request = None;
            if self.read_error.take().is_some() {
                cx.notify();
            }
        }
        if let Some(target) = effect.start {
            self.start_read(target, cx);
        }
        if effect.cancel {
            self.start_section_read(cx);
        }
    }

    pub fn mark_unread(&mut self, channel: &str, previous: Option<&str>, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            self.section_reads.cancel(channel);
            self.read_state.mark_unread(channel);
            self.read_request = None;
            self.read_error = None;
            stonks_core::messages::read::mark_unread(snapshot, channel, previous);
            self.start_section_read(cx);
            cx.notify();
        }
    }

    pub fn retry_read(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| self.read_state.retry(snapshot))
        {
            self.start_read(target, cx);
        }
    }

    fn start_read(&mut self, target: reads::Target, cx: &mut Context<Self>) {
        if self.read_error.take().is_some() {
            cx.notify();
        }
        let policy = self.policy.clone();
        let request = target.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    stonks_core::messages::read::acknowledge(
                        &policy,
                        &request.channel,
                        &request.message,
                    )
                    .await
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                let confirmed = result.is_ok()
                    || this.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.current_user.id == target.user
                            && stonks_core::messages::read::cursor(snapshot, &target.channel)
                                .is_some_and(|cursor| cursor >= target.message.as_str())
                    });
                if !this.read_state.complete(&target, confirmed) {
                    return;
                }
                match result {
                    Ok(()) => {
                        if let Some(snapshot) = this.snapshot.as_mut() {
                            if snapshot.current_user.id != target.user {
                                return;
                            }
                            stonks_core::messages::read::advance(
                                snapshot,
                                &target.channel,
                                &target.message,
                            );
                        }
                        this.read_error = None;
                    }
                    Err(_) if confirmed => this.read_error = None,
                    Err(error) => this.read_error = Some(error.to_string()),
                }
                let next = this
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| this.read_state.next(snapshot));
                if let Some(next) = next {
                    this.start_read(next, cx);
                }
                this.start_section_read(cx);
                cx.notify();
            })
            .ok();
        });
        self.read_request = Some(ReadRequest { abort, _task: task });
    }

    pub fn load_members(&mut self, server_id: String, retry: bool, cx: &mut Context<Self>) {
        if self.snapshot.is_none() || (!retry && self.member_server.as_ref() == Some(&server_id)) {
            return;
        }
        self.member_request = None;
        self.member_server = Some(server_id.clone());
        self.member_generation = self.member_generation.wrapping_add(1);
        let generation = self.member_generation;
        self.member_loading = true;
        self.member_events.clear();
        self.member_error = None;
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let request_server = server_id.clone();
            let Ok(result) = spawn_core(Abortable::new(
                async move { connection::fetch_server_members(&policy, request_server).await },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.member_generation != generation {
                    return;
                }
                this.member_loading = false;
                match (result, this.snapshot.as_mut()) {
                    (Ok(roster), Some(snapshot)) => {
                        snapshot
                            .members
                            .retain(|member| member.id.server != server_id);
                        snapshot.members.extend(
                            roster
                                .members
                                .into_iter()
                                .filter(|member| member.id.server == server_id),
                        );
                        for user in roster.users {
                            if snapshot.current_user.id == user.id {
                                snapshot.current_user = user.clone();
                            }
                            upsert(&mut snapshot.users, user, |user| user.id.clone());
                        }
                        members::deduplicate(snapshot);
                        for event in this.member_events.drain(..) {
                            members::apply(snapshot, &event);
                        }
                    }
                    (Err(error), _) => this.member_error = Some(error.to_string()),
                    _ => {}
                }
                this.member_events.clear();
                cx.notify();
            })
            .ok();
        });
        self.member_request = Some(MemberRequest { abort, _task: task });
    }
}

fn upsert<T>(values: &mut Vec<T>, value: T, id_of: impl Fn(&T) -> String) {
    let id = id_of(&value);
    if let Some(existing) = values.iter_mut().find(|candidate| id_of(candidate) == id) {
        *existing = value;
    } else {
        values.push(value);
    }
}

mod mutations;
