//! Page bodies for the user settings host. The host owns navigation and scrolling.
//!
//! Declare `mod preference_pages;`, create one `Entity<State>` with
//! `cx.new(|cx| State::new(window, cx))` after preference bootstrap, and retain
//! `cx.observe(&state, |_, _, cx| cx.notify())` in the host. Map the six tabs
//! to `Page` and insert `render(page, &state, window, cx)` into the content pane.
mod disclosure;
mod unavailable;
mod voice;
mod voice_sounds;

use crate::preferences::NativePreferences;
use gpui_kit::*;
use stonks_core::preferences::{
    Change, DateFormat, Language, PreferenceError, Preferences, ThemePreference, TimeFormat,
};
use stonks_ui::{
    components::{
        settings_row, settings_section, Button, ButtonVariant, Switch, UiAccordion, UiColorPicker,
        UiColorPickerEvent, UiColorPickerState,
    },
    i18n::t,
    Theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Language,
    Notifications,
    Voice,
    Desktop,
    Keybinds,
}

/// Allocate once per settings window, after `NativePreferences::install`.
/// Keep this entity alive when changing tabs; do not allocate it per render.
pub struct State {
    accent: Entity<UiColorPickerState>,
    language_fields: [disclosure::Disclosure; 3],
    sounds_open: bool,
    voice_sounds: voice_sounds::VoiceSounds,
    voice: voice::VoiceFields,
    save_error: Option<PreferenceError>,
    _accent_subscription: Subscription,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let accent = cx.new(|cx| UiColorPickerState::new(window, cx));
        sync_accent(&accent, window, cx);
        let subscription = cx.subscribe_in(&accent, window, |this, picker, event, window, cx| {
            if let UiColorPickerEvent::Change(Some(color)) = event {
                let c = Rgba::from(*color);
                if !this.save(
                    Change::Accent([c.r, c.g, c.b].map(|c| (255. * c).round() as u8)),
                    window,
                    cx,
                ) {
                    sync_accent(&picker, window, cx);
                }
            }
        });
        Self {
            accent,
            language_fields: Default::default(),
            sounds_open: false,
            voice_sounds: Default::default(),
            voice: Default::default(),
            save_error: None,
            _accent_subscription: subscription,
        }
    }

    fn save(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.save_error = NativePreferences::commit(change, window, cx).err();
        cx.notify();
        self.save_error.is_none()
    }
}

fn sync_accent(picker: &Entity<UiColorPickerState>, window: &mut Window, cx: &mut App) {
    let accent = cx.global::<NativePreferences>().saved.accent.as_str();
    // Accent is validated by the core preference projection.
    if let Ok(value) = u32::from_str_radix(&accent[1..], 16) {
        picker.update(cx, |state, cx| state.set_value(rgb(value), window, cx));
    }
}

/// Render into the host's settings content pane (no tab header or scroll container).
/// Call from the host's render method with `&mut App` (a `Context` coerces to it).
pub fn render(page: Page, state: &Entity<State>, window: &mut Window, cx: &mut App) -> Div {
    state.update(cx, |state, cx| {
        let theme = Theme::current(cx);
        let prefs = cx.global::<NativePreferences>().clone();
        let disabled = prefs.load_error.is_some();
        let body = match page {
            Page::Appearance => appearance(state, &theme, &prefs.saved, disabled, cx),
            Page::Language => language(state, &theme, &prefs.saved, disabled, cx),
            Page::Notifications => notifications(state, &theme, &prefs.saved, disabled, cx),
            Page::Voice => voice::render(state, &theme, &prefs.saved, disabled, window, cx),
            Page::Desktop => unavailable::desktop(&theme, window, cx),
            Page::Keybinds => unavailable::keybinds(&theme),
        };
        let mut content = div().flex().flex_col().min_w_0();
        if let Some(error) = state.save_error.or(prefs.load_error) {
            content = content.child(div().text_color(theme.danger).child(error.to_string()));
        }
        content.child(body)
    })
}

