//! Application-wide GPUI release discovery. There is no installable state.

use gpui_kit::*;
use stonks_core::release_discovery::{latest_public_version, DiscoveryError};

use crate::runtime::spawn_core;

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
        if matches!(self.status, Status::Checking) {
            return;
        }
        self.status = Status::Checking;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_core(latest_public_version()).await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(version) => Status::PublicRelease(version.to_string()),
                    Err(error) => Status::Failed(error),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }
}
