//! Live Tauri-compatible shortcut dispatch for the authenticated window.
use super::*;
use crate::preferences::NativePreferences;
use stonks_core::preferences::keybinds::Action;

pub(crate) fn chord(stroke: &Keystroke) -> Option<String> {
    let m = stroke.modifiers;
    if m.function {
        return None;
    }
    let key = match stroke.key.as_str() {
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "escape" => "Escape",
        "space" => "Space",
        "enter" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        key if key.chars().count() == 1
            || key
                .strip_prefix('f')
                .is_some_and(|n| n.parse::<u8>().is_ok()) =>
        {
            key
        }
        _ => return None,
    };
    let key = if key.chars().count() == 1 {
        stroke
            .key_char
            .as_deref()
            .filter(|s| s.chars().count() == 1 && !s.chars().any(char::is_control))
            .unwrap_or(key)
    } else {
        key
    };
    stonks_core::preferences::keybinds::normalize(&format!(
        "{}{}{}{}{}",
        if m.control { "Control+" } else { "" },
        if m.alt { "Alt+" } else { "" },
        if m.shift { "Shift+" } else { "" },
        if m.platform { "Meta+" } else { "" },
        if key.starts_with('f') && key.len() > 1 {
            key.to_uppercase()
        } else {
            key.to_owned()
        },
    ))
}

fn cycle<'a>(
    items: impl Iterator<Item = &'a str>,
    selected: Option<&str>,
    previous: bool,
) -> Option<String> {
    let items: Vec<_> = items.collect();
    let len = items.len();
    if len == 0 {
        return None;
    }
    let index = match items.iter().position(|id| Some(*id) == selected) {
        Some(i) => (i + if previous { len - 1 } else { 1 }) % len,
        None if previous => len - 1,
        None => 0,
    };
    Some(items[index].to_owned())
}

