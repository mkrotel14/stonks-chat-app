//! Message action policy and presentation. The host owns the target message,
//! command handlers, and visibility (row hover/focus or an open popup).
use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    canvas, div, point, px, App, Bounds, BoxShadow, ElementId, Entity, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Styled, Window,
};
use stonks_ui::{
    components::{Button, ButtonVariant, MenuEntry, UiContextMenu},
    icons::Icon,
    Theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum MessageAction {
    Reply,
    MarkUnread,
    CopyText,
    React,
    Pin,
    Unpin,
    Edit,
    Delete,
    Report,
    CopyLink,
    CopyId,
}

use MessageAction::*;

const MENU_GROUPS: &[&[MessageAction]] = &[
    &[Reply, MarkUnread, CopyText],
    &[React, Pin, Unpin, Edit, Delete, Report],
    &[CopyLink, CopyId],
];

impl MessageAction {
    fn id(self) -> &'static str {
        match self {
            Reply => "reply",
            MarkUnread => "mark-unread",
            CopyText => "copy-text",
            React => "react",
            Pin => "pin",
            Unpin => "unpin",
            Edit => "edit",
            Delete => "delete",
            Report => "report",
            CopyLink => "copy-link",
            CopyId => "copy-id",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Reply => "Reply",
            MarkUnread => "Mark as unread",
            CopyText => "Copy text",
            React => "React",
            Pin => "Pin message",
            Unpin => "Unpin message",
            Edit => "Edit message",
            Delete => "Delete message",
            Report => "Report message",
            CopyLink => "Copy message link",
            CopyId => "Copy message ID",
        }
    }

    fn danger(self) -> bool {
        matches!(self, Delete | Report)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(in super::super) struct ActionEligibility {
    /// A normal server-backed message, not a system, pending or failed row.
    pub actionable: bool,
    pub own: bool,
    pub can_send: bool,
    pub can_react: bool,
    pub can_manage: bool,
    pub pinned: bool,
    pub has_text: bool,
    pub has_link: bool,
    pub developer_mode: bool,
}

impl ActionEligibility {
    fn allows(self, action: MessageAction) -> bool {
        self.actionable
            && match action {
                Reply => self.can_send,
                MarkUnread => true,
                CopyText => self.has_text,
                React => self.can_react,
                Pin => self.can_manage && !self.pinned,
                Unpin => self.can_manage && self.pinned,
                Edit => self.own && self.can_send,
                Delete => self.own || self.can_manage,
                Report => !self.own,
                CopyLink => self.has_link,
                CopyId => self.developer_mode,
            }
    }
}

/// Snapshot of eligible actions with real handlers. Rebuild when the message,
/// permissions or handler availability changes, and recheck before dispatch.
#[derive(Clone, Debug)]
pub(in super::super) struct MessageActions {
    own: bool,
    available: Vec<MessageAction>,
}

impl MessageActions {
    pub fn new(eligibility: ActionEligibility, wired: impl Fn(MessageAction) -> bool) -> Self {
        Self {
            own: eligibility.own,
            available: MENU_GROUPS
                .iter()
                .flat_map(|group| group.iter().copied())
                .filter(|action| eligibility.allows(*action) && wired(*action))
                .collect(),
        }
    }

    /// Revalidate a toolbar action against the current action snapshot.
    pub fn contains(&self, action: MessageAction) -> bool {
        self.available.contains(&action)
    }

    /// Resolve a shared menu Select event against the current action snapshot.
    pub fn resolve(&self, id: &str) -> Option<MessageAction> {
        self.available.iter().copied().find(|a| a.id() == id)
    }

    fn entries(&self) -> Vec<MenuEntry> {
        let mut entries = Vec::new();
        for group in MENU_GROUPS {
            let mut rows = group
                .iter()
                .filter(|a| self.available.contains(a))
                .peekable();
            if rows.peek().is_some() && !entries.is_empty() {
                entries.push(MenuEntry::Separator);
            }
            entries.extend(rows.map(|action| {
                let row = MenuEntry::action(action.id(), action.label());
                if action.danger() {
                    row.danger()
                } else {
                    row
                }
            }));
        }
        entries
    }

    /// Returns false for an empty menu; only consume the row's right click on true.
    pub fn open_menu(
        &self,
        menu: &Entity<UiContextMenu>,
        anchor: MenuAnchor,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let entries = self.entries();
        if entries.is_empty() {
            return false;
        }
        menu.update(cx, |menu, cx| match anchor {
            MenuAnchor::Pointer(point) => menu.open(point, entries, window, cx),
            MenuAnchor::Toolbar(bounds) => menu.open_below_right(bounds, entries, window, cx),
        });
        true
    }

    /// Mount inside the relative message row. `top` is the source row's
    /// mode-dependent offset (normally -15 px). Popup flags are for this row.
    pub fn toolbar(
        &self,
        message_id: &str,
        theme: Theme,
        top: Pixels,
        menu_open: bool,
        reaction_open: bool,
        on_event: impl Fn(ToolbarEvent, &mut Window, &mut App) + 'static,
    ) -> Option<impl IntoElement> {
        if self.available.is_empty() {
            return None;
        }
        let on_event = Rc::new(on_event);
        let mut toolbar = div()
            .id(ElementId::Name(
                format!("message-actions-{message_id}").into(),
            ))
            .absolute()
            .top(top)
            .right(px(theme.space2))
            .flex()
            // Occlusion also blocks the host row’s hover hitbox, which would
            // unmount this toolbar as soon as the pointer reached it.
            .on_mouse_down(gpui_kit::MouseButton::Right, |_, _, cx| {
                cx.stop_propagation()
            })
            .p(px(theme.space1))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_sm))
            .bg(theme.surface_raised)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }]);
        for (action, icon, label, expanded) in [
            (Some(Reply), Icon::Reply, "Reply", None),
            (
                Some(React),
                Icon::Smile,
                "Add reaction",
                Some(reaction_open),
            ),
            (Some(Edit), Icon::Pencil, "Edit", None),
            (
                Some(Delete),
                Icon::Trash,
                if self.own {
                    "Delete"
                } else {
                    "Delete as moderator"
                },
                None,
            ),
            (None, Icon::Ellipsis, "More actions", Some(menu_open)),
        ] {
            if action.is_some_and(|action| !self.available.contains(&action)) {
                continue;
            }
            let danger = action.is_some_and(MessageAction::danger);
            let bounds = Rc::new(Cell::new(None));
            let painted_bounds = bounds.clone();
            let on_event = on_event.clone();
            let mut button = Button::new(action.map_or("more", MessageAction::id), theme)
                .variant(if danger {
                    ButtonVariant::DangerQuiet
                } else {
                    ButtonVariant::Quiet
                })
                .icon_only(true)
                .tooltip(label)
                .leading(icon.render_stroke(
                    if danger { theme.danger } else { theme.text },
                    20.,
                    1.8,
                ))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    if let Some(bounds) = bounds.get() {
                        let event = match action {
                            Some(action) => ToolbarEvent::Action(action, bounds),
                            None => ToolbarEvent::More(bounds),
                        };
                        on_event(event, window, cx);
                    }
                });
            if let Some(expanded) = expanded {
                button = button.expanded(expanded);
            }
            toolbar = toolbar.child(
                div().relative().child(button).child(
                    canvas(
                        move |bounds, _, _| painted_bounds.set(Some(bounds)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                ),
            );
        }
        Some(toolbar)
    }
}

