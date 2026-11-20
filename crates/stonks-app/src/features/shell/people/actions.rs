//! Member context menu and profile placement.

use super::super::{interactions::MenuAction, *};
use stonks_core::member_actions::Action;
use stonks_ui::components::MenuEntry;

impl ShellView {
    pub(in crate::features::shell) fn open_member_menu(
        &mut self,
        server: String,
        user: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_actions.clear();
        let actions = self
            .member_actions
            .read(cx)
            .context_actions(&server, &user, cx);
        let own = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.current_user.id == user);
        let mut entries = vec![];
        let mut previous = None;
        for action in actions {
            let group = match action {
                Action::Profile | Action::Message => 0,
                Action::Identity | Action::Roles => 1,
                Action::Kick | Action::Ban => 2,
                _ => 3,
            };
            if previous.is_some_and(|previous| previous != group) {
                entries.push(MenuEntry::Separator);
            }
            previous = Some(group);
            let label = if own && action == Action::Identity {
                "Edit your identity"
            } else {
                action.label()
            };
            let mut item = self.item(
                MenuAction::Member {
                    server: server.clone(),
                    user: user.clone(),
                    action,
                },
                label,
            );
            if action.dangerous() || action == Action::Report {
                item = item.danger();
            }
            entries.push(item);
        }
        self.context_menu
            .update(cx, |menu, cx| menu.open(position, entries, window, cx));
        cx.notify();
    }
    pub(in crate::features::shell) fn open_member_profile(
        &mut self,
        server: String,
        user: String,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_focus.is_none() {
            self.overlay_focus = window.focused(cx);
        }
        let bounds = self.member_bounds.get(&user).copied();
        let viewport = window.viewport_size();
        let x = (bounds.map(|bounds| bounds.right()).unwrap_or(pointer.x) - px(360.))
            .min(viewport.width - px(372.))
            .max(px(12.));
        let y = (bounds.map(|bounds| bounds.bottom()).unwrap_or(pointer.y) + px(6.))
            .min(viewport.height - px(430.))
            .max(px(12.));
        self.member_actions.update(cx, |actions, cx| {
            actions.open_profile(server, user, Some(point(x, y)), window, cx)
        });
        cx.notify();
    }
}