impl ShellView {
    pub(super) fn shortcut(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.is_held
            || event.prefer_character_input
            || self.voice.read(cx).camera_setup_open()
            || self.voice.read(cx).screen_setup_open()
            || self.voice.read(cx).soundboard_open()
            || self.voice.read(cx).music_playlist_open(cx)
            || self.has_overlay_other_than_message_panel(cx)
            || gpui_kit::base::GlobalState::is_in_deferred_context(cx)
            || self.composer.read(cx).shortcut_popup_open()
        {
            return false;
        }
        let Some(chord) = chord(&event.keystroke) else {
            return false;
        };
        let prefs = cx.global::<NativePreferences>();
        if prefs.load_error.is_some() {
            return false;
        }
        let Some(action) = prefs.saved.keybinds.matching(&chord) else {
            return false;
        };
        use Action::*;
        if self.message_panel.read(cx).mode().is_some() {
            let m = event.keystroke.modifiers;
            if !matches!(action, ToggleSearch | TogglePins) || (!m.control && !m.alt && !m.platform)
            {
                return false;
            }
        }
        if self.composer.read(cx).editor_focused(window, cx) {
            let m = event.keystroke.modifiers;
            if !matches!(action, JumpToLatest | UploadAttachment)
                || (!m.control && !m.alt && !m.platform && chord != "Escape")
            {
                return false;
            }
        }
        if action == JumpToLatest
            && self.focused
            && event.keystroke.key == "escape"
            && !self.timeline.read(cx).can_jump_to_latest()
        {
            return false;
        }
        match action {
            PreviousChannel | NextChannel => {
                let session = self.session.read(cx);
                let Some(snapshot) = &session.snapshot else {
                    return false;
                };
                let visible = |channel: &&stonks_core::connection::ClientChannel| {
                    !channel.is_voice() && session.can_view_channel(&channel.id)
                };
                let next = if let Some(server) = snapshot
                    .servers
                    .iter()
                    .find(|s| Some(s.id.as_str()) == self.selected_server.as_deref())
                {
                    cycle(
                        server
                            .channels
                            .iter()
                            .filter_map(|id| {
                                snapshot.channels.iter().find(|channel| &channel.id == id)
                            })
                            .filter(visible)
                            .map(|channel| channel.id.as_str()),
                        self.selected_channel.as_deref(),
                        action == PreviousChannel,
                    )
                } else if self.home
                    && (!self.focused || self.compact_nav.tab == compact::Tab::Messages)
                {
                    cycle(
                        snapshot
                            .channels
                            .iter()
                            .filter(|channel| channel.server.is_none())
                            .filter(visible)
                            .map(|channel| channel.id.as_str()),
                        self.selected_channel.as_deref(),
                        action == PreviousChannel,
                    )
                } else {
                    None
                };
                let Some(next) = next else {
                    return false;
                };
                self.select_text_view(next, cx);
            }
            PreviousServer | NextServer => {
                let Some(snapshot) = &self.session.read(cx).snapshot else {
                    return false;
                };
                let Some(next) = cycle(
                    snapshot.servers.iter().map(|server| server.id.as_str()),
                    self.selected_server.as_deref(),
                    action == PreviousServer,
                ) else {
                    return false;
                };
                if self.focused {
                    self.compact_nav.open_server(next);
                    self.apply_compact_route(window, cx);
                } else {
                    self.select_server(next, cx);
                }
            }
            ToggleMute | ToggleDeafen => {
                if !self
                    .voice
                    .read(cx)
                    .shortcut_toggle_audio(action == ToggleDeafen)
                {
                    return false;
                }
            }
            MarkServerRead => {
                let conversations =
                    self.home && (!self.focused || self.compact_nav.tab == compact::Tab::Messages);
                let server = self.selected_server.as_deref();
                if server.is_some() || conversations {
                    self.session.update(cx, |session, cx| {
                        session.mark_section_read(server, cx);
                    });
                }
            }
            _ if self.selected_channel.is_none() => return false,
            FocusComposer => self.focus_composer(window, cx),
            JumpToLatest => self
                .timeline
                .update(cx, |timeline, cx| timeline.jump_to_latest(cx)),
            ToggleSearch | TogglePins => self.toggle_message_panel(
                if action == ToggleSearch {
                    PanelMode::Search
                } else {
                    PanelMode::Pins
                },
                window,
                cx,
            ),
            UploadAttachment => {
                if !self.composer.read(cx).can_pick() {
                    return false;
                }
                self.composer
                    .update(cx, |composer, cx| composer.pick_files(window, cx));
            }
        }
        window.prevent_default();
        cx.stop_propagation();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{chord, cycle, Keystroke};

    #[test]
    fn shortcut_chords_translate_linux_keys_to_tauri_names() {
        let mut stroke = Keystroke::parse("ctrl-alt-up").unwrap();
        assert_eq!(chord(&stroke).as_deref(), Some("Control+Alt+ArrowUp"));
        stroke = Keystroke::parse("ctrl-shift-m").unwrap();
        assert_eq!(chord(&stroke).as_deref(), Some("Control+Shift+M"));
        assert_eq!(
            chord(&Keystroke::parse("f8").unwrap()).as_deref(),
            Some("F8")
        );
        assert_eq!(
            chord(&Keystroke::parse("shift-escape").unwrap()).as_deref(),
            Some("Shift+Escape")
        );
        stroke.key = "control".into();
        assert_eq!(chord(&stroke), None);
    }

    #[test]
    fn shortcut_navigation_wraps_and_handles_missing_selection() {
        let items = ["one", "two"];
        assert_eq!(
            cycle(items.into_iter(), Some("one"), true).as_deref(),
            Some("two")
        );
        assert_eq!(
            cycle(items.into_iter(), Some("two"), false).as_deref(),
            Some("one")
        );
        assert_eq!(
            cycle(items.into_iter(), None, false).as_deref(),
            Some("one")
        );
        assert_eq!(cycle(std::iter::empty(), None, false), None);
    }
}
