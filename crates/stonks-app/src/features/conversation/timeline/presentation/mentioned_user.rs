//! The compact global-user aside opened by formatted @mentions. This is the
//! timeline's small profile, separate from the member-management dialog.
use gpui_kit::*;
use std::sync::Arc;
use stonks_core::connection::{AppSnapshot, ClientUser};
use stonks_ui::{
    components::{avatar, BotBadge, UiCloseButton},
    i18n::t,
    Theme,
};

pub(crate) const SOURCE_SCOPE: &str = "mention-profile";
pub(crate) const AVATAR_EDGE: u32 = 108;

#[derive(Debug, PartialEq)]
pub(crate) struct Profile {
    pub name: String,
    pub username: String,
    pub pronouns: Option<String>,
    pub status: Option<String>,
    pub avatar_url: Option<String>,
    pub bot: bool,
}
pub(crate) fn user<'a>(snapshot: &'a AppSnapshot, id: &str) -> Option<&'a ClientUser> {
    snapshot
        .users
        .iter()
        .chain(std::iter::once(&snapshot.current_user))
        .find(|user| user.id == id)
}
impl Profile {
    pub fn new(user: &ClientUser, autumn: Option<&str>) -> Self {
        let present = |value: Option<String>| value.filter(|value| !value.is_empty());
        Self {
            name: user
                .display_name
                .clone()
                .unwrap_or_else(|| user.username.clone()),
            username: format!(
                "@{}{}",
                user.username,
                user.discriminator
                    .as_ref()
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("#{value}"))
                    .unwrap_or_default()
            ),
            pronouns: present(user.pronouns.clone()),
            status: present(user.status.as_ref().and_then(|status| status.text.clone())),
            avatar_url: user
                .avatar
                .as_ref()
                .and_then(|file| stonks_core::media::avatar_url(autumn, &file.tag, &file.id)),
            bot: user
                .bot
                .as_ref()
                .is_some_and(|value| !value.is_null() && value != &serde_json::Value::Bool(false)),
        }
    }
}

pub(crate) fn render(
    profile: Profile,
    image: Option<Arc<RenderImage>>,
    theme: Theme,
    focus: &FocusHandle,
    close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut copy = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .child(
                    div()
                        .min_w_0()
                        .text_ellipsis()
                        .font_weight(FontWeight::BOLD)
                        .child(profile.name.clone()),
                )
                .children(profile.bot.then(|| BotBadge::new(theme))),
        )
        .child(
            div()
                .text_size(px(theme.font_size_xs))
                .text_color(theme.text_muted)
                .child(profile.username),
        );
    if let Some(pronouns) = profile.pronouns {
        copy = copy.child(
            div()
                .text_size(px(theme.font_size_md * (5. / 6.)))
                .text_color(theme.text_muted)
                .child(pronouns),
        );
    }
    if let Some(status) = profile.status {
        copy = copy.child(div().my(px(theme.font_size_md)).child(status));
    }
    div()
        .id("timeline-mentioned-user")
        .absolute()
        .left(px(26.))
        .right(px(26.))
        .bottom(px(76.))
        .max_w(px(330.))
        .track_focus(focus)
        .tab_index(-1)
        .aria_label(format!("{} {}", profile.name, t("settings.pages.profile")))
        .flex()
        .gap(px(theme.space3))
        .pt(px(48.))
        .px(px(16.))
        .pb(px(16.))
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_md))
        .bg(theme.surface_raised)
        .text_color(theme.text)
        .text_size(px(theme.font_size_md))
        .line_height(relative(1.5))
        .shadow(vec![BoxShadow {
            color: theme.shadow_md.color.into(),
            offset: point(px(0.), px(theme.shadow_md.dy)),
            blur_radius: px(theme.shadow_md.blur),
            spread_radius: px(0.),
            inset: false,
        }])
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .absolute()
                .right(px(8.))
                .top(px(8.))
                .child(UiCloseButton::new("timeline-mentioned-user-close", theme).on_click(close)),
        )
        .child(avatar(&theme, &profile.name, 54., image))
        .child(copy)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::Profile;
    #[test]
    fn compact_profile_uses_global_user_fields_and_animated_original_avatar() {
        let user = serde_json::from_value(serde_json::json!({"_id":"user","username":"reader","discriminator":"0042","display_name":"Global Reader","pronouns":"they/them","status":{"text":"Reading"},"avatar":{"_id":"avatar","tag":"avatars"},"bot":{"owner":"owner"}})).unwrap();
        let profile = Profile::new(&user, Some("https://cdn.example.test/files"));
        assert_eq!(profile.name, "Global Reader");
        assert_eq!(profile.username, "@reader#0042");
        assert_eq!(profile.pronouns.as_deref(), Some("they/them"));
        assert_eq!(profile.status.as_deref(), Some("Reading"));
        assert_eq!(
            profile.avatar_url.as_deref(),
            Some("https://cdn.example.test/files/avatars/avatar/original")
        );
        assert!(profile.bot);
    }
    #[test]
    fn optional_empty_fields_and_nonbot_do_not_create_blank_profile_rows() {
        let user = serde_json::from_value(serde_json::json!({"_id":"user","username":"reader","discriminator":"","pronouns":"","status":{"text":""},"bot":false})).unwrap();
        let profile = Profile::new(&user, None);
        assert_eq!(profile.name, "reader");
        assert_eq!(profile.username, "@reader");
        assert_eq!(profile.pronouns, None);
        assert_eq!(profile.status, None);
        assert!(!profile.bot);
        assert_eq!(profile.avatar_url, None);
    }
}
