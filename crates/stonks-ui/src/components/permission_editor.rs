//! Source permission groups and high-bit-safe allow/inherit/deny editing.
use crate::{
    components::{Button, ButtonVariant},
    i18n::t,
    Theme,
};
use gpui_kit::*;
use std::rc::Rc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionChoice {
    Allow,
    Inherit,
    Deny,
}
pub fn change_permission(
    allow: u64,
    deny: u64,
    bit: u8,
    choice: PermissionChoice,
    binary: bool,
) -> (u64, u64) {
    let mask = 1u64 << bit;
    let a = allow & !mask;
    let d = deny & !mask;
    match choice {
        PermissionChoice::Allow => (a | mask, d),
        PermissionChoice::Deny if !binary => (a, d | mask),
        _ => (a, d),
    }
}
pub const GROUPS: &[(&str, &[(u8, &str, &str)])] = &[
    (
        "serverSettings.permission.groupAdministration",
        &[
            (
                0,
                "serverSettings.permission.manageChannel",
                "serverSettings.permission.manageChannelHelp",
            ),
            (
                1,
                "serverSettings.permission.manageServer",
                "serverSettings.permission.manageServerHelp",
            ),
            (
                2,
                "serverSettings.permission.managePermissions",
                "serverSettings.permission.managePermissionsHelp",
            ),
            (
                3,
                "serverSettings.permission.manageRole",
                "serverSettings.permission.manageRoleHelp",
            ),
            (
                4,
                "serverSettings.permission.manageCustomisation",
                "serverSettings.permission.manageCustomisationHelp",
            ),
        ],
    ),
    (
        "serverSettings.permission.groupMembers",
        &[
            (
                6,
                "serverSettings.permission.kickMembers",
                "serverSettings.permission.kickMembersHelp",
            ),
            (
                7,
                "serverSettings.permission.banMembers",
                "serverSettings.permission.banMembersHelp",
            ),
            (
                8,
                "serverSettings.permission.timeoutMembers",
                "serverSettings.permission.timeoutMembersHelp",
            ),
            (
                9,
                "serverSettings.permission.assignRoles",
                "serverSettings.permission.assignRolesHelp",
            ),
            (
                10,
                "serverSettings.permission.changeNickname",
                "serverSettings.permission.changeNicknameHelp",
            ),
            (
                11,
                "serverSettings.permission.manageNicknames",
                "serverSettings.permission.manageNicknamesHelp",
            ),
            (
                12,
                "serverSettings.permission.changeAvatar",
                "serverSettings.permission.changeAvatarHelp",
            ),
            (
                13,
                "serverSettings.permission.removeAvatars",
                "serverSettings.permission.removeAvatarsHelp",
            ),
        ],
    ),
    (
        "serverSettings.permission.groupChannels",
        &[
            (
                20,
                "serverSettings.permission.viewChannel",
                "serverSettings.permission.viewChannelHelp",
            ),
            (
                21,
                "serverSettings.permission.readMessageHistory",
                "serverSettings.permission.readMessageHistoryHelp",
            ),
            (
                22,
                "serverSettings.permission.sendMessage",
                "serverSettings.permission.sendMessageHelp",
            ),
            (
                23,
                "serverSettings.permission.manageMessages",
                "serverSettings.permission.manageMessagesHelp",
            ),
            (
                24,
                "serverSettings.permission.manageWebhooks",
                "serverSettings.permission.manageWebhooksHelp",
            ),
            (
                25,
                "serverSettings.permission.inviteOthers",
                "serverSettings.permission.inviteOthersHelp",
            ),
            (
                26,
                "serverSettings.permission.sendEmbeds",
                "serverSettings.permission.sendEmbedsHelp",
            ),
            (
                27,
                "serverSettings.permission.uploadFiles",
                "serverSettings.permission.uploadFilesHelp",
            ),
            (
                28,
                "serverSettings.permission.masquerade",
                "serverSettings.permission.masqueradeHelp",
            ),
            (
                29,
                "serverSettings.permission.react",
                "serverSettings.permission.reactHelp",
            ),
            (
                41,
                "serverSettings.permission.useExternalEmojis",
                "serverSettings.permission.useExternalEmojisHelp",
            ),
            (
                39,
                "serverSettings.permission.bypassSlowmode",
                "serverSettings.permission.bypassSlowmodeHelp",
            ),
        ],
    ),
    (
        "serverSettings.permission.groupVoice",
        &[
            (
                30,
                "serverSettings.permission.connect",
                "serverSettings.permission.connectHelp",
            ),
            (
                31,
                "serverSettings.permission.speak",
                "serverSettings.permission.speakHelp",
            ),
            (
                32,
                "serverSettings.permission.video",
                "serverSettings.permission.videoHelp",
            ),
            (
                33,
                "serverSettings.permission.muteMembers",
                "serverSettings.permission.muteMembersHelp",
            ),
            (
                34,
                "serverSettings.permission.deafenMembers",
                "serverSettings.permission.deafenMembersHelp",
            ),
            (
                35,
                "serverSettings.permission.moveMembers",
                "serverSettings.permission.moveMembersHelp",
            ),
            (
                36,
                "serverSettings.permission.listen",
                "serverSettings.permission.listenHelp",
            ),
        ],
    ),
    (
        "serverSettings.permission.groupMentions",
        &[
            (
                37,
                "serverSettings.permission.mentionEveryone",
                "serverSettings.permission.mentionEveryoneHelp",
            ),
            (
                38,
                "serverSettings.permission.mentionRoles",
                "serverSettings.permission.mentionRolesHelp",
            ),
        ],
    ),
];
#[derive(IntoElement)]
pub struct PermissionEditor {
    theme: Theme,
    id: SharedString,
    allow: u64,
    deny: u64,
    binary: bool,
    compact: Option<bool>,
    disabled: bool,
    on_change: Rc<dyn Fn(u64, u64, &mut Window, &mut App)>,
}
impl PermissionEditor {
    pub fn new(
        id: impl Into<SharedString>,
        theme: Theme,
        allow: u64,
        deny: u64,
        on_change: impl Fn(u64, u64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            theme,
            id: id.into(),
            allow,
            deny,
            binary: false,
            compact: None,
            disabled: false,
            on_change: Rc::new(on_change),
        }
    }
    pub fn binary(mut self, value: bool) -> Self {
        self.binary = value;
        self
    }
    pub fn compact(mut self, value: bool) -> Self {
        self.compact = Some(value);
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}
impl RenderOnce for PermissionEditor {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let compact = self
            .compact
            .unwrap_or(f32::from(window.viewport_size().width) < 800.);
        div()
            .flex()
            .flex_col()
            .gap(px(theme.space5))
            .children(GROUPS.iter().map(|(label, items)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space1))
                    .child(
                        div()
                            .mb(px(theme.space1))
                            .text_size(px(theme.font_size_sm))
                            .text_color(theme.text_muted)
                            .child(t(label).to_uppercase()),
                    )
                    .children(items.iter().map(|(bit, label, hint)| {
                        let selected = if self.allow & (1u64 << bit) != 0 {
                            PermissionChoice::Allow
                        } else if self.binary || self.deny & (1u64 << bit) != 0 {
                            PermissionChoice::Deny
                        } else {
                            PermissionChoice::Inherit
                        };
                        let mut controls = div()
                            .flex()
                            .flex_shrink_0()
                            .rounded(px(theme.radius_sm))
                            .bg(theme.canvas);
                        for choice in [
                            PermissionChoice::Allow,
                            PermissionChoice::Inherit,
                            PermissionChoice::Deny,
                        ] {
                            if self.binary && choice == PermissionChoice::Inherit {
                                continue;
                            }
                            let (variant, key, symbol) = match choice {
                                PermissionChoice::Allow => {
                                    (ButtonVariant::PermissionAllow, "serverSettings.allow", "✓")
                                }
                                PermissionChoice::Inherit => (
                                    ButtonVariant::PermissionInherit,
                                    "serverSettings.inherit",
                                    "·",
                                ),
                                PermissionChoice::Deny => {
                                    (ButtonVariant::PermissionDeny, "serverSettings.deny", "×")
                                }
                            };
                            let callback = self.on_change.clone();
                            let (a, d) =
                                change_permission(self.allow, self.deny, *bit, choice, self.binary);
                            controls = controls.child(
                                Button::new(
                                    ElementId::Name(format!("{}-{bit}-{symbol}", self.id).into()),
                                    theme,
                                )
                                .variant(variant)
                                .selected(selected == choice)
                                .tooltip(t(key))
                                .label(symbol)
                                .disabled(self.disabled)
                                .on_click(move |_, window, cx| callback(a, d, window, cx)),
                            );
                        }
                        let mut row = div()
                            .flex()
                            .min_h(px(52.))
                            .items_center()
                            .justify_between()
                            .gap(px(theme.space4))
                            .py(px(theme.space2))
                            .px(px(10.))
                            .rounded(px(theme.radius_sm));
                        if compact {
                            row = row.flex_col().items_start();
                        }
                        row.child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .flex_1()
                                .gap(px(theme.space1))
                                .child(
                                    div()
                                        .text_size(px(theme.font_size_md))
                                        .text_color(theme.text)
                                        .child(t(label)),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme.font_size_sm))
                                        .text_color(theme.text_muted)
                                        .child(t(hint)),
                                ),
                        )
                        .child(controls)
                    }))
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::{change_permission, PermissionChoice::*};
    #[test]
    fn toggling_high_bits_preserves_other_permissions_and_clears_opposing_mask() {
        let (a, d) = change_permission(1 << 41, 1 << 38, 38, Allow, false);
        assert_eq!(a, (1 << 41) | (1 << 38));
        assert_eq!(d, 0);
        let (a, d) = change_permission(a, d, 41, Deny, false);
        assert_eq!(a, 1 << 38);
        assert_eq!(d, 1 << 41);
        let (a, d) = change_permission(a, d, 41, Inherit, false);
        assert_eq!((a, d), (1 << 38, 0));
    }
    #[test]
    fn binary_denial_removes_allow_without_creating_override() {
        assert_eq!(change_permission(1 << 32, 0, 32, Deny, true), (0, 0));
    }
}
