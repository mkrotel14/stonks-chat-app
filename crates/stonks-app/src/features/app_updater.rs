//! Application-wide GPUI release discovery and confirmed managed updates.

use gpui_kit::*;
use stonks_core::release_discovery::{latest_public_version, DiscoveryError};

use crate::runtime::spawn_core;

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
#[path = "app_updater/managed.rs"]
mod managed;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
pub(crate) use managed::Candidate;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
use managed::{InstalledAction, Kind};

pub(crate) const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub(crate) struct Updater {
    pub(crate) status: Status,
    task: Option<Task<()>>,
}

#[derive(Default)]
pub(crate) enum Status {
    #[default]
    Idle,
    Checking,
    PublicRelease(String),
    Failed(DiscoveryError),
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "linux", target_os = "windows")
    ))]
    Available(Candidate),
    Current,
    Confirming,
    Downloading,
    Installing,
    Installed,
    InstallFailed(String),
}

impl Status {
    pub(crate) fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Confirming | Self::Downloading | Self::Installing
        )
    }
}

struct SharedUpdater(Entity<Updater>);
impl Global for SharedUpdater {}

impl Updater {
    pub(crate) fn shared(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<SharedUpdater>()
            .map(|state| state.0.clone())
    }

    pub(super) fn start(cx: &mut App) -> Entity<Self> {
        if let Some(updater) = Self::shared(cx) {
            return updater;
        }
        let updater = cx.new(|cx| {
            let mut updater = Self::default();
            updater.check(cx);
            updater
        });
        cx.set_global(SharedUpdater(updater.clone()));
        updater
    }

    pub(crate) fn check(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() || matches!(self.status, Status::Installed) {
            return;
        }
        self.status = Status::Checking;
        self.task = Some(cx.spawn(async move |this, cx| {
            let status = spawn_core(async {
                #[cfg(all(
                    target_arch = "x86_64",
                    any(target_os = "linux", target_os = "windows")
                ))]
                if let Some(kind) = tokio::task::spawn_blocking(managed::managed_kind)
                    .await
                    .ok()
                    .flatten()
                {
                    return match managed::check(kind).await {
                        Ok(Some(candidate)) => Status::Available(candidate),
                        Ok(None) => Status::Current,
                        Err(error) => Status::InstallFailed(error),
                    };
                }
                match latest_public_version().await {
                    Ok(version) => Status::PublicRelease(version.to_string()),
                    Err(error) => Status::Failed(error),
                }
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.status = status;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
impl Updater {
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use stonks_ui::i18n::t;

        let Status::Available(candidate) = &self.status else {
            return;
        };
        if window.has_active_prompt() {
            return;
        }
        let candidate = candidate.clone();
        let answer = window.prompt(
            PromptLevel::Warning,
            &t("settings.desktop.archConfirm"),
            Some(&format!(
                "{}\n{}",
                candidate.label(),
                t(match candidate.kind() {
                    Kind::Arch => "settings.desktop.archConfirmDetail",
                    Kind::Rpm => "settings.desktop.rpmConfirmDetail",
                    Kind::Windows => "settings.desktop.windowsConfirmDetail",
                })
            )),
            &[
                PromptButton::cancel(t("common.cancel")),
                PromptButton::new(t("settings.desktop.archInstall")),
            ],
            cx,
        );
        self.status = Status::Confirming;
        self.task = Some(cx.spawn(async move |this, cx| {
            if !matches!(answer.await, Ok(1)) {
                let _ = this.update(cx, |updater, cx| {
                    updater.status = Status::Available(candidate);
                    cx.notify();
                });
                return;
            }
            if this
                .update(cx, |updater, cx| {
                    updater.status = Status::Downloading;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let (sender, receiver) = futures_channel::oneshot::channel();
            let installer = spawn_core(managed::install(candidate, move || {
                let _ = sender.send(());
            }));
            if receiver.await.is_ok() {
                let _ = this.update(cx, |updater, cx| {
                    updater.status = Status::Installing;
                    cx.notify();
                });
            }
            let result = installer.await;
            let _ = this.update(cx, |updater, cx| {
                updater.status = match &result {
                    Ok(_) => Status::Installed,
                    Err(error) => Status::InstallFailed(error.clone()),
                };
                cx.notify();
                match result {
                    #[cfg(target_os = "linux")]
                    Ok(InstalledAction::Restart) => {
                        cx.set_restart_path("/usr/bin/stonks-app".into());
                        cx.restart();
                    }
                    #[cfg(target_os = "windows")]
                    Ok(InstalledAction::Quit) => cx.quit(),
                    _ => {}
                }
            });
        }));
        cx.notify();
    }
}