fn appearance(
    state: &State,
    theme: &Theme,
    prefs: &Preferences,
    disabled: bool,
    cx: &Context<State>,
) -> Div {
    let mut choices = div()
        .flex()
        .flex_wrap()
        .max_w(px(570. + 2. * theme.space3))
        .gap(px(theme.space3));
    for (value, key) in [
        (ThemePreference::Light, "light"),
        (ThemePreference::Dark, "dark"),
        (ThemePreference::System, "system"),
    ] {
        let state = cx.entity().downgrade();
        let preview = |theme: Theme| {
            div()
                .flex()
                .flex_1()
                .h_full()
                .bg(theme.surface)
                .child(div().w(relative(0.28)).h_full().bg(theme.sidebar))
        };
        let mut sample = div()
            .flex()
            .w_full()
            .h(px(74.))
            .overflow_hidden()
            .rounded(px(theme.radius_sm));
        if value != ThemePreference::Dark {
            sample = sample.child(preview(Theme::light()));
        }
        if value != ThemePreference::Light {
            sample = sample.child(preview(Theme::dark()));
        }
        let label = t(&format!("settings.appearance.{key}"));
        choices = choices.child(
            div().flex_1().min_w(px(160.)).max_w_full().child(
                Button::new(key, *theme)
                    .variant(ButtonVariant::Choice)
                    .accessibility_label(label.clone())
                    .leading(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(theme.space2))
                            .child(sample)
                            .child(
                                div()
                                    .px(px(theme.space1))
                                    .pb(px(theme.space1))
                                    .text_size(px(theme.font_size_sm))
                                    .font_weight(FontWeight::BOLD)
                                    .child(label),
                            ),
                    )
                    .selected(prefs.theme == value)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        let _ = state
                            .update(cx, |this, cx| this.save(Change::Theme(value), window, cx));
                    }),
            ),
        );
    }
    let accent = t("settings.appearance.accent");
    div()
        .child(settings_section(theme, &t("settings.appearance.theme")).child(choices))
        .child(
            settings_section(theme, &t("settings.appearance.personalisation"))
                .child(settings_row(
                    theme,
                    &accent,
                    &t("settings.appearance.accentDescription"),
                    UiColorPicker::new(&state.accent, *theme)
                        .label(accent.clone())
                        .disabled(disabled),
                ))
                .child(toggle(
                    theme,
                    "settings.appearance.compact",
                    prefs.compact_mode,
                    disabled,
                    Change::CompactMode,
                    cx,
                ))
                .child(toggle(
                    theme,
                    "settings.appearance.motion",
                    prefs.reduced_motion,
                    disabled,
                    Change::ReducedMotion,
                    cx,
                )),
        )
}

const LANGUAGE: &[(&str, &str, Change)] = &[
    ("en", "english", Change::Language(Language::English)),
    (
        "pt-BR",
        "portuguese",
        Change::Language(Language::Portuguese),
    ),
];
const DATE: &[(&str, &str, Change)] = &[
    (
        "day-first",
        "dateDayFirst",
        Change::DateFormat(DateFormat::DayFirst),
    ),
    (
        "month-first",
        "dateMonthFirst",
        Change::DateFormat(DateFormat::MonthFirst),
    ),
    (
        "year-first",
        "dateYearFirst",
        Change::DateFormat(DateFormat::YearFirst),
    ),
];
const TIME: &[(&str, &str, Change)] = &[
    (
        "24-hour",
        "time24",
        Change::TimeFormat(TimeFormat::TwentyFourHour),
    ),
    (
        "12-hour",
        "time12",
        Change::TimeFormat(TimeFormat::TwelveHour),
    ),
];

