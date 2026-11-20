use super::MenuEntry;
#[derive(Debug, PartialEq)]
pub(super) enum KeyIntent {
    Move(i32, bool),
    OpenSubmenu,
    Activate,
    CloseSubmenu,
    Close,
    Ignore,
}
pub(super) fn key_intent(key: &str, entry: Option<&MenuEntry>, has_parent: bool) -> KeyIntent {
    match key {
        "down" => KeyIntent::Move(1, false),
        "up" => KeyIntent::Move(-1, false),
        "home" => KeyIntent::Move(-1, true),
        "end" => KeyIntent::Move(1, true),
        "right"
            if matches!(
                entry,
                Some(MenuEntry::Submenu {
                    disabled: false,
                    ..
                })
            ) =>
        {
            KeyIntent::OpenSubmenu
        }
        "enter" | "space" if entry.is_some_and(MenuEntry::enabled) => KeyIntent::Activate,
        "left" | "escape" if has_parent => KeyIntent::CloseSubmenu,
        "escape" | "tab" => KeyIntent::Close,
        _ => KeyIntent::Ignore,
    }
}
pub(super) fn navigation_panel(selected: &[usize], open_path: &[usize]) -> Vec<usize> {
    selected
        .split_last()
        .map(|(_, parent)| parent.to_vec())
        .unwrap_or_else(|| open_path.to_vec())
}
pub(super) fn next_enabled(
    entries: &[MenuEntry],
    current: Option<usize>,
    direction: i32,
    end: bool,
) -> Option<usize> {
    let enabled = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.enabled().then_some(index))
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        return None;
    }
    let current =
        current.and_then(|index| enabled.iter().position(|candidate| *candidate == index));
    let next = if end {
        if direction > 0 {
            enabled.len() - 1
        } else {
            0
        }
    } else {
        current
            .map(|current| (current as i32 + direction).rem_euclid(enabled.len() as i32) as usize)
            .unwrap_or(if direction > 0 { 0 } else { enabled.len() - 1 })
    };
    Some(enabled[next])
}
#[cfg(test)]
mod tests {
    use super::{key_intent, navigation_panel, next_enabled, KeyIntent};
    use crate::components::context_menu::MenuEntry;
    #[test]
    fn right_never_activates_ordinary_or_disabled_actions() {
        let delete = MenuEntry::action("delete", "Delete").danger();
        assert_eq!(key_intent("right", Some(&delete), false), KeyIntent::Ignore);
        assert_eq!(
            key_intent("enter", Some(&delete), false),
            KeyIntent::Activate
        );
        let submenu = MenuEntry::Submenu {
            label: "More".into(),
            disabled: false,
            items: vec![delete],
        };
        assert_eq!(
            key_intent("right", Some(&submenu), false),
            KeyIntent::OpenSubmenu
        );
        assert_eq!(
            key_intent("right", Some(&submenu.disabled(true)), false),
            KeyIntent::Ignore
        );
    }
    #[test]
    fn navigation_skips_disabled_and_separators_wraps_and_targets_extremes() {
        let entries = vec![
            MenuEntry::Separator,
            MenuEntry::action("a", "A"),
            MenuEntry::action("b", "B").disabled(true),
            MenuEntry::action("c", "C"),
        ];
        assert_eq!(next_enabled(&entries, None, 1, false), Some(1));
        assert_eq!(next_enabled(&entries, Some(1), 1, false), Some(3));
        assert_eq!(next_enabled(&entries, Some(3), 1, false), Some(1));
        assert_eq!(next_enabled(&entries, Some(1), -1, false), Some(3));
        assert_eq!(next_enabled(&entries, None, -1, true), Some(1));
        assert_eq!(next_enabled(&entries, None, 1, true), Some(3));
        assert_eq!(next_enabled(&[], None, 1, false), None);
    }
    #[test]
    fn escape_closes_one_submenu_tab_dismisses_the_whole_menu() {
        assert_eq!(key_intent("escape", None, true), KeyIntent::CloseSubmenu);
        assert_eq!(key_intent("escape", None, false), KeyIntent::Close);
        assert_eq!(key_intent("tab", None, true), KeyIntent::Close);
        assert_eq!(key_intent("left", None, false), KeyIntent::Ignore);
    }
    #[test]
    fn open_submenu_does_not_redirect_navigation_while_trigger_has_focus() {
        assert_eq!(navigation_panel(&[2], &[2]), Vec::<usize>::new());
        assert_eq!(navigation_panel(&[2, 1], &[2]), vec![2]);
        assert_eq!(navigation_panel(&[], &[2]), vec![2]);
    }
}
