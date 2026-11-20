//! Native SVG icons, including Lucide (ISC — see `assets/icons/LICENSE`)
//! and custom glyphs from the Tauri app.
//!
//! Lucide strokes use `currentColor`; we substitute the theme color into the
//! source at render time so icons tint exactly like the CSS originals.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui_kit::prelude::Styled;
use gpui_kit::{img, px, Image, ImageFormat, IntoElement, Rgba, SharedString};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    ArrowLeftRight,
    ArrowUpDown,
    Ban,
    BellOff,
    Check,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    CircleX,
    Copy,
    Download,
    Ellipsis,
    Hash,
    LockOpen,
    Menu,
    MessageCircle,
    Minus,
    PanelLeft,
    PanelLeftOpen,
    Reply,
    Pencil,
    Trash,
    Pin,
    Plus,
    Search,
    Send,
    Smile,
    SendHorizontal,
    Settings,
    Server,
    Square,
    UserMinus,
    UserPlus,
    Users,
    Mic,
    MicOff,
    Headphones,
    HeadphonesOff,
    PhoneOff,
    Video,
    CameraOutline,
    MonitorUp,
    GripHorizontal,
    Volume2,
    RailVoice,
    Play,
    Pause,
    Music,
    VolumeX,
    ExternalLink,
    Maximize,
    Minimize,
    SessionMobile,
    SessionBrowser,
    SessionDesktop,
    SessionGeneric,
    X,
}

impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Icon::ArrowLeftRight => "arrow-left-right",
            Icon::ArrowUpDown => "arrow-up-down",
            Icon::Ban => "ban",
            Icon::BellOff => "bell-off",
            Icon::Check => "check",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronRight => "chevron-right",
            Icon::CircleX => "circle-x",
            Icon::Copy => "copy",
            Icon::Download => "download",
            Icon::Ellipsis => "ellipsis",
            Icon::Hash => "hash",
            Icon::LockOpen => "lock-open",
            Icon::Menu => "menu",
            Icon::MessageCircle => "message-circle",
            Icon::Minus => "minus",
            Icon::PanelLeft => "panel-left",
            Icon::PanelLeftOpen => "panel-left-open",
            Icon::Reply => "reply",
            Icon::Pencil => "pencil",
            Icon::Trash => "trash",
            Icon::Pin => "pin",
            Icon::Plus => "plus",
            Icon::Search => "search",
            Icon::Send => "send",
            Icon::Smile => "smile",
            Icon::SendHorizontal => "send-horizontal",
            Icon::Settings => "settings",
            Icon::Server => "server",
            Icon::Square => "square",
            Icon::UserMinus => "user-minus",
            Icon::UserPlus => "user-plus",
            Icon::Users => "users",
            Icon::Mic => "mic",
            Icon::MicOff => "mic-off",
            Icon::Headphones => "headphones",
            Icon::HeadphonesOff => "headphones-off",
            Icon::PhoneOff => "phone-off",
            Icon::Video => "video",
            Icon::CameraOutline => "camera-outline",
            Icon::MonitorUp => "monitor-up",
            Icon::GripHorizontal => "grip-horizontal",
            Icon::Volume2 => "volume-2",
            Icon::RailVoice => "rail-voice",
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Music => "music",
            Icon::VolumeX => "volume-x",
            Icon::ExternalLink => "external-link",
            Icon::Maximize => "maximize",
            Icon::Minimize => "minimize",
            Icon::SessionMobile => "session-mobile",
            Icon::SessionBrowser => "session-browser",
            Icon::SessionDesktop => "session-desktop",
            Icon::SessionGeneric => "session-generic",
            Icon::X => "x",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Icon::Send => include_str!("../assets/icons/send.svg"),
            Icon::Smile => include_str!("../assets/icons/smile.svg"),
            Icon::SendHorizontal => include_str!("../assets/icons/send-horizontal.svg"),
            Icon::ChevronDown => include_str!("../assets/icons/chevron-down.svg"),
            Icon::ChevronUp => include_str!("../assets/icons/chevron-up.svg"),
            Icon::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            Icon::ArrowLeftRight => include_str!("../assets/icons/arrow-left-right.svg"),
            Icon::ArrowUpDown => include_str!("../assets/icons/arrow-up-down.svg"),
            Icon::Ban => include_str!("../assets/icons/ban.svg"),
            Icon::BellOff => include_str!("../assets/icons/bell-off.svg"),
            Icon::Check => include_str!("../assets/icons/check.svg"),
            Icon::CircleX => include_str!("../assets/icons/circle-x.svg"),
            Icon::Copy => include_str!("../assets/icons/copy.svg"),
            Icon::Download => include_str!("../assets/icons/download.svg"),
            Icon::Ellipsis => include_str!("../assets/icons/ellipsis.svg"),
            Icon::Hash => include_str!("../assets/icons/hash.svg"),
            Icon::LockOpen => include_str!("../assets/icons/lock-open.svg"),
            Icon::Menu => include_str!("../assets/icons/menu.svg"),
            Icon::MessageCircle => include_str!("../assets/icons/message-circle.svg"),
            Icon::Minus => include_str!("../assets/icons/minus.svg"),
            Icon::PanelLeft => include_str!("../assets/icons/panel-left.svg"),
            Icon::PanelLeftOpen => include_str!("../assets/icons/panel-left-open.svg"),
            Icon::Reply => include_str!("../assets/icons/reply.svg"),
            Icon::Pencil => include_str!("../assets/icons/pencil.svg"),
            Icon::Trash => include_str!("../assets/icons/trash.svg"),
            Icon::Pin => include_str!("../assets/icons/pin.svg"),
            Icon::Plus => include_str!("../assets/icons/plus.svg"),
            Icon::Search => include_str!("../assets/icons/search.svg"),
            Icon::Settings => include_str!("../assets/icons/settings.svg"),
            Icon::Server => include_str!("../assets/icons/server.svg"),
            Icon::Square => include_str!("../assets/icons/square.svg"),
            Icon::UserMinus => include_str!("../assets/icons/user-minus.svg"),
            Icon::UserPlus => include_str!("../assets/icons/user-plus.svg"),
            Icon::Users => include_str!("../assets/icons/users.svg"),
            Icon::Mic => include_str!("../assets/icons/mic.svg"),
            Icon::MicOff => include_str!("../assets/icons/mic-off.svg"),
            Icon::Headphones => include_str!("../assets/icons/headphones.svg"),
            Icon::HeadphonesOff => include_str!("../assets/icons/headphones-off.svg"),
            Icon::PhoneOff => include_str!("../assets/icons/phone-off.svg"),
            Icon::Video => include_str!("../assets/icons/video.svg"),
            Icon::CameraOutline => include_str!("../assets/icons/camera-outline.svg"),
            Icon::MonitorUp => include_str!("../assets/icons/monitor-up.svg"),
            Icon::GripHorizontal => include_str!("../assets/icons/grip-horizontal.svg"),
            Icon::Volume2 => include_str!("../assets/icons/volume-2.svg"),
            Icon::RailVoice => include_str!("../assets/icons/rail-voice.svg"),
            Icon::Play => include_str!("../assets/icons/play.svg"),
            Icon::Pause => include_str!("../assets/icons/pause.svg"),
            Icon::Music => include_str!("../assets/icons/music.svg"),
            Icon::VolumeX => include_str!("../assets/icons/volume-x.svg"),
            Icon::ExternalLink => include_str!("../assets/icons/external-link.svg"),
            Icon::Maximize => include_str!("../assets/icons/maximize.svg"),
            Icon::Minimize => include_str!("../assets/icons/minimize.svg"),
            Icon::SessionMobile => include_str!("../assets/icons/session-mobile.svg"),
            Icon::SessionBrowser => include_str!("../assets/icons/session-browser.svg"),
            Icon::SessionDesktop => include_str!("../assets/icons/session-desktop.svg"),
            Icon::SessionGeneric => include_str!("../assets/icons/session-generic.svg"),
            Icon::X => include_str!("../assets/icons/x.svg"),
        }
    }

    /// Render at `size` px, stroked in `color`.
    /// Cache image identities so every frame reuses GPUI's decoded SVG resource.
    pub fn render(self, color: Rgba, size: f32) -> impl IntoElement {
        self.render_stroke(color, size, 2.)
    }

    /// Explicit Lucide stroke weight, e.g. ShellHeader.tsx's 1.8.
    pub fn render_stroke(self, color: Rgba, size: f32, stroke: f32) -> impl IntoElement {
        thread_local! {
            static CACHE: RefCell<HashMap<(Icon, SharedString, u32), Arc<Image>>> = RefCell::new(HashMap::new());
        }
        let tint = hex_color(color);
        let image = CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let key = (self, tint.clone(), stroke.to_bits());
            if let Some(image) = cache.get(&key) {
                return image.clone();
            }
            if cache.len() >= 512 {
                cache.clear();
            }
            let source = self
                .source()
                .replace("currentColor", &tint)
                .replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke}\""));
            let image = Arc::new(Image::from_bytes(ImageFormat::Svg, source.into_bytes()));
            cache.insert(key, image.clone());
            image
        });
        img(image).w(px(size)).h(px(size))
    }
}

fn hex_color(color: Rgba) -> SharedString {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
    .into()
}
