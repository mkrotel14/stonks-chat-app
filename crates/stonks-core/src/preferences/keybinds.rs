//! Tauri-compatible shortcut names, defaults and conflict detection.
use serde::Deserialize;
use std::collections::BTreeMap;

macro_rules! actions {
    ($($variant:ident, $name:literal, $default:literal;)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Action { $($variant,)+ }
        impl Action {
            pub const ALL: [Self; 12] = [$(Self::$variant,)+];
            pub const fn name(self) -> &'static str { match self { $(Self::$variant => $name,)+ } }
            pub const fn default_binding(self) -> &'static str { match self { $(Self::$variant => $default,)+ } }
        }
    };
}
actions! {
    PreviousChannel, "previousChannel", "Alt+ArrowUp";
    NextChannel, "nextChannel", "Alt+ArrowDown";
    PreviousServer, "previousServer", "Control+Alt+ArrowUp";
    NextServer, "nextServer", "Control+Alt+ArrowDown";
    FocusComposer, "focusComposer", "Control+K";
    JumpToLatest, "jumpToLatest", "Escape";
    MarkServerRead, "markServerRead", "Shift+Escape";
    ToggleSearch, "toggleSearch", "Control+F";
    TogglePins, "togglePins", "Control+Shift+P";
    UploadAttachment, "uploadAttachment", "Control+U";
    ToggleMute, "toggleMute", "Control+Shift+M";
    ToggleDeafen, "toggleDeafen", "Control+Shift+D";
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Keybinds(BTreeMap<String, serde_json::Value>);

impl Keybinds {
    pub fn binding(&self, action: Action) -> &str {
        self.0
            .get(action.name())
            .and_then(|v| v.as_str())
            .unwrap_or(action.default_binding())
    }

    pub fn conflict(&self, action: Action, value: &str) -> Option<Action> {
        let value = normalize(value)?;
        (!value.is_empty())
            .then(|| {
                Action::ALL.into_iter().find(|other| {
                    *other != action
                        && normalize(self.binding(*other)).as_deref() == Some(value.as_str())
                })
            })
            .flatten()
    }

    pub fn matching(&self, value: &str) -> Option<Action> {
        if value.is_empty() {
            return None;
        }
        Action::ALL
            .into_iter()
            .find(|action| normalize(self.binding(*action)).as_deref() == Some(value))
    }
}

pub fn normalize(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some(String::new());
    }
    let mut parts = value.split('+').peekable();
    let mut mods = [false; 4];
    let names = ["Control", "Alt", "Shift", "Meta"];
    while let Some(part) = parts.next() {
        if let Some(i) = names.iter().position(|name| *name == part) {
            mods[i] = true;
        } else if !part.is_empty()
            && parts.peek().is_none()
            && !part.chars().any(char::is_whitespace)
        {
            let key = if part.chars().count() == 1 {
                part.to_uppercase()
            } else {
                part.to_owned()
            };
            return Some(
                names
                    .into_iter()
                    .zip(mods)
                    .filter_map(|(name, on)| on.then_some(name))
                    .chain(std::iter::once(key.as_str()))
                    .collect::<Vec<_>>()
                    .join("+"),
            );
        } else {
            return None;
        }
    }
    None
}

pub fn format(value: &str) -> String {
    value
        .split('+')
        .map(|part| match part {
            "Control" => "Ctrl",
            "ArrowUp" => "↑",
            "ArrowDown" => "↓",
            "ArrowLeft" => "←",
            "ArrowRight" => "→",
            _ => part,
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{load_file, write_change, Change};
    use serde_json::{json, Value};

    #[test]
    fn keybinds_patch_individual_actions_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let original = json!({"theme":"light", "keybinds": {"markServerRead": "F8", "futureAction": {"keep":true}}});
        std::fs::write(&path, original.to_string()).unwrap();
        let action = Action::FocusComposer;
        assert_eq!(
            load_file(&path).unwrap().keybinds.matching("Control+K"),
            Some(action)
        );
        write_change(&path, Change::Keybind(action, "Shift+Control+j".into())).unwrap();
        let saved = load_file(&path).unwrap().keybinds;
        assert_eq!(saved.matching("Control+Shift+J"), Some(action));
        assert_eq!(saved.matching("Control+K"), None);
        assert_eq!(saved.matching("F8"), Some(Action::MarkServerRead));
        assert!(write_change(&path, Change::Keybind(action, "F8".into())).is_err());
        let mark = Action::MarkServerRead;
        let rebound = write_change(&path, Change::Keybind(mark, "F9".into())).unwrap();
        assert_eq!(rebound.keybinds.matching("F8"), None);
        assert_eq!(rebound.keybinds.matching("F9"), Some(mark));
        let saved = write_change(&path, Change::Keybind(action, String::new()))
            .unwrap()
            .keybinds;
        assert_eq!(saved.binding(action), "");
        assert_eq!(saved.matching("Control+Shift+J"), None);
        write_change(
            &path,
            Change::Keybind(action, action.default_binding().into()),
        )
        .unwrap();
        assert_eq!(
            load_file(&path).unwrap().keybinds.matching("Control+K"),
            Some(action)
        );
        let document: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            document["keybinds"]["futureAction"],
            original["keybinds"]["futureAction"]
        );
        assert_eq!(document["keybinds"]["markServerRead"], "F9");
        assert_eq!(document["theme"], "light");
    }

    #[test]
    fn keybinds_normalize_and_detect_default_conflicts() {
        let binds = Keybinds::default();
        assert_eq!(binds.matching("Shift+Escape"), Some(Action::MarkServerRead));
        assert_eq!(
            binds.conflict(Action::ToggleSearch, "Alt+Control+ArrowUp"),
            Some(Action::PreviousServer)
        );
        assert_eq!(normalize("Control+Shift"), None);
        assert_eq!(normalize("Control+A+B"), None);
        assert_eq!(format("Control+Alt+ArrowUp"), "Ctrl + Alt + ↑");
    }
}
