//! The channel timeline: history fetch, live gateway messages, bottom-
//! anchored scrolling, message actions, and reply navigation.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::*;
use serde_json::Value;
use stonks_core::connection::GatewayEvent;
use stonks_core::http::HttpPolicy;
use stonks_core::messages::models::TimelineMessage;
use stonks_core::messages::normalize::normalize_message;
use stonks_ui::i18n::format_message_timestamp_ms;
use stonks_ui::{
    components::{ContextMenuEvent, UiContextMenu},
    Theme,
};

use super::media::image_viewer::ViewerImage;
use crate::shared::images::{attachment_url, Images};
pub type OpenImage = Rc<dyn Fn(ViewerImage, &mut Window, &mut App)>;
pub enum TimelineEvent {
    OpenImage(ViewerImage),
    ResetMedia,
    ActionUiChanged,
    NavigateChannel(String),
    Reply(Box<TimelineMessage>),
}
impl EventEmitter<TimelineEvent> for Timeline {}
use super::media::media_playback::MediaPlayback;
use super::media::media_player::{self, MediaPlayers, MediaPlayersEvent};
use self::reactions::{ReactionEvent, Reactions};
use crate::session::{Session, SessionEvent};
pub type RevealMedia = Rc<dyn Fn(String, &mut App)>;

mod history;
mod message_actions;
mod reactions;
mod presentation;

use history::{cache, fetch::HistoryRequest, paging};
use message_actions::{
    policy::MessageAction,
    surface::{ActionRequest, MessageActionUi},
};
use presentation::{author, layout, mentioned_user, navigation, system};

/// Millisecond timestamp encoded in the first 48 bits of a ULID (Crockford
/// base32), matching how the Solid timeline dates messages.
fn ulid_timestamp_ms(id: &str) -> Option<u64> {
    const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut value = 0_u64;
    for byte in id.as_bytes().get(..10)? {
        let digit = CROCKFORD.iter().position(|candidate| candidate == byte)? as u64;
        value = value.checked_mul(32)?.checked_add(digit)?;
    }
    Some(value)
}

fn message_timestamp(message: &TimelineMessage) -> String {
    ulid_timestamp_ms(&message.id)
        .map(|millis| format_message_timestamp_ms(millis as i64))
        .unwrap_or_default()
}

pub struct Timeline {
    cache: cache::ChannelCache,
    jump_target: Option<String>,
    action_menu: Entity<UiContextMenu>,
    action_target: Option<(String, Bounds<Pixels>)>,
    action_ui: Entity<MessageActionUi>,
    editor_row: Option<String>,
    pending_pins: HashSet<String>,
    _action_ui_events: Subscription,
    _action_ui_observe: Subscription,
    hovered_message: Option<String>,
    _action_menu_events: Subscription,
    navigation: navigation::State,
    session: Entity<Session>,
    policy: Arc<HttpPolicy>,
    channel_id: Option<String>,
    messages: Vec<TimelineMessage>,
    has_more: bool,
    loading: bool,
    error: Option<String>,
    mutation_error: Option<String>,
    generation: u64,
    list: ListState,
    painted_rows: HashMap<String, Bounds<Pixels>>,
    oldest_cursor: Option<String>,
    deleted_messages: HashSet<String>,
    pending_edits: HashMap<String, paging::PendingEdit>,
    request: Option<HistoryRequest>,
    insets: f32,
    composer_inset: f32,
    pinned_to_bottom: bool,
    can_read_history: bool,
    channel_intro: layout::ChannelIntro,
    author_headers: HashMap<String, author::Header>,
    mentioned_user: Option<String>,
    mention_opener: Option<WeakFocusHandle>,
    mention_focus: FocusHandle,
    revealed_spoilers: HashSet<String>,
    images: Entity<Images>,
    playback: Entity<MediaPlayback>,
    players: Entity<MediaPlayers>,
    reactions: Entity<Reactions>,
    reaction_events: Vec<GatewayEvent>,
    _reactions: Subscription,
    _reaction_state: Subscription,
    _player_layout: Subscription,
    _images_observe: Subscription,
    _subscription: Subscription,
    _session_observe: Subscription,
    _activation: Subscription,
}

