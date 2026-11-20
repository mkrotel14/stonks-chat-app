//! Native server settings with source overview, role, emoji and invite workflows.
use crate::runtime::spawn_core;
use crate::shared::images::Images;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::sync::Arc;
use stonks_core::{
    connection::{ClientChannel, ClientServer},
    http::HttpPolicy,
    media,
    server_settings::{self as service, Overview, ServerEmoji, ServerInvite, SYSTEM_EVENTS},
};
use stonks_ui::{
    components::select::{IndexPath, SearchableVec},
    components::{
        avatar, input_field, settings_field, settings_row, settings_section, Button, ButtonVariant,
        Feedback, FeedbackTone, SettingsInput, UiSelect, UiSelectEvent, UiSelectOption,
        UiSelectState,
    },
    i18n::t,
    icons::Icon,
    Theme,
};
mod actions;
mod coordination;
mod dialogs;
mod emoji;
mod extra;
mod feedback;
mod invites;
mod overview;
mod resources;
mod roles;
mod sync;
mod view;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Emojis,
    Roles,
    Invites,
}
impl Page {
    fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Emojis => 1,
            Self::Roles => 2,
            Self::Invites => 3,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Overview => "serverSettings.overview",
            Self::Emojis => "serverSettings.emojis",
            Self::Roles => "serverSettings.roles",
            Self::Invites => "serverSettings.invites",
        }
    }
}
pub enum ServerSettingsEvent {
    Close,
    Updated(ClientServer),
    EmojisChanged,
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
pub struct ServerSettings {
    extra: extra::Extra,
    server: ClientServer,
    channels: Vec<ClientChannel>,
    can_manage: bool,
    policy: Arc<HttpPolicy>,
    images: Entity<Images>,
    icon_url: Option<String>,
    autumn: Option<String>,
    deployment: Option<String>,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    systems: Vec<Entity<UiSelectState>>,
    initial: Overview,
    page: Page,
    settings_index: bool,
    compact_layout: bool,
    busy: bool,
    save_request: Option<Request>,
    list_request: Option<Request>,
    generation: u64,
    save_generation: u64,
    loading: bool,
    list_error: Option<String>,
    emojis: Option<Vec<ServerEmoji>>,
    invites: Option<Vec<ServerInvite>>,
    error: Option<String>,
    notice: Option<String>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ServerSettingsEvent> for ServerSettings {}
impl ServerSettings {
    pub fn new(
        server: ClientServer,
        channels: Vec<ClientChannel>,
        can_manage: bool,
        policy: Arc<HttpPolicy>,
        images: Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = Overview::from_server(&server);
        let name = cx
            .new(|cx| InputState::new(window, cx).validate(|value, _| value.chars().count() <= 32));
        name.update(cx, |state, cx| {
            state.set_value(initial.name.clone(), window, cx)
        });
        let description = cx.new(|cx| TextareaState::new(window, cx));
        description.update(cx, |state, cx| {
            state.set_value(initial.description.clone(), window, cx)
        });
        let mut subscriptions = vec![
            cx.subscribe(&name, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    if this.draft(cx) != this.initial {
                        this.notice = None;
                    }
                    cx.notify();
                }
            }),
            cx.subscribe(&description, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    if this.draft(cx) != this.initial {
                        this.notice = None;
                    }
                    cx.notify();
                }
            }),
            cx.observe(&images, |_, _, cx| cx.notify()),
        ];
        let mut systems = Vec::new();
        for key in SYSTEM_EVENTS {
            let mut options = vec![UiSelectOption::new("", t("serverSettings.disabled"))];
            options.extend(
                channels
                    .iter()
                    .filter(|channel| overview::text_channel(channel, &server.id))
                    .map(|channel| {
                        UiSelectOption::new(
                            channel.id.clone(),
                            format!("# {}", channel.name.as_deref().unwrap_or("")),
                        )
                    }),
            );
            let selected = initial
                .system_messages
                .get(key)
                .map(String::as_str)
                .unwrap_or("");
            // Preserve a deleted/inaccessible channel until the user explicitly changes it.
            // The draft validator prevents publishing that stale selection.
            if !selected.is_empty()
                && !channels.iter().any(|channel| {
                    channel.id == selected && overview::text_channel(channel, &server.id)
                })
            {
                options.push(UiSelectOption::new(
                    selected.to_owned(),
                    t("serverSettings.unknownChannel"),
                ));
            }
            let selected_index = if selected.is_empty() {
                0
            } else {
                channels
                    .iter()
                    .filter(|channel| overview::text_channel(channel, &server.id))
                    .position(|channel| channel.id == selected)
                    .map(|index| index + 1)
                    .unwrap_or(options.len() - 1)
            };
            let select = cx.new(|cx| {
                UiSelectState::new(
                    SearchableVec::new(options),
                    Some(IndexPath::new(selected_index)),
                    window,
                    cx,
                )
            });
            subscriptions.push(cx.subscribe(&select, |this, _, event, cx| {
                if matches!(event, UiSelectEvent::Confirm(_)) {
                    if this.draft(cx) != this.initial {
                        this.notice = None;
                    }
                    cx.notify();
                }
            }));
            systems.push(select);
        }
        let context = policy.authenticated_context().ok();
        let autumn = context
            .as_ref()
            .and_then(|context| context.server.endpoints.autumn.clone());
        let deployment = context
            .as_ref()
            .map(|context| context.server.deployment_url.clone());
        let icon_url = server
            .icon
            .as_ref()
            .and_then(|file| media::file_url(autumn.as_deref(), &file.tag, &file.id));
        // Entity release also covers backdrop/Escape dismissal owned by the host.
        cx.on_release(|this, cx| {
            this.images.update(cx, |images, _| {
                images.set_sources("server-settings", Vec::<(String, u32)>::new())
            });
        })
        .detach();
        let extra = extra::Extra::new(window, cx);
        Self {
            extra,
            server,
            channels,
            can_manage,
            policy,
            images,
            icon_url,
            autumn,
            deployment,
            name,
            description,
            systems,
            initial,
            page: Page::Overview,
            settings_index: true,
            compact_layout: f32::from(window.viewport_size().width) < 800.,
            busy: false,
            save_request: None,
            list_request: None,
            generation: 0,
            save_generation: 0,
            loading: false,
            list_error: None,
            emojis: None,
            invites: None,
            error: None,
            notice: None,
            _subscriptions: subscriptions,
        }
    }
    pub fn server_id(&self) -> &str {
        &self.server.id
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.extra.frame_visible && self.compact_layout && self.settings_index {
            self.extra.navigation_focus[self.page.index()].focus(window, cx);
        } else {
            self.name.update(cx, |input, cx| input.focus(window, cx));
        }
    }
}
