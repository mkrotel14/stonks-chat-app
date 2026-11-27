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
use managed::{InstallEvent, Outcome};

#[path = "app_updater/details.rs"]
pub(crate) mod details;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
#[path = "app_updater/progress.rs"]
mod progress;

pub(crate) const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub(crate) struct Updater {
    pub(crate) status: Status,
    task: Option<Task<()>>,
    pub(crate) progress: Option<(u64, u64)>,
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "linux", target_os = "windows")
    ))]
    retry: Option<Candidate>,
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
    Downloading,
    Verifying,
    Installing,
    #[cfg(target_os = "linux")]
    Restarting,
    #[cfg(target_os = "windows")]
    InstallerLaunched,
    InstallFailed(String),
}

impl Status {
    pub(crate) fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading | Self::Verifying | Self::Installing
        )
    }

    pub(crate) fn handing_off(&self) -> bool {
        #[cfg(target_os = "linux")]
        if matches!(self, Self::Restarting) {
            return true;
        }
        #[cfg(target_os = "windows")]
        if matches!(self, Self::InstallerLaunched) {
            return true;
        }
        false
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
        if self.status.busy() || self.status.handing_off() {
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
    pub(crate) fn begin_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Status::Available(candidate) = &self.status else {
            return;
        };
        let candidate = candidate.clone();
        progress::open(window, cx);
        self.install(candidate, cx);
    }

    pub(crate) fn retry_install(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::InstallFailed(_)) {
            if let Some(candidate) = self.retry.clone() {
                self.install(candidate, cx);
            }
        }
    }

    pub(crate) fn dismiss_install_failure(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::InstallFailed(_)) {
            if let Some(candidate) = self.retry.take() {
                self.status = Status::Available(candidate);
                cx.notify();
            }
        }
    }

    fn install(&mut self, candidate: Candidate, cx: &mut Context<Self>) {
        self.retry = Some(candidate.clone());
        self.progress = None;
        self.status = Status::Downloading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let (sender, mut receiver) = tokio::sync::watch::channel(None);
            let install = spawn_core(managed::install_with_events(candidate, move |event| {
                sender.send_replace(Some(event));
            }));
            while receiver.changed().await.is_ok() {
                if let Some(event) = *receiver.borrow_and_update() {
                    let _ = this.update(cx, |updater, cx| {
                        updater.status = match event {
                            InstallEvent::DownloadProgress { downloaded, total } => {
                                updater.progress = Some((downloaded, total));
                                Status::Downloading
                            }
                            InstallEvent::Verifying => Status::Verifying,
                            InstallEvent::Installing => Status::Installing,
                        };
                        cx.notify();
                    });
                }
            }
            let result = install.await;
            let _ = this.update(cx, |updater, cx| {
                match result {
                    #[cfg(target_os = "linux")]
                    Ok(Outcome::Restart(path)) => {
                        updater.status = Status::Restarting;
                        cx.set_restart_path(path);
                        cx.restart();
                    }
                    #[cfg(target_os = "windows")]
                    Ok(Outcome::HandedOff) => {
                        updater.status = Status::InstallerLaunched;
                        cx.quit();
                    }
                    Err(error) => updater.status = Status::InstallFailed(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
