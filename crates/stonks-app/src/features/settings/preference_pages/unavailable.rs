//! Desktop preferences and reference rows without a native edit/apply path.
use crate::preferences::NativePreferences;
use gpui_kit::*;
use stonks_ui::{
    components::{settings_row, settings_section, Switch},
    i18n::{t, t_vars},
    Theme,
};

#[derive(IntoElement)]
struct Unavailable(Theme);

impl RenderOnce for Unavailable {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let locale = cx.global::<NativePreferences>().saved.effective_language();
        div()
            .text_color(self.0.text_muted)
            .text_size(px(self.0.font_size_xs))
            .child(if locale == "pt-BR" {
                "Indisponível neste cliente"
            } else {
                "Unavailable in this client"
            })
    }
}

pub(super) fn rows(theme: &Theme, group: &str, names: &[&str]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme.space2))
        .children(names.iter().map(|name| {
            let key = format!("settings.{group}.{name}");
            let desc_key = format!("{key}Description");
            let desc = t(&desc_key);
            settings_row(
                theme,
                &t(&key),
                if desc == desc_key { "" } else { &desc },
                Unavailable(*theme),
            )
        }))
}

fn desktop_toggle(
    theme: &Theme,
    key: &'static str,
    description: &str,
    checked: bool,
    change: fn(bool) -> super::Change,
    cx: &Context<super::State>,
) -> Div {
    let label = t(key);
    let prefs = cx.global::<NativePreferences>();
    if prefs.load_error.is_some() {
        return settings_row(theme, &label, description, Unavailable(*theme));
    }
    let state = cx.entity().downgrade();
    settings_row(
        theme,
        &label,
        description,
        Switch::new(key, *theme)
            .label(label.clone())
            .checked(checked)
            .on_change(move |enabled, window, cx| {
                let _ = state.update(cx, |this, cx| this.save(change(*enabled), window, cx));
            }),
    )
}

pub(super) fn desktop(theme: &Theme, window: &Window, cx: &Context<super::State>) -> Div {
    let label = t("settings.desktop.startComputer");
    let description = t("settings.desktop.startComputerDescription");
    let state = cx.entity().downgrade();
    let autostart = match crate::desktop::starts_with_computer() {
        Ok(enabled) if cfg!(target_os = "linux") => settings_row(
            theme,
            &label,
            &description,
            Switch::new("settings.desktop.startComputer", *theme)
                .label(label.clone())
                .checked(enabled)
                .on_change(move |enabled, _, cx| {
                    let _ = state.update(cx, |this, cx| {
                        this.save_error = crate::desktop::set_start_with_computer(*enabled)
                            .err()
                            .map(|_| super::PreferenceError::WriteFailed);
                        cx.notify();
                    });
                }),
        ),
        _ => settings_row(theme, &label, &description, Unavailable(*theme)),
    };
    let native = cx.global::<NativePreferences>();
    let prefs = &native.saved;
    let tray = if cfg!(any(target_os = "linux", target_os = "windows")) {
        desktop_toggle(
            theme,
            "settings.desktop.minimizeTray",
            &t("settings.desktop.nativeMinimizeTrayDescription"),
            prefs.minimize_to_tray,
            super::Change::MinimizeToTray,
            cx,
        )
    } else {
        rows(theme, "desktop", &["minimizeTray"])
    };
    let renderer = window.gpu_specs().map_or_else(
        || t("settings.desktop.rendererUnknown"),
        |specs| {
            t_vars(
                if specs.is_software_emulated {
                    "settings.desktop.rendererSoftware"
                } else {
                    "settings.desktop.rendererGpu"
                },
                &[("device", &specs.device_name)],
            )
        },
    );
    let mut renderer_description = format!(
        "{}\n{renderer}",
        t("settings.desktop.hardwareAccelerationDescription"),
    );
    if prefs.hardware_acceleration != native.startup_hardware_acceleration {
        renderer_description.push_str(&format!("\n{}", t("settings.desktop.rendererPending")));
    }
    let acceleration = if cfg!(target_os = "linux") {
        desktop_toggle(
            theme,
            "settings.desktop.hardwareAcceleration",
            &renderer_description,
            prefs.hardware_acceleration,
            super::Change::HardwareAcceleration,
            cx,
        )
    } else {
        rows(theme, "desktop", &["hardwareAcceleration"])
    };
    let start_minimized = if crate::desktop::supports_start_minimized(cx) {
        desktop_toggle(
            theme,
            "settings.desktop.startMinimized",
            &t("settings.desktop.nativeStartMinimizedDescription"),
            prefs.native_start_minimized,
            super::Change::NativeStartMinimized,
            cx,
        )
    } else {
        rows(theme, "desktop", &["startMinimized"])
    };
    div()
        .child(
            settings_section(theme, &t("settings.desktop.behaviour")).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .child(autostart)
                    .child(tray)
                    .child(start_minimized),
            ),
        )
        .child(
            settings_section(theme, &t("settings.desktop.integration")).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .child(desktop_toggle(
                        theme,
                        "settings.desktop.spellcheck",
                        &t("settings.desktop.spellcheckDescription"),
                        prefs.spellcheck,
                        super::Change::Spellcheck,
                        cx,
                    ))
                    .child(acceleration)
                    .child(desktop_toggle(
                        theme,
                        "settings.desktop.developerMode",
                        &t("settings.desktop.developerModeDescription"),
                        prefs.developer_mode,
                        super::Change::DeveloperMode,
                        cx,
                    )),
            ),
        )
        .child(crate::features::settings::updater_section::render(cx))
}

#[path = "keybinds.rs"]
mod keybind_page;

pub(super) fn keybinds(_theme: &Theme) -> Div {
    div().child(keybind_page::KeybindPage)
}
