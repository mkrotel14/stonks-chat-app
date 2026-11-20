use super::*;
use std::collections::{HashMap, HashSet};
use stonks_core::connection::ConnectionManager;

#[derive(Clone)]
pub enum Dialog {
    CreateRole,
    DeleteRole(String, String),
    Emoji(ServerEmoji),
    CreateInvite,
    RevokeInvite(ServerInvite),
}
pub struct Extra {
    pub invites: super::invites::InviteState,
    pub emoji: super::emoji::EmojiState,
    pub roles: super::roles::RoleState,
    pub pending_snapshot: Option<(ClientServer, Vec<ClientChannel>)>,
    pub last_notice: Option<String>,
    pub last_error: Option<String>,
    pub notice_timer: Option<Task<()>>,
    pub error_timer: Option<Task<()>>,
    pub frame_visible: bool,
    pub authorization: Option<Arc<ConnectionManager>>,
    pub users: HashMap<String, String>,
    pub dialog: Option<Dialog>,
    pub dialog_focus: FocusHandle,
    pub heading_focus: FocusHandle,
    pub navigation_focus: [FocusHandle; 4],
    pub previous_focus: Option<FocusHandle>,
    pub restore_focus: bool,
    pub picker: Option<Task<()>>,
    pub picker_generation: u64,
    pub _subscriptions: Vec<Subscription>,
}
impl Extra {
    pub fn new(window: &mut Window, cx: &mut Context<ServerSettings>) -> Self {
        let role_name = cx
            .new(|cx| InputState::new(window, cx).validate(|value, _| value.chars().count() <= 32));
        let emoji_name = cx
            .new(|cx| InputState::new(window, cx).validate(|value, _| value.chars().count() <= 32));
        let rename_emoji = cx
            .new(|cx| InputState::new(window, cx).validate(|value, _| value.chars().count() <= 32));
        let invite_link = cx.new(|cx| InputState::new(window, cx));
        let invite_channel = select(vec![], window, cx);
        let invite_duration = select(
            [
                (0, "invite.never"),
                (3600, "invite.hour"),
                (86400, "invite.day"),
                (604800, "invite.week"),
            ]
            .into_iter()
            .map(|(value, key)| UiSelectOption::new(value.to_string(), t(key)))
            .collect(),
            window,
            cx,
        );
        let invite_uses = select(
            [0, 1, 5, 10, 25, 50, 100]
                .into_iter()
                .map(|value| {
                    UiSelectOption::new(
                        value.to_string(),
                        if value == 0 {
                            t("invite.unlimited")
                        } else {
                            value.to_string()
                        },
                    )
                })
                .collect(),
            window,
            cx,
        );
        let subscriptions = vec![
            cx.observe(&role_name, |_, _, cx| cx.notify()),
            cx.subscribe_in(&role_name, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && matches!(this.extra.dialog, Some(Dialog::CreateRole))
                    && !this
                        .extra
                        .roles
                        .role_name
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                {
                    this.mutate(
                        super::actions::Mutation::CreateRole(
                            this.extra.roles.role_name.read(cx).value().to_string(),
                        ),
                        window,
                        cx,
                    );
                }
            }),
            cx.observe(&emoji_name, |_, _, cx| cx.notify()),
            cx.observe(&rename_emoji, |_, _, cx| cx.notify()),
            cx.observe(&invite_channel, |_, _, cx| cx.notify()),
            cx.observe(&invite_duration, |_, _, cx| cx.notify()),
            cx.observe(&invite_uses, |_, _, cx| cx.notify()),
        ];
        Self {
            invites: super::invites::InviteState {
                invite_eligible: vec![],
                invite_link,
                invite_channel,
                invite_duration,
                invite_uses,
                created_invite: None,
            },
            emoji: super::emoji::EmojiState {
                emoji_name,
                rename_emoji,
                emoji_file: None,
                emoji_preview: None,
                preview_task: None,
            },
            roles: super::roles::RoleState {
                uploading_role: None,
                role_drafts: HashMap::new(),
                expanded: HashSet::new(),
                role_name,
            },
            pending_snapshot: None,
            last_notice: None,
            last_error: None,
            notice_timer: None,
            error_timer: None,
            frame_visible: true,
            authorization: None,
            users: HashMap::new(),
            dialog: None,
            dialog_focus: cx.focus_handle(),
            heading_focus: cx.focus_handle(),
            navigation_focus: std::array::from_fn(|_| cx.focus_handle()),
            previous_focus: None,
            restore_focus: false,
            picker: None,
            picker_generation: 0,
            _subscriptions: subscriptions,
        }
    }
}
pub fn select(
    options: Vec<UiSelectOption>,
    window: &mut Window,
    cx: &mut Context<ServerSettings>,
) -> Entity<UiSelectState> {
    let index = (!options.is_empty()).then_some(IndexPath::new(0));
    cx.new(|cx| UiSelectState::new(SearchableVec::new(options), index, window, cx))
}
