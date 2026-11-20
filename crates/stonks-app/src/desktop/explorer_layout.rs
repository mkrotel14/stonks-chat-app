//! Desktop explorer preferences, independent of the current viewport.
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, fs::File, io::Read, path::PathBuf, rc::Rc};

pub(crate) const CHANNEL_MIN: f32 = 150.;
pub(crate) const CHANNEL_MAX: f32 = 380.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct ExplorerPreferences {
    pub(crate) rail_hidden: bool,
    pub(crate) channel_collapsed: bool,
    pub(crate) channel_expanded_width: Option<f32>,
}

impl ExplorerPreferences {
    pub(crate) fn channel_width(self, viewport: f32) -> f32 {
        self.channel_expanded_width
            .unwrap_or(if viewport <= 1320. { 250. } else { 292. })
    }
}

#[derive(Clone)]
pub(super) struct ExplorerLayout(Rc<RefCell<Store>>);
impl Global for ExplorerLayout {}

struct Store {
    path: Option<PathBuf>,
    current: ExplorerPreferences,
    written: ExplorerPreferences,
}

impl ExplorerLayout {
    pub(super) fn load(persist: bool) -> Self {
        let path = persist
            .then(super::config_home)
            .and_then(Result::ok)
            .map(|dir| dir.join(super::APP_ID).join("explorer-layout.json"));
        Self(Rc::new(RefCell::new(Store::load(path))))
    }

    pub(super) fn flush(&self) {
        if let Err(error) = self.0.borrow_mut().flush() {
            eprintln!("Stonks explorer layout save failed: {}", error.kind());
        }
    }
}

pub(crate) fn preferences(cx: &App) -> ExplorerPreferences {
    cx.try_global::<ExplorerLayout>()
        .map_or_else(ExplorerPreferences::default, |layout| {
            layout.0.borrow().current
        })
}

pub(crate) fn update(cx: &App, change: impl FnOnce(&mut ExplorerPreferences)) {
    if let Some(layout) = cx.try_global::<ExplorerLayout>() {
        change(&mut layout.0.borrow_mut().current);
    }
}

impl Store {
    fn load(path: Option<PathBuf>) -> Self {
        let mut current: ExplorerPreferences = path
            .as_ref()
            .and_then(|path| {
                let mut data = Vec::new();
                File::open(path)
                    .ok()?
                    .take(4097)
                    .read_to_end(&mut data)
                    .ok()?;
                (data.len() <= 4096).then_some(())?;
                serde_json::from_slice(&data).ok()
            })
            .unwrap_or_default();
        current.channel_expanded_width = current
            .channel_expanded_width
            .filter(|width| width.is_finite() && *width >= CHANNEL_MIN)
            .map(|width| width.min(CHANNEL_MAX).round());
        Self {
            path,
            current,
            written: current,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            if self.written != self.current {
                super::atomic_write(path, &serde_json::to_vec(&self.current)?)?;
                self.written = self.current;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_collapsed_explorers_and_remembers_expanded_width_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("explorer-layout.json");
        let mut store = Store::load(Some(path.clone()));
        store.flush().unwrap();
        assert!(!path.exists());
        store.current = ExplorerPreferences {
            rail_hidden: true,
            channel_collapsed: true,
            channel_expanded_width: Some(321.),
        };
        store.flush().unwrap();
        let mut restored = Store::load(Some(path.clone()));
        assert_eq!(restored.current, store.current);
        assert_eq!(restored.current.channel_width(850.), 321.);
        restored.current.channel_collapsed = false;
        restored.flush().unwrap();
        assert_eq!(Store::load(Some(path)).current, restored.current);
    }

    #[test]
    fn invalid_width_does_not_discard_visibility_and_bad_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("explorer-layout.json");
        std::fs::write(&path, r#"{"rail_hidden":true,"channel_expanded_width":0}"#).unwrap();
        let preferences = Store::load(Some(path.clone())).current;
        assert!(preferences.rail_hidden);
        assert_eq!(preferences.channel_width(1320.), 250.);
        assert_eq!(preferences.channel_width(1321.), 292.);
        std::fs::write(&path, "invalid").unwrap();
        assert_eq!(
            Store::load(Some(path)).current,
            ExplorerPreferences::default()
        );
    }
}
