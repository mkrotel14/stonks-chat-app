//! Native preference bootstrap and application. The host supplies the config path.
use gpui_kit::{App, Global, PromptButton, PromptLevel, Window, WindowAppearance};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use stonks_core::preferences::{self, Change, PreferenceError, Preferences, ThemePreference};

#[derive(Clone, Debug)]
pub struct NativePreferences {
    pub saved: Preferences,
    pub path: PathBuf,
    pub load_error: Option<PreferenceError>,
    pub startup_hardware_acceleration: bool,
    motion_baseline: bool,
    link_prompt_pending: Arc<AtomicBool>,
}
impl Global for NativePreferences {}
impl NativePreferences {
    pub fn load(path: &Path) -> Self {
        match preferences::load_file(path) {
            Ok(saved) => Self {
                path: path.to_owned(),
                startup_hardware_acceleration: saved.hardware_acceleration,
                saved,
                motion_baseline: false,
                link_prompt_pending: Default::default(),
                load_error: None,
            },
            Err(error) => Self {
                path: path.to_owned(),
                saved: Preferences::privacy_safe_fallback(),
                startup_hardware_acceleration: true,
                motion_baseline: false,
                link_prompt_pending: Default::default(),
                load_error: Some(error),
            },
        }
    }
    /// Install before constructing auth/shell views. Theme installation stays
    /// host-owned because `system` depends on the current window appearance.
    pub fn install(mut self, cx: &mut App) {
        self.motion_baseline = cx
            .try_global::<Self>()
            .map_or_else(|| cx.reduce_motion(), |current| current.motion_baseline);
        self.apply_presentation(cx);
        cx.set_global(self);
        cx.set_global(stonks_ui::external_links::ExternalLinkHandler(
            Self::open_external_link,
        ));
    }

    fn apply_presentation(&self, cx: &mut App) {
        stonks_ui::i18n::configure(
            self.saved.effective_language(),
            self.saved.date_format.as_str(),
            self.saved.time_format.as_str(),
        );
        cx.set_reduce_motion(self.saved.reduced_motion || self.motion_baseline);
    }

    pub fn commit(
        change: Change,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), PreferenceError> {
        let mut current = cx.global::<Self>().clone();
        let before = current.saved.clone();
        current.saved = preferences::write_change(&current.path, change)?;
        current.load_error = None;
        let theme_changed =
            before.theme != current.saved.theme || before.accent != current.saved.accent;
        current.apply_presentation(cx);
        cx.set_global(current);
        if theme_changed {
            Self::apply_theme(window, cx);
        }
        cx.refresh_windows();
        Ok(())
    }

    fn open_external_link(value: &str, window: &mut Window, cx: &mut App) {
        use stonks_ui::i18n::{t, t_vars};

        let Some(url) = preferences::external_web_url(value) else {
            return;
        };
        let current = cx.global::<Self>();
        let pending = current.link_prompt_pending.clone();
        if pending.load(Ordering::Relaxed) || window.has_active_prompt() {
            return;
        }
        if !current.saved.warns_for_link(&url) {
            cx.open_url(url.as_str());
            return;
        }
        pending.store(true, Ordering::Relaxed);
        let origin = url.origin().ascii_serialization();
        let answer = window.prompt(
            PromptLevel::Warning,
            &t("messages.externalLinkWarning"),
            Some(&format!("{origin}\n{}", url.as_str())),
            &[
                PromptButton::cancel(t("common.cancel")),
                PromptButton::new(t("messages.openLink")),
                PromptButton::new(t_vars("messages.trustLinkOrigin", &[("origin", &origin)])),
            ],
            cx,
        );
        window
            .spawn(cx, async move |cx| {
                let answer = answer.await;
                pending.store(false, Ordering::Relaxed);
                if let Ok(choice @ (1 | 2)) = answer {
                    let _ = cx.update(|window, cx| {
                        if choice == 2 {
                            if let Err(error) =
                                Self::commit(Change::TrustLinkOrigin(origin), window, cx)
                            {
                                let _ = window.prompt(
                                    PromptLevel::Warning,
                                    &error.to_string(),
                                    None,
                                    &[PromptButton::cancel(t("common.cancel"))],
                                    cx,
                                );
                                return;
                            }
                        }
                        cx.open_url(url.as_str());
                    });
                }
            })
            .detach();
    }

    /// Also used by a host on startup and on window appearance changes.
    pub fn apply_theme(window: &mut Window, cx: &mut App) {
        use gpui_kit::component::theme::{Theme as KitTheme, ThemeMode};
        use stonks_ui::{theme::CurrentTheme, Theme};
        let saved = &cx.global::<Self>().saved;
        let light = match saved.theme {
            ThemePreference::Light => true,
            ThemePreference::Dark => false,
            ThemePreference::System => matches!(
                window.appearance(),
                WindowAppearance::Light | WindowAppearance::VibrantLight
            ),
        };
        let theme =
            if light { Theme::light() } else { Theme::dark() }.with_accent(saved.accent.as_str());
        cx.set_global(CurrentTheme(theme));
        KitTheme::change(
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            Some(window),
            cx,
        );
    }
}
