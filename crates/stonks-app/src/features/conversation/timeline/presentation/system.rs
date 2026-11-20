//! System-event rows, matching MessageRow/systemDescription in the Tauri UI.
//! Keep event semantics in presentation: normalized message content remains
//! the original server payload, and call_started stays out of the timeline.

use gpui_kit::{div, px, AnyElement, FontWeight, IntoElement, ParentElement, Styled};
use stonks_core::messages::models::TimelineMessage;
use stonks_ui::{i18n::t, icons::Icon, Theme};

/// The Tauri timeline excludes call_started records from reading history.
/// Other unknown system types remain visible with their text/fallback label.
pub fn is_visible(message: &TimelineMessage) -> bool {
    !(message.system && message.system_type.as_deref() == Some("call_started"))
}

fn event_key(kind: Option<&str>) -> Option<&'static str> {
    match kind? {
        "user_joined" => Some("messages.system.userJoined"),
        "user_left" => Some("messages.system.userLeft"),
        "user_kicked" => Some("messages.system.userKicked"),
        "user_banned" => Some("messages.system.userBanned"),
        "channel_renamed" => Some("messages.system.channelRenamed"),
        "channel_description_changed" => Some("messages.system.channelDescriptionChanged"),
        "channel_icon_changed" => Some("messages.system.channelIconChanged"),
        "message_pinned" => Some("messages.system.messagePinned"),
        "message_unpinned" => Some("messages.system.messageUnpinned"),
        _ => None,
    }
}

/// Event text follows the author pill, so these translations contain no
/// user-name interpolation and preserve the source's compact row structure.
pub fn description(message: &TimelineMessage) -> String {
    event_key(message.system_type.as_deref())
        .map(t)
        .or_else(|| {
            message
                .content
                .clone()
                .filter(|content| !content.trim().is_empty())
        })
        .unwrap_or_else(|| t("messages.system.updatedConversation"))
}

fn event_icon(kind: Option<&str>) -> Icon {
    match kind {
        Some("user_joined") => Icon::UserPlus,
        Some("user_left" | "user_kicked") => Icon::UserMinus,
        Some("user_banned") => Icon::Ban,
        Some("message_pinned" | "message_unpinned") => Icon::Pin,
        Some("channel_renamed" | "channel_description_changed" | "channel_icon_changed") => {
            Icon::Settings
        }
        _ => Icon::MessageCircle,
    }
}

/// Returns only system rows. Callers should render normal messages through
/// their regular path and omit invisible call_started records entirely.
pub fn render(message: &TimelineMessage, theme: &Theme) -> Option<AnyElement> {
    if !message.system || !is_visible(message) {
        return None;
    }
    Some(
        div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .min_h(px(30.))
            .px(px(theme.space4))
            .py(px(theme.space1))
            .gap(px(theme.space3))
            .child(
                div()
                    .w(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(event_icon(message.system_type.as_deref()).render(theme.link, 18.)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .gap(px(theme.space1))
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .max_w_full()
                            .px(px(7.))
                            .py(px(2.))
                            .rounded(px(theme.radius_sm))
                            .bg(theme.surface_selected)
                            .text_color(theme.link)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(theme.font_size_xs))
                            .child(message.author_name.clone()),
                    )
                    .child(div().min_w_0().flex_1().child(description(message))),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use stonks_core::messages::normalize::normalize_message;

    fn message(kind: Option<&str>, content: Option<&str>) -> TimelineMessage {
        let mut value = serde_json::json!({
            "_id":"01M0P54M56DA883E6ZYDK00BKC", "channel":"channel", "author":"user",
            "content":content
        });
        if let Some(kind) = kind {
            value["system"] = serde_json::json!({"type":kind});
        }
        normalize_message(&value, &HashMap::new(), &HashMap::new(), None).unwrap()
    }

    #[test]
    fn known_events_use_specific_localized_descriptions() {
        for (kind, key) in [
            ("user_joined", "userJoined"),
            ("user_left", "userLeft"),
            ("user_kicked", "userKicked"),
            ("user_banned", "userBanned"),
            ("channel_renamed", "channelRenamed"),
            ("channel_description_changed", "channelDescriptionChanged"),
            ("channel_icon_changed", "channelIconChanged"),
            ("message_pinned", "messagePinned"),
            ("message_unpinned", "messageUnpinned"),
        ] {
            assert_eq!(
                event_key(Some(kind)),
                Some(format!("messages.system.{key}").as_str())
            );
            assert!(is_visible(&message(Some(kind), None)));
        }
    }

    #[test]
    fn call_records_are_hidden_but_normal_and_unknown_messages_stay_visible() {
        let call = message(Some("call_started"), None);
        assert!(!is_visible(&call));
        assert!(render(&call, &Theme::dark()).is_none());
        let normal = message(None, Some("normal content"));
        assert!(is_visible(&normal));
        assert!(render(&normal, &Theme::dark()).is_none());
        let unknown = message(Some("new_server_event"), Some("server-authored event text"));
        assert!(is_visible(&unknown));
        assert_eq!(description(&unknown), "server-authored event text");
        assert!(render(&unknown, &Theme::dark()).is_some());
    }

    #[test]
    fn unknown_event_without_content_uses_a_translated_fallback() {
        let unknown = message(Some("new_server_event"), Some(" \n "));
        assert_eq!(
            description(&unknown),
            t("messages.system.updatedConversation")
        );
    }
}
