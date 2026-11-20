//! Stonks Chat — the GPUI application.
//!
//! Bootstrap only: window, theme, fonts, kit init. Features live under
//! `features/` by product context. Shared UI services live under `shared/`;
//! session and runtime infrastructure sit outside the feature tree.

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod desktop;
mod features;
mod preferences;
mod runtime;
mod session;
mod shared;

use gpui_kit::component::theme::{Theme as KitTheme, ThemeMode};
use stonks_ui::fonts;

fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let launch = match desktop::Launch::from_environment() {
        Ok(Some(launch)) => launch,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Stonks desktop startup failed: {}", error.kind());
            return ExitCode::FAILURE;
        }
    };
    #[cfg(not(target_os = "windows"))]
    let preferences_path = features::app::shared_config_path()
        .with_file_name(stonks_core::preferences::PREFERENCES_FILE);
    #[cfg(target_os = "windows")]
    let preferences_path = match desktop::config_home() {
        Ok(base) => base
            .join("com.rdxlabs.stonks")
            .join(stonks_core::preferences::PREFERENCES_FILE),
        Err(error) => {
            eprintln!("Stonks desktop configuration unavailable: {}", error.kind());
            return ExitCode::FAILURE;
        }
    };
    let mut preferences = preferences::NativePreferences::load(&preferences_path);
    let software_only = cfg!(target_os = "linux")
        && !preferences.saved.hardware_acceleration
        && !launch.automatic_renderer_override;
    preferences.startup_hardware_acceleration = !software_only;
    #[cfg(target_os = "linux")]
    {
        use gpui_wgpu::{set_native_adapter_policy, NativeAdapterPolicy};
        let policy = if software_only {
            NativeAdapterPolicy::SoftwareOnly
        } else {
            NativeAdapterPolicy::Automatic
        };
        if set_native_adapter_policy(policy).is_err() {
            eprintln!("Stonks renderer policy was already initialized");
            return ExitCode::FAILURE;
        }
    }
    let opened = std::rc::Rc::new(std::cell::Cell::new(false));
    let startup = opened.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            KitTheme::change(ThemeMode::Dark, None, cx);
            if let Some(error) = &preferences.load_error {
                eprintln!("preferences: saved preferences unavailable: {error:?}");
            }
            preferences.install(cx);
            startup.set(launch.run(cx));
        });
    if opened.get() {
        ExitCode::SUCCESS
    } else {
        if software_only {
            eprintln!("Software-only startup failed. Install a compatible CPU graphics adapter, or relaunch with --automatic-renderer and re-enable Hardware acceleration in Desktop settings. The saved preference was not changed.");
        }
        ExitCode::FAILURE
    }
}