impl Timeline {
    pub fn invalidate_emojis(&mut self, cx: &mut Context<Self>) {
        self.reactions
            .update(cx, |reactions, cx| reactions.invalidate_emojis(cx));
    }

    pub fn has_mentioned_user(&self) -> bool {
        self.mentioned_user.is_some()
    }
    pub fn close_mentioned_user(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mentioned_user.take().is_none() {
            return;
        }
        self.images.update(cx, |images, _| {
            images.set_sources(mentioned_user::SOURCE_SCOPE, std::iter::empty())
        });
        if self.mention_focus.contains_focused(window, cx) {
            if let Some(focus) = self.mention_opener.take().and_then(|focus| focus.upgrade()) {
                focus.focus(window, cx);
            } else {
                window.blur(cx);
            }
        }
        self.mention_opener = None;
        cx.notify();
    }
    fn open_mentioned_user(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| mentioned_user::user(snapshot, id).is_some())
        {
            return;
        }
        self.mention_opener = window.focused(cx).map(|focus| focus.downgrade());
        self.mentioned_user = Some(id.to_owned());
        cx.notify();
    }
    fn mentioned_user_view(&self, theme: Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let snapshot = session.snapshot.as_ref()?;
        let user = mentioned_user::user(snapshot, self.mentioned_user.as_deref()?)?;
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.as_deref());
        let profile = mentioned_user::Profile::new(user, autumn);
        let image = self.images.update(cx, |images, cx| {
            images.set_sources(
                mentioned_user::SOURCE_SCOPE,
                profile
                    .avatar_url
                    .iter()
                    .cloned()
                    .map(|url| (url, mentioned_user::AVATAR_EDGE)),
            );
            images.get(
                profile.avatar_url.as_deref(),
                mentioned_user::AVATAR_EDGE,
                cx,
            )
        });
        Some(mentioned_user::render(
            profile,
            image,
            theme,
            &self.mention_focus,
            cx.listener(|this, _, window, cx| this.close_mentioned_user(window, cx)),
        ))
    }
    pub fn forget_channel(&mut self, channel: &str, cx: &mut Context<Self>) {
        self.cache.invalidate(channel);
        if self.channel_id.as_deref() == Some(channel) {
            self.reset(cx);
        }
    }
    pub fn reload_channel(&mut self, channel: &str, cx: &mut Context<Self>) {
        self.cache.invalidate(channel);
        if self.channel_id.as_deref() == Some(channel) {
            self.reset(cx);
            self.activate_channel(channel.to_owned(), cx);
            self.fetch_page(None, cx);
        }
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.cache.clear();
        if self.channel_id.is_some() {
            self.reset(cx);
        }
    }
    fn activate_channel(&mut self, channel: String, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        self.can_read_history = session.can_channel_permission(&channel, 1 << 21);
        self.channel_intro = session
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.channels.iter().find(|entry| entry.id == channel))
            .map(|channel| layout::ChannelIntro {
                name: channel.name.clone(),
                description: channel.description.clone(),
            })
            .unwrap_or_default();
        self.playback.update(cx, |playback, cx| {
            playback.set_channel(Some(channel.clone()), cx)
        });
        self.channel_id = Some(channel);
        self.reactions.update(cx, |reactions, cx| {
            reactions.set_channel(self.channel_id.clone(), cx)
        });
    }
    pub fn receive_sent(&mut self, mut message: TimelineMessage, cx: &mut Context<Self>) {
        if self.channel_id.as_ref() != Some(&message.channel_id) || self.jump_target.is_some() {
            return;
        }
        if message.author_name == "Unknown user" {
            if let Some(snapshot) = self.session.read(cx).snapshot.as_ref() {
                let user = &snapshot.current_user;
                message.author_name = user
                    .display_name
                    .clone()
                    .unwrap_or_else(|| user.username.clone());
                message.author_avatar_url = user.avatar.as_ref().and_then(|file| {
                    stonks_core::media::avatar_url(
                        self.session
                            .read(cx)
                            .server
                            .as_ref()
                            .and_then(|server| server.endpoints.autumn.as_deref()),
                        &file.tag,
                        &file.id,
                    )
                });
            }
        }
        self.list.set_follow_mode(FollowMode::Tail);
        self.pinned_to_bottom = true;
        if let Err(index) = self
            .messages
            .binary_search_by(|old| old.id.cmp(&message.id))
        {
            self.messages.insert(index, message);
            self.list.splice(index + 1..index + 1, 1);
            cx.notify();
        }
    }
    pub fn new(
        session: Entity<Session>,
        policy: Arc<HttpPolicy>,
        images: Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let action_menu = cx.new(UiContextMenu::new);
        let action_menu_events =
            cx.subscribe_in(&action_menu, window, |this, _, event, window, cx| {
                // UiContextMenu emits Close before Select, so keep the target through Close.
                if let ContextMenuEvent::Select(id) = event {
                    if let Some((target, anchor)) = this.action_target.take() {
                        if let Some(message) = this.action_message(&target) {
                            if let Some(action) = this.message_actions(message, cx).resolve(id) {
                                this.dispatch_action(&target, action, anchor, window, cx);
                            }
                        }
                    }
                }
                cx.notify();
            });
        let action_ui = cx.new(|cx| MessageActionUi::new(window, cx));
        let image_source = images.clone();
        action_ui
            .read(cx)
            .picker()
            .clone()
            .update(cx, |picker, cx| {
                picker.set_image_provider(move |url, cx| {
                    image_source.update(cx, |images, cx| images.get(Some(url), 48, cx))
                });
                picker.refresh_images(cx);
            });
        let action_ui_events = cx.subscribe_in(
            &action_ui,
            window,
            |this, _, request: &ActionRequest, window, cx| {
                this.submit_action(request, window, cx);
            },
        );
        let action_ui_observe = cx.observe(&action_ui, |this, ui, cx| {
            let next = ui
                .read(cx)
                .target()
                .filter(|id| ui.read(cx).editing(id))
                .map(str::to_owned);
            for id in this.editor_row.iter().chain(next.iter()) {
                if let Ok(index) = this.messages.binary_search_by(|message| message.id.cmp(id)) {
                    this.list.remeasure_items(index + 1..index + 2);
                }
            }
            this.editor_row = next;
            cx.emit(TimelineEvent::ActionUiChanged);
            cx.notify();
        });
        let playback = cx.new(|cx| MediaPlayback::new(session.clone(), cx));
        let players = cx.new(|cx| MediaPlayers::new(playback.clone(), policy.clone(), window, cx));
        let reactions = cx.new(|cx| Reactions::new(session.clone(), policy.clone(), cx));
        let reaction_subscription = cx.subscribe(&reactions, |this, _, event, cx| {
            match event {
                ReactionEvent::MetadataChanged => {
                    this.refresh_action_emojis(cx);
                    this.list.remeasure_items(1..this.list.item_count())
                }
                ReactionEvent::Membership {
                    channel,
                    message,
                    emoji,
                    user,
                    present,
                } if this.channel_id.as_ref() == Some(channel) => {
                    if let Some(index) = this.messages.iter().position(|entry| &entry.id == message)
                    {
                        reactions::set_membership(
                            &mut this.messages[index].reactions,
                            emoji,
                            user,
                            *present,
                        );
                        this.list.remeasure_items(index + 1..index + 2);
                    }
                }
                ReactionEvent::Failed { channel, error }
                    if this.channel_id.as_ref() == Some(channel) =>
                {
                    this.mutation_error = Some(error.clone());
                    this.list.remeasure_items(0..1);
                }
                _ => {}
            }
            cx.notify();
        });
        let reaction_state = cx.observe(&reactions, |_, _, cx| cx.notify());
        let player_layout = cx.subscribe(&players, |this, _, _: &MediaPlayersEvent, cx| {
            if this.pinned_to_bottom && this.jump_target.is_none() {
                this.list.set_follow_mode(FollowMode::Tail);
            }
            this.list.remeasure_items(1..this.list.item_count());
            cx.notify();
        });
        let list = ListState::new(1, ListAlignment::Bottom, px(300.));
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.session.update(cx, |session, cx| {
                    session.observe_read_view(this.channel_id.as_deref(), None, false, cx)
                });
            }
            cx.notify();
        });
        list.set_follow_mode(FollowMode::Tail);
        let timeline = cx.entity().downgrade();
        list.set_scroll_handler(move |_, window, cx| {
            // The handler runs inside ListState's borrow; defer mutations.
            let timeline = timeline.clone();
            window.defer(cx, move |window, cx| {
                timeline
                    .update(cx, |this, cx| {
                        this.navigation.stop_scroll();
                        if this.action_ui.read(cx).action() == Some(MessageAction::React) {
                            this.clear_action_ui(window, cx);
                        }
                        this.pinned_to_bottom = layout::within(
                            this.distance_to_latest(),
                            layout::PINNED_THRESHOLD,
                            this.list.is_following_tail(),
                        );
                        let near_top = layout::near_start(-f32::from(
                            this.list.scroll_px_offset_for_scrollbar().y,
                        ));
                        if near_top && this.error.is_none() {
                            this.load_older(cx);
                        }
                        cx.notify();
                    })
                    .ok();
            });
        });
        let images_observe = cx.observe(&images, |this, _, cx| {
            if this.pinned_to_bottom && this.jump_target.is_none() {
                this.list.set_follow_mode(FollowMode::Tail);
            }
            this.list.remeasure_items(1..this.list.item_count());
            cx.notify();
        });
        let session_observe = cx.observe(&session, |this, session, cx| {
            if session.read(cx).snapshot.is_none()
                || this
                    .channel_id
                    .as_ref()
                    .is_some_and(|channel| !session.read(cx).can_view_text_channel(channel))
            {
                this.clear(cx);
            } else if let Some(channel) = this.channel_id.clone() {
                let can_read = session.read(cx).can_channel_permission(&channel, 1 << 21);
                if can_read != this.can_read_history {
                    this.cache.invalidate(&channel);
                    this.reset(cx);
                    this.activate_channel(channel, cx);
                    if can_read {
                        this.fetch_page(None, cx);
                    }
                } else {
                    let intro = session
                        .read(cx)
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot.channels.iter().find(|entry| entry.id == channel)
                        })
                        .map(|channel| layout::ChannelIntro {
                            name: channel.name.clone(),
                            description: channel.description.clone(),
                        })
                        .unwrap_or_default();
                    if intro != this.channel_intro {
                        this.channel_intro = intro;
                        this.list.remeasure_items(0..1);
                    }
                }
            }
            if this.mentioned_user.as_deref().is_some_and(|id| {
                !session
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| mentioned_user::user(snapshot, id).is_some())
            }) {
                this.mentioned_user = None;
                this.mention_opener = None;
                this.images.update(cx, |images, _| {
                    images.set_sources(mentioned_user::SOURCE_SCOPE, std::iter::empty())
                });
            }
            cx.notify();
        });
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Gateway(event) => this.apply_gateway_event(event.clone(), cx),
        });
        cx.on_release(|this, cx| {
            this.images.update(cx, |images, _| {
                images.set_sources(mentioned_user::SOURCE_SCOPE, std::iter::empty())
            })
        })
        .detach();
        Self {
            cache: Default::default(),
            jump_target: None,
            action_menu,
            action_target: None,
            action_ui,
            editor_row: None,
            pending_pins: HashSet::new(),
            _action_ui_events: action_ui_events,
            _action_ui_observe: action_ui_observe,
            hovered_message: None,
            _action_menu_events: action_menu_events,
            navigation: Default::default(),
            session,
            policy,
            channel_id: None,
            messages: Vec::new(),
            has_more: false,
            loading: false,
            error: None,
            mutation_error: None,
            generation: 0,
            list,
            painted_rows: HashMap::new(),
            oldest_cursor: None,
            deleted_messages: HashSet::new(),
            pending_edits: HashMap::new(),
            request: None,
            insets: 724.,
            composer_inset: layout::COMPOSER_INSET,
            pinned_to_bottom: true,
            can_read_history: false,
            channel_intro: Default::default(),
            author_headers: HashMap::new(),
            mentioned_user: None,
            mention_opener: None,
            mention_focus: cx.focus_handle(),
            revealed_spoilers: HashSet::new(),
            images,
            playback,
            players,
            reactions,
            reaction_events: Vec::new(),
            _reactions: reaction_subscription,
            _reaction_state: reaction_state,
            _player_layout: player_layout,
            _images_observe: images_observe,
            _subscription: subscription,
            _session_observe: session_observe,
            _activation: activation,
        }
    }

    pub fn set_channel(&mut self, channel_id: String, cx: &mut Context<Self>) {
        if !self.session.read(cx).can_view_text_channel(&channel_id) {
            self.clear(cx);
            return;
        }
        if self.channel_id.as_deref() == Some(channel_id.as_str()) {
            return;
        }
        if let Some(previous) = self.channel_id.clone() {
            if self.can_read_history
                && !self.loading
                && self.error.is_none()
                && self.jump_target.is_none()
            {
                self.cache.store(
                    previous,
                    cache::CachedChannel {
                        messages: self.messages.clone(),
                        has_more: self.has_more,
                        oldest_cursor: self.oldest_cursor.clone(),
                        offset: self.list.logical_scroll_top(),
                        tail: self.list.is_following_tail(),
                    },
                );
            }
        }
        self.reset(cx);
        let cached = self.cache.take(&channel_id);
        self.activate_channel(channel_id, cx);
        if !self.can_read_history {
            cx.notify();
            return;
        }
        if let Some(cached) = cached {
            self.messages = cached.messages;
            self.has_more = cached.has_more;
            self.oldest_cursor = cached.oldest_cursor;
            self.list.reset(self.messages.len() + 1);
            self.list.set_follow_mode(FollowMode::Tail);
            if !cached.tail {
                self.list.scroll_to(cached.offset);
                self.pinned_to_bottom = false;
            }
            cx.notify();
        } else {
            self.fetch_page(None, cx);
        }
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        cx.emit(TimelineEvent::ResetMedia);
        self.players.update(cx, |players, cx| players.reset(cx));
        self.playback
            .update(cx, |playback, cx| playback.set_channel(None, cx));
        self.session.update(cx, |session, cx| {
            session.observe_read_view(None, None, false, cx)
        });
        self.request = None;
        self.jump_target = None;
        self.action_target = None;
        self.pending_pins.clear();
        self.hovered_message = None;
        self.navigation.reset();
        self.channel_id = None;
        self.reactions
            .update(cx, |reactions, cx| reactions.set_channel(None, cx));
        self.reaction_events.clear();
        self.channel_intro = Default::default();
        self.author_headers.clear();
        self.mentioned_user = None;
        self.mention_opener = None;
        self.images.update(cx, |images, _| {
            images.set_sources(mentioned_user::SOURCE_SCOPE, std::iter::empty())
        });
        self.can_read_history = false;
        self.pinned_to_bottom = true;
        self.messages.clear();
        self.painted_rows.clear();
        self.revealed_spoilers.clear();
        self.has_more = false;
        self.loading = false;
        self.oldest_cursor = None;
        self.deleted_messages.clear();
        self.pending_edits.clear();
        self.error = None;
        self.mutation_error = None;
        self.generation = self.generation.wrapping_add(1);
        self.list.reset(1);
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    /// Search/pin results use the same nearby-history endpoint as the Solid timeline.
    pub fn jump_to(&mut self, message_id: String, cx: &mut Context<Self>) {
        if let Some(index) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        {
            self.focus_message(message_id, index, cx);
            return;
        }
        let channel = self.channel_id.clone();
        self.reset(cx);
        let Some(channel) = channel else {
            return;
        };
        self.activate_channel(channel, cx);
        if !self.can_read_history {
            cx.notify();
            return;
        }
        self.jump_target = Some(message_id);
        self.fetch_page(None, cx);
    }

    fn apply_gateway_event(&mut self, event: GatewayEvent, cx: &mut Context<Self>) {
        // Inactive cached channels must never hide live edits/deletions or new messages.
        if matches!(
            event.event_type.as_str(),
            "ChannelCreate"
                | "ChannelUpdate"
                | "ChannelDelete"
                | "ChannelGroupJoin"
                | "ChannelGroupLeave"
                | "ServerUpdate"
                | "ServerDelete"
                | "ServerRoleUpdate"
                | "ServerRoleDelete"
                | "ServerMemberUpdate"
                | "ServerMemberLeave"
        ) {
            self.cache.clear();
        }
        if event.event_type.starts_with("Message") || event.event_type == "BulkMessageDelete" {
            if let Some(channel) = event
                .data
                .get("channel")
                .or_else(|| event.data.get("channel_id"))
                .or_else(|| event.data.get("channelId"))
                .and_then(Value::as_str)
            {
                self.cache.invalidate(channel);
            }
        }
        let Some(channel_id) = self.channel_id.clone() else {
            return;
        };
        if self.loading
            && matches!(
                event.event_type.as_str(),
                "MessageReact" | "MessageUnreact" | "MessageRemoveReaction" | "MessageUpdate"
            )
            && reactions::event_channel(&event) == Some(channel_id.as_str())
        {
            self.reaction_events.push(event.clone());
        }
        if matches!(
            event.event_type.as_str(),
            "MessageReact" | "MessageUnreact" | "MessageRemoveReaction" | "MessageUpdate"
        ) {
            for (index, message) in self.messages.iter_mut().enumerate() {
                if reactions::apply_event(message, &event) {
                    self.list.remeasure_items(index + 1..index + 2);
                    cx.notify();
                }
            }
        }
        match event.event_type.as_str() {
            "Message" => {
                // Nearby history has an unloaded gap to the latest messages.
                // Keep it explicit until Jump Latest fetches a contiguous page.
                if self.jump_target.is_some() {
                    return;
                }
                if event.data.get("channel").and_then(Value::as_str) != Some(channel_id.as_str()) {
                    return;
                }
                let (users, members) = self.author_context(cx);
                let autumn = self
                    .session
                    .read(cx)
                    .server
                    .as_ref()
                    .and_then(|server| server.endpoints.autumn.clone());
                let Ok(message) =
                    normalize_message(&event.data, &users, &members, autumn.as_deref())
                else {
                    return;
                };
                if !system::is_visible(&message) {
                    return;
                }
                if self.deleted_messages.contains(&message.id) {
                    return;
                }
                let Err(index) = self
                    .messages
                    .binary_search_by(|existing| existing.id.cmp(&message.id))
                else {
                    return;
                };
                if layout::within(
                    self.distance_to_latest(),
                    layout::NEAR_BOTTOM_THRESHOLD,
                    self.list.is_following_tail(),
                ) {
                    self.list.set_follow_mode(FollowMode::Tail);
                    self.pinned_to_bottom = true;
                }
                self.messages.insert(index, message);
                self.list.splice(index + 1..index + 1, 1);
                if index + 2 < self.list.item_count() {
                    self.list.remeasure_items(index + 2..index + 3);
                }
                cx.notify();
            }
            "MessageDelete" | "BulkMessageDelete" => {
                if event.data.get("channel").and_then(Value::as_str) != Some(channel_id.as_str()) {
                    return;
                }
                let ids = event.data.get("id").into_iter().chain(
                    event
                        .data
                        .get("ids")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                );
                let mut removed = false;
                for id in ids.filter_map(Value::as_str) {
                    self.deleted_messages.insert(id.to_owned());
                    if let Some(index) = self.messages.iter().position(|message| message.id == id) {
                        removed = true;
                        self.messages.remove(index);
                        self.list.splice(index + 1..index + 2, 0);
                        if index + 1 < self.list.item_count() {
                            self.list.remeasure_items(index + 1..index + 2);
                        }
                    }
                }
                if let Some(before) = paging::deletion_probe(
                    removed,
                    self.messages.len(),
                    self.has_more,
                    self.oldest_cursor.as_deref(),
                )
                .map(str::to_owned)
                {
                    self.fetch_page(Some(before), cx);
                }
                if removed {
                    cx.notify();
                }
            }
            "MessageUpdate" => {
                if event.data.get("channel").and_then(Value::as_str) != Some(channel_id.as_str()) {
                    return;
                }
                let Some(id) = event.data.get("id").and_then(Value::as_str) else {
                    return;
                };
                let mut data = event
                    .data
                    .get("data")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                if event
                    .data
                    .get("clear")
                    .and_then(Value::as_array)
                    .is_some_and(|clear| clear.iter().any(|value| value.as_str() == Some("Embeds")))
                {
                    data["embeds"] = serde_json::json!([]);
                }
                if let Some(index) = self.messages.iter().position(|message| message.id == id) {
                    let message = &mut self.messages[index];
                    paging::apply_edit(message, &data);
                    self.list.remeasure_items(index + 1..index + 2);
                    cx.notify();
                } else if self.loading {
                    // Preserve updates to messages still in the REST response.
                    self.pending_edits
                        .entry(id.to_owned())
                        .or_default()
                        .update(&data);
                }
            }
            "MessageAppend" => {
                if event.data.get("channel").and_then(Value::as_str) != Some(channel_id.as_str()) {
                    return;
                }
                let Some(id) = event.data.get("id").and_then(Value::as_str) else {
                    return;
                };
                let Some(embeds) = event
                    .data
                    .get("append")
                    .and_then(|append| append.get("embeds"))
                    .and_then(Value::as_array)
                else {
                    return;
                };
                if let Some(index) = self.messages.iter().position(|message| message.id == id) {
                    if paging::append_embeds(&mut self.messages[index], embeds) {
                        self.list.remeasure_items(index + 1..index + 2);
                        cx.notify();
                    }
                } else if self.loading {
                    self.pending_edits
                        .entry(id.to_owned())
                        .or_default()
                        .append(embeds);
                }
            }
            _ => {}
        }
    }

    /// Users/members maps for normalize_message's author resolution,
    /// rebuilt from the session snapshot per event.
    fn author_context(&self, cx: &App) -> (HashMap<String, Value>, HashMap<String, Value>) {
        let session = self.session.read(cx);
        let mut users = HashMap::new();
        let mut members = HashMap::new();
        if let Some(snapshot) = session.snapshot.as_ref() {
            for user in &snapshot.users {
                if let Ok(value) = serde_json::to_value(user) {
                    users.insert(user.id.clone(), value);
                }
            }
            for member in &snapshot.members {
                if let Ok(value) = serde_json::to_value(member) {
                    members.insert(member.id.user.clone(), value);
                }
            }
        }
        (users, members)
    }
}