fn language(
    state: &mut State,
    theme: &Theme,
    prefs: &Preferences,
    disabled: bool,
    cx: &Context<State>,
) -> Div {
    type Access = fn(&mut State) -> &mut disclosure::Disclosure;
    let specs: [(_, _, _, Access); 3] = [
        ("language", LANGUAGE, prefs.effective_language(), |state| {
            &mut state.language_fields[0]
        }),
        ("dateFormat", DATE, prefs.date_format.as_str(), |state| {
            &mut state.language_fields[1]
        }),
        ("timeFormat", TIME, prefs.time_format.as_str(), |state| {
            &mut state.language_fields[2]
        }),
    ];
    let mut section = settings_section(theme, &t("settings.language.heading"));
    for (id, choices, value, access) in specs {
        let selected = choices
            .iter()
            .position(|(key, _, _)| *key == value)
            .unwrap_or(0);
        let options = choices
            .iter()
            .map(|(_, label, change)| disclosure::Choice {
                label: t(&format!("settings.language.{label}")),
                description: (id == "language")
                    .then(|| t(&format!("settings.language.{label}Description"))),
                change: Some(change.clone()),
            })
            .collect();
        section = section.child(disclosure::render(
            access(state),
            access,
            id,
            theme,
            t(&format!("settings.language.{id}")),
            None,
            options,
            selected,
            disabled,
            cx,
        ));
    }
    section
}

fn toggle(
    theme: &Theme,
    key: &'static str,
    checked: bool,
    disabled: bool,
    change: fn(bool) -> Change,
    cx: &Context<State>,
) -> Div {
    let label = t(key);
    let state = cx.entity().downgrade();
    settings_row(
        theme,
        &label,
        &t(&format!("{key}Description")),
        Switch::new(key, *theme)
            .label(label.clone())
            .checked(checked)
            .disabled(disabled)
            .on_change(move |value, window, cx| {
                let _ = state.update(cx, |this, cx| this.save(change(*value), window, cx));
            }),
    )
}

fn notifications(
    state: &mut State,
    theme: &Theme,
    prefs: &Preferences,
    disabled: bool,
    cx: &mut Context<State>,
) -> Div {
    let weak = cx.entity().downgrade();
    let sounds = UiAccordion::new(
        "notification-sounds",
        *theme,
        t("settings.notifications.sounds"),
    )
    .subtitle(t("settings.notifications.soundsDescription"))
    .open(state.sounds_open)
    .on_toggle(move |_, _, cx| {
        let _ = weak.update(cx, |state, cx| {
            state.sounds_open = !state.sounds_open;
            cx.notify();
        });
    })
    .child(
        div()
            .flex()
            .flex_col()
            .gap(px(theme.space2))
            .child(toggle(
                theme,
                "settings.notifications.playSounds",
                prefs.notification_sounds,
                disabled,
                Change::NotificationSounds,
                cx,
            ))
            .child(toggle(
                theme,
                "settings.notifications.direct",
                prefs.direct_message_sounds,
                disabled || !prefs.notification_sounds,
                Change::DirectMessageSounds,
                cx,
            ))
            .child(toggle(
                theme,
                "settings.notifications.server",
                prefs.server_message_sounds,
                disabled || !prefs.notification_sounds,
                Change::ServerMessageSounds,
                cx,
            ))
            .child(toggle(
                theme,
                "settings.notifications.mentions",
                prefs.mention_sounds,
                disabled || !prefs.notification_sounds,
                Change::MentionSounds,
                cx,
            ))
            .child(toggle(
                theme,
                "settings.notifications.calls",
                prefs.call_sounds,
                disabled || !prefs.notification_sounds,
                Change::CallSounds,
                cx,
            )),
    );
    div()
        .flex()
        .flex_col()
        .gap(px(theme.space2))
        .child(
            settings_section(theme, &t("settings.notifications.messages")).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .child(toggle(
                        theme,
                        "settings.notifications.desktop",
                        prefs.desktop_notifications,
                        disabled,
                        Change::DesktopNotifications,
                        cx,
                    ))
                    .child(toggle(
                        theme,
                        "settings.notifications.mentionsOnly",
                        prefs.mentions_only,
                        disabled || !prefs.desktop_notifications,
                        Change::MentionsOnly,
                        cx,
                    )),
            ),
        )
        .child(sounds)
        .child(state.voice_sounds.render(theme, cx))
}