pub(in super::super) enum ToolbarEvent {
    /// Button bounds are available for anchored handlers such as React.
    Action(MessageAction, Bounds<Pixels>),
    More(Bounds<Pixels>),
}

#[derive(Clone, Copy)]
pub(in super::super) enum MenuAnchor {
    Pointer(Point<Pixels>),
    Toolbar(Bounds<Pixels>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_both_policy_and_handler() {
        let policy = ActionEligibility {
            actionable: true,
            can_send: true,
            ..Default::default()
        };
        let actions = MessageActions::new(policy, |a| matches!(a, Reply | Edit));
        assert_eq!(actions.resolve("reply"), Some(Reply));
        assert_eq!(actions.resolve("edit"), None);
        assert_eq!(actions.resolve("mark-unread"), None);
        assert_eq!(actions.entries().len(), 1);
        assert!(MessageActions::new(ActionEligibility::default(), |_| true)
            .entries()
            .is_empty());
    }

    #[test]
    fn copy_id_is_available_only_in_developer_mode() {
        let mut policy = ActionEligibility {
            actionable: true,
            ..Default::default()
        };
        assert_eq!(
            MessageActions::new(policy, |_| true).resolve("copy-id"),
            None
        );
        policy.developer_mode = true;
        assert_eq!(
            MessageActions::new(policy, |_| true).resolve("copy-id"),
            Some(CopyId)
        );
        policy.actionable = false;
        assert_eq!(
            MessageActions::new(policy, |_| true).resolve("copy-id"),
            None
        );
    }

    #[test]
    fn skips_empty_groups_and_marks_moderation_dangerous() {
        let policy = ActionEligibility {
            actionable: true,
            can_manage: true,
            pinned: true,
            has_link: true,
            ..Default::default()
        };
        let actions = MessageActions::new(policy, |a| !matches!(a, MarkUnread | Report));
        assert_eq!(actions.resolve("pin"), None);
        assert_eq!(actions.resolve("unpin"), Some(Unpin));
        assert!(matches!(
            actions.entries().as_slice(),
            [
                MenuEntry::Item { danger: false, .. },
                MenuEntry::Item { danger: true, .. },
                MenuEntry::Separator,
                MenuEntry::Item { danger: false, .. },
            ]
        ));
    }
}
