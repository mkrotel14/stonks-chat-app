//! The authenticated shell: server rail, channel sidebar, timeline pane and
//! member list. Mirrors the Solid app's `ShellLayout` grid
//! (56 / 292 / 1fr / 280) and `ChannelSidebar`/`ServerRail`/`MemberList`
//! modules. The pane header doubles as the window titlebar under
//! client-side decorations.

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_ui::components::{Button, ButtonVariant};
use stonks_ui::i18n::t;
use stonks_ui::icons::Icon;
use stonks_ui::Theme;

use super::conversation::media::image_viewer::ImageViewer;
use super::conversation::message_panel::{MessagePanel, PanelMode};
use super::conversation::timeline::Timeline;
use super::server_settings::{ServerSettings, ServerSettingsEvent};
use crate::session::Session;
use crate::shared::images::Images;
use std::sync::Arc;
use stonks_core::http::HttpPolicy;

mod compact;
mod conversation;
#[path = "../../desktop_notifications.rs"]
mod desktop_notifications;
mod home;
mod interactions;
mod layout;
mod motion;
mod navigation;
mod notification_routing;
mod panels;
mod people;
mod render;
pub(crate) mod shortcuts;
mod update;
mod wiring;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Explorer {
    Channels,
    Members,
}
struct SidebarDrag {
    member: bool,
    start_x: f32,
    start_width: f32,
}

const RAIL_WIDTH: f32 = 56.;
const MEMBERS_WIDTH: f32 = 280.;
const HEADER_HEIGHT: f32 = 66.;

pub struct ShellView {
    image_viewer: Entity<ImageViewer>,
    _image_viewer_sub: Subscription,
    _image_viewer_observe: Subscription,
    _timeline_media: Subscription,
    context_menu: Entity<stonks_ui::components::UiContextMenu>,
    menu_actions: std::collections::HashMap<String, interactions::MenuAction>,
    _context_sub: Subscription,
    channel_settings: Option<Entity<super::channel_settings::ChannelSettings>>,
    channel_sub: Option<Subscription>,
    member_actions: Entity<people::member_actions::MemberActions>,
    _member_sub: Subscription,
    notifications: Entity<navigation::notifications::Notifications>,
    _notifications_observe: Subscription,
    _live_notifications: Subscription,
    desktop_delivery: Option<notification_routing::DesktopDelivery>,
    member_bounds: std::collections::HashMap<String, Bounds<Pixels>>,
    shell_error: Option<String>,
    voice: Entity<super::voice::Voice>,
    _voice_observe: Subscription,
    policy: Arc<HttpPolicy>,
    message_panel: Entity<MessagePanel>,
    _panel_sub: Subscription,
    _panel_observe: Subscription,
    settings: Option<Entity<ServerSettings>>,
    settings_sub: Option<Subscription>,
    account_settings: Option<Entity<super::settings::Settings>>,
    account_settings_sub: Option<Subscription>,
    account_settings_observe: Option<Subscription>,
    update_details: Option<Entity<super::app::updater::details::Details>>,
    overlay_focus: Option<FocusHandle>,
    overlay_container: FocusHandle,
    drawer: Option<Explorer>,
    focused: bool,
    compact_nav: compact::CompactNav,
    wide_selection: Option<(bool, Option<String>, Option<String>)>,
    channel_width: f32,
    member_width: f32,
    drag: Option<SidebarDrag>,
    identity: Option<(String, String)>,
    session: Entity<Session>,
    timeline: Entity<Timeline>,
    images: Entity<Images>,
    _images_observe: Subscription,
    selected_server: Option<String>,
    selected_channel: Option<String>,
    _observe: Subscription,
    _preferences_observe: Subscription,
    panel_entrance: u64,
    collapsed_groups: std::collections::HashSet<String>,
    collapsed_channels: std::collections::HashSet<String>,
    composer: Entity<super::conversation::composer::Composer>,
    _composer_sub: Subscription,
    _typing_sub: Subscription,
    _typing_events: Subscription,
    show_members: bool,
    server_rail: layout::ServerRail,
    show_channels: bool,
    pub(super) home_messages: super::home::messages::Messages,
    home_friends: Option<home::MountedFriends>,
    wide_friends: bool,
    home: bool,
}
pub struct SignOut;
impl EventEmitter<SignOut> for ShellView {}
