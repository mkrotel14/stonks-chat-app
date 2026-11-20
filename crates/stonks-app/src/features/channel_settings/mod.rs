//! Channel settings and destructive-action confirmations, ported from Tauri.
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
pub use stonks_core::channel_actions::ChannelAccess;
use stonks_core::{
    channel_actions::{self as service, Overview},
    connection::{ClientChannel, ClientServer, ConnectionManager, PermissionOverride},
    http::HttpPolicy,
};
use stonks_ui::{
    Theme,
    components::select::{IndexPath, SearchableVec},
    components::{
        Button, ButtonVariant, Feedback, FeedbackTone, SettingsInput, Switch, UiCloseButton,
        UiSelect, UiSelectEvent, UiSelectOption, UiSelectState, input_field, settings_field,
        settings_row, settings_section,
    },
    i18n::t,
    icons::Icon,
};
mod danger;
mod overview;
mod permissions;
mod view;

pub use danger::ChannelDanger;
use permissions::PermissionDraft;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Permissions,
}
impl Page {
    fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Permissions => 1,
        }
    }

    fn label(self) -> &'static str {
        if self == Self::Overview {
            "Overview"
        } else {
            "Permissions"
        }
    }
}
pub enum ChannelSettingsEvent {
    Updated(ClientChannel),
    Deleted(String),
    Reset { channel_id: String, deleted: usize },
    Error(String),
    Close,
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
enum Action {
    Overview(Overview),
    Permissions(Option<String>, PermissionOverride),
    Danger(ChannelDanger),
}
impl Action {
    fn allowed(&self, access: ChannelAccess) -> bool {
        match self {
            Self::Overview(_) => access.manage_channel,
            Self::Permissions(..) => access.manage_permissions,
            Self::Danger(ChannelDanger::Reset) => access.manage_messages,
            Self::Danger(ChannelDanger::Delete) => access.manage_channel,
        }
    }
}
enum Completion {
    Updated(ClientChannel),
    Deleted,
    Reset(usize),
}
pub struct ChannelSettings {
    channel: ClientChannel,
    server: ClientServer,
    policy: Arc<HttpPolicy>,
    authorization: Arc<ConnectionManager>,
    access: ChannelAccess,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    slowmode: Entity<UiSelectState>,
    nsfw: bool,
    initial: Overview,
    permissions: BTreeMap<Option<String>, PermissionDraft>,
    page: Page,
    settings_index: bool,
    compact_layout: bool,
    frame_visible: bool,
    confirmation: Option<ChannelDanger>,
    pending_focus: bool,
    pending_target: Option<FocusHandle>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    request: Option<Request>,
    active_permission: Option<u64>,
    generation: u64,
    focus: FocusHandle,
    navigation_focus: FocusHandle,
    heading_focus: FocusHandle,
    page_focus: [FocusHandle; 2],
    confirmation_focus: FocusHandle,
    feedback_generation: u64,
    feedback_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ChannelSettingsEvent> for ChannelSettings {}
impl ChannelSettings {
    pub fn new(
        channel: ClientChannel,
        server: ClientServer,
        policy: Arc<HttpPolicy>,
        authorization: Arc<ConnectionManager>,
        access: ChannelAccess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = Overview::from_channel(&channel);
        let name = cx.new(|cx| {
            InputState::new(window, cx).validate(|value, _| value.encode_utf16().count() <= 32)
        });
        name.update(cx, |state, cx| {
            state.set_value(initial.name.clone(), window, cx)
        });
        let description = cx.new(|cx| TextareaState::new(window, cx));
        description.update(cx, |state, cx| {
            state.set_value(initial.description.clone(), window, cx)
        });
        let slowmode_options = overview::slowmode_options(initial.slowmode)
            .into_iter()
            .map(|(value, label)| UiSelectOption::new(value.to_string(), label))
            .collect::<Vec<_>>();
        let selected = overview::slowmode_options(initial.slowmode)
            .iter()
            .position(|(value, _)| *value == initial.slowmode)
            .unwrap_or(0);
        let slowmode = cx.new(|cx| {
            UiSelectState::new(
                SearchableVec::new(slowmode_options),
                Some(IndexPath::new(selected)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&name, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.notice = None;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&description, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = this.description.read(cx).value();
                    if value.encode_utf16().count() > 1024 {
                        let capped = overview::utf16_prefix(&value, 1024).to_owned();
                        this.description
                            .update(cx, |input, cx| input.set_value(capped, window, cx));
                    }
                    this.notice = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&slowmode, |this, _, event, cx| {
                if matches!(event, UiSelectEvent::Confirm(_)) {
                    this.notice = None;
                    cx.notify();
                }
            }),
        ];
        let mut permissions = BTreeMap::new();
        permissions.insert(
            None,
            PermissionDraft {
                value: channel.default_permissions.clone().unwrap_or_default(),
                open: false,
            },
        );
        for id in server.roles.keys() {
            permissions.insert(
                Some(id.clone()),
                PermissionDraft {
                    value: channel
                        .role_permissions
                        .get(id)
                        .cloned()
                        .unwrap_or_default(),
                    open: false,
                },
            );
        }
        Self {
            nsfw: initial.nsfw,
            initial,
            channel,
            server,
            policy,
            authorization,
            access,
            name,
            description,
            slowmode,
            permissions,
            page: Page::Overview,
            settings_index: true,
            compact_layout: f32::from(window.viewport_size().width) < 800.,
            frame_visible: true,
            confirmation: None,
            pending_focus: f32::from(window.viewport_size().width) < 800.,
            pending_target: None,
            busy: false,
            error: None,
            notice: None,
            request: None,
            active_permission: None,
            generation: 0,
            focus: cx.focus_handle(),
            navigation_focus: cx.focus_handle(),
            heading_focus: cx.focus_handle(),
            page_focus: std::array::from_fn(|_| cx.focus_handle()),
            confirmation_focus: cx.focus_handle(),
            feedback_generation: 0,
            feedback_task: None,
            _subscriptions: subscriptions,
        }
    }
    pub fn is_settings_open(&self) -> bool {
        self.frame_visible
    }
    pub fn visible(&self) -> bool {
        self.frame_visible || self.confirmation.is_some() || self.busy
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn channel_id(&self) -> &str {
        &self.channel.id
    }
    pub fn set_frame_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.frame_visible != visible {
            self.frame_visible = visible;
            cx.notify();
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmation.is_some() {
            self.confirmation_focus.focus(window, cx);
        } else if self.frame_visible && self.compact_layout && self.settings_index {
            self.page_focus[self.page.index()].focus(window, cx);
        } else if self.frame_visible || self.busy {
            self.focus.focus(window, cx);
        }
    }
    pub fn set_access(&mut self, access: ChannelAccess, cx: &mut Context<Self>) {
        if self.access == access {
            return;
        }
        self.access = access;
        if self.active_permission.is_some_and(|required| {
            self.authorization
                .require_channel_permission(&self.channel.id, required)
                .is_err()
        }) {
            self.request = None;
            self.busy = false;
            self.generation = self.generation.wrapping_add(1);
            self.active_permission = None;
            self.show_feedback(
                Some("Your permissions changed. The channel action was cancelled.".into()),
                None,
                cx,
            );
        }
        if self
            .confirmation
            .is_some_and(|action| !self.allowed_danger(action))
        {
            self.confirmation = None;
            self.pending_focus = self.frame_visible;
        }
        if self.frame_visible && !access.settings() {
            // Losing settings access dismisses the whole retained frame,
            // even while the compact settings index is open.
            self.cancel(cx);
            cx.emit(ChannelSettingsEvent::Close);
        } else if !self.frame_visible && !self.busy && self.confirmation.is_none() {
            cx.emit(ChannelSettingsEvent::Close);
        }
        cx.notify();
    }
    pub fn refresh(
        &mut self,
        channel: ClientChannel,
        server: ClientServer,
        cx: &mut Context<Self>,
    ) {
        if channel.id != self.channel.id || server.id != self.server.id {
            return;
        }
        // Hosts may synchronize retained overlays during rendering. Identical
        // snapshots must not create an entity notification/render loop.
        if serde_json::to_value(&channel).ok() == serde_json::to_value(&self.channel).ok()
            && serde_json::to_value(&server).ok() == serde_json::to_value(&self.server).ok()
        {
            return;
        }
        for id in server.roles.keys() {
            self.permissions
                .entry(Some(id.clone()))
                .or_insert_with(|| PermissionDraft {
                    value: channel
                        .role_permissions
                        .get(id)
                        .cloned()
                        .unwrap_or_default(),
                    open: false,
                });
        }
        self.permissions
            .retain(|id, _| id.as_ref().is_none_or(|id| server.roles.contains_key(id)));
        self.channel = channel;
        self.server = server;
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.confirmation.take().is_some() {
            self.pending_focus = self.frame_visible;
            if !self.frame_visible {
                cx.emit(ChannelSettingsEvent::Close);
            }
        } else if self.frame_visible && self.compact_layout && !self.settings_index {
            self.settings_index = true;
            self.pending_target = Some(self.page_focus[self.page.index()].clone());
            self.pending_focus = false;
        } else {
            self.request = None;
            self.generation = self.generation.wrapping_add(1);
            self.frame_visible = false;
            cx.emit(ChannelSettingsEvent::Close);
        }
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.active_permission = None;
        self.confirmation = None;
        self.frame_visible = false;
        self.pending_focus = false;
        self.pending_target = None;
        self.feedback_task = None;
        cx.notify();
    }
    fn perform(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy || !action.allowed(self.access) {
            return;
        }
        self.request = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.active_permission = Some(match &action {
            Action::Overview(_) | Action::Danger(ChannelDanger::Delete) => service::MANAGE_CHANNEL,
            Action::Permissions(..) => service::MANAGE_PERMISSIONS,
            Action::Danger(ChannelDanger::Reset) => service::MANAGE_MESSAGES,
        });
        let permissions_notice = matches!(&action, Action::Permissions(..));
        self.busy = true;
        self.error = None;
        self.notice = None;
        let id = self.channel.id.clone();
        let policy = self.policy.clone();
        let manager = self.authorization.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let result = spawn_core(Abortable::new(
                async move {
                    match action {
                        Action::Overview(draft) => service::update(&policy, &manager, &id, &draft)
                            .await
                            .map(Completion::Updated),
                        Action::Permissions(role, value) => service::set_permissions(
                            &policy,
                            &manager,
                            &id,
                            role.as_deref(),
                            &value,
                        )
                        .await
                        .map(Completion::Updated),
                        Action::Danger(ChannelDanger::Delete) => {
                            service::delete(&policy, &manager, &id)
                                .await
                                .map(|_| Completion::Deleted)
                        }
                        Action::Danger(ChannelDanger::Reset) => {
                            service::reset(&policy, &manager, &id)
                                .await
                                .map(Completion::Reset)
                        }
                    }
                },
                registration,
            ))
            .await;
            let Ok(result) = result else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                this.active_permission = None;
                match result {
                    Ok(Completion::Updated(channel)) => {
                        this.initial = Overview::from_channel(&channel);
                        this.channel = channel.clone();
                        this.show_feedback(
                            None,
                            Some(if permissions_notice {
                                "Channel permissions saved.".into()
                            } else {
                                "Channel settings saved.".into()
                            }),
                            cx,
                        );
                        cx.emit(ChannelSettingsEvent::Updated(channel));
                    }
                    Ok(Completion::Deleted) => {
                        cx.emit(ChannelSettingsEvent::Deleted(this.channel.id.clone()));
                        cx.emit(ChannelSettingsEvent::Close);
                    }
                    Ok(Completion::Reset(deleted)) => {
                        this.channel.last_message_id = None;
                        this.show_feedback(
                            None,
                            Some(if deleted == 1 {
                                "Deleted 1 message.".into()
                            } else {
                                format!("Deleted {deleted} messages.")
                            }),
                            cx,
                        );
                        cx.emit(ChannelSettingsEvent::Reset {
                            channel_id: this.channel.id.clone(),
                            deleted,
                        });
                        if !this.frame_visible {
                            cx.emit(ChannelSettingsEvent::Close);
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        this.show_feedback(Some(message.clone()), None, cx);
                        if !this.frame_visible {
                            cx.emit(ChannelSettingsEvent::Error(message));
                            cx.emit(ChannelSettingsEvent::Close);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.request = Some(Request { abort, _task: task });
        cx.notify();
    }
    fn show_feedback(
        &mut self,
        error: Option<String>,
        notice: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.feedback_task = None;
        self.feedback_generation = self.feedback_generation.wrapping_add(1);
        let generation = self.feedback_generation;
        let delay = if error.is_some() { 7000 } else { 4500 };
        self.error = error;
        self.notice = notice;
        self.feedback_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            this.update(cx, |this, cx| {
                if this.feedback_generation == generation {
                    this.error = None;
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
}
