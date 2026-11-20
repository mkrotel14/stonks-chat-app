#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Profile,
    Message,
    Identity,
    Roles,
    Kick,
    Ban,
    Block,
    Report,
    Add,
    Accept,
    Decline,
    Cancel,
    Remove,
    Unblock,
    Transfer,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Profile => "Profile",
            Self::Message => "Send message",
            Self::Identity => "Edit identity",
            Self::Roles => "Edit roles",
            Self::Kick => "Kick member",
            Self::Ban => "Ban member",
            Self::Block => "Block user",
            Self::Report => "Report user",
            Self::Add => "Add friend",
            Self::Accept => "Accept request",
            Self::Decline => "Decline request",
            Self::Cancel => "Cancel request",
            Self::Remove => "Remove friend",
            Self::Unblock => "Unblock user",
            Self::Transfer => "Transfer ownership",
        }
    }
    pub fn dangerous(self) -> bool {
        matches!(
            self,
            Self::Kick | Self::Ban | Self::Block | Self::Remove | Self::Decline | Self::Transfer
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    Friend,
    Incoming,
    Outgoing,
    Blocked,
    Pending,
    None,
    Unknown,
}
pub fn relationship_kind(value: Option<&str>) -> RelationshipKind {
    let value = value
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '_' | '-'))
        .collect::<String>();
    match value.as_str() {
        "friend" | "friends" => RelationshipKind::Friend,
        "incoming" | "incomingrequest" | "pendingincoming" | "requestreceived" => {
            RelationshipKind::Incoming
        }
        "outgoing" | "outgoingrequest" | "pendingoutgoing" | "requestsent" => {
            RelationshipKind::Outgoing
        }
        "blocked" | "block" => RelationshipKind::Blocked,
        "pending" => RelationshipKind::Pending,
        "" | "none" | "norelationship" => RelationshipKind::None,
        _ => RelationshipKind::Unknown,
    }
}
pub fn relationship_actions(value: Option<&str>, bot: bool) -> Vec<Action> {
    if bot {
        return vec![Action::Message];
    }
    match relationship_kind(value) {
        RelationshipKind::Friend => vec![Action::Message, Action::Remove, Action::Block],
        RelationshipKind::Incoming => vec![Action::Accept, Action::Decline, Action::Block],
        RelationshipKind::Outgoing => vec![Action::Cancel, Action::Block],
        RelationshipKind::Blocked => vec![Action::Unblock],
        RelationshipKind::None => vec![Action::Add, Action::Block],
        _ => vec![],
    }
}

/// Exact visibility rules from ShellContextMenus' member branch.
pub fn context_actions(self_user: bool, permissions: u64) -> Vec<Action> {
    let can = |mask| permissions & mask == mask;
    let mut actions = vec![Action::Profile];
    if !self_user {
        actions.push(Action::Message);
    }
    if (self_user && (can(super::CHANGE_NICKNAME) || can(super::CHANGE_AVATAR)))
        || (!self_user && can(super::MANAGE_NICKNAMES))
    {
        actions.push(Action::Identity);
    }
    if can(super::ASSIGN_ROLES) {
        actions.push(Action::Roles);
    }
    if !self_user {
        if can(super::KICK_MEMBERS) {
            actions.push(Action::Kick);
        }
        if can(super::BAN_MEMBERS) {
            actions.push(Action::Ban);
        }
        actions.extend([Action::Block, Action::Report]);
    }
    actions
}
