//! Source notification preferences: optimistic edits, ordered writes and rollback.
use crate::runtime::spawn_core;
use gpui_kit::*;
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use stonks_core::channel_preferences::{self, Change, ChannelPreferences};
pub(in crate::features::shell) struct Notifications {
    pub saved: ChannelPreferences,
    pub ready: bool,
    pub error: Option<String>,
    path: PathBuf,
    queue: VecDeque<(String, Change)>,
    writing: bool,
    request: Option<Task<()>>,
    _timer: Task<()>,
}
pub(in crate::features::shell) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn replay(saved: &mut ChannelPreferences, queue: &VecDeque<(String, Change)>) {
    for (channel, change) in queue {
        saved.apply(channel, *change);
    }
}
impl Notifications {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let timer = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        });
        let mut this = Self {
            saved: Default::default(),
            ready: false,
            error: None,
            path,
            queue: VecDeque::new(),
            writing: false,
            request: None,
            _timer: timer,
        };
        this.retry(cx);
        this
    }
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if self.writing {
            return;
        }
        self.error = None;
        let path = self.path.clone();
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = spawn_core(async move {
                tokio::task::spawn_blocking(move || channel_preferences::load(&path))
                    .await
                    .unwrap_or_else(|_| Err("Could not load channel preferences.".into()))
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(mut saved) => {
                        replay(&mut saved, &this.queue);
                        this.saved = saved;
                        this.ready = true;
                    }
                    Err(error) => {
                        this.error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }
    pub fn change(&mut self, channel: String, change: Change, cx: &mut Context<Self>) {
        if !self.ready {
            return;
        }
        self.error = None;
        self.saved.apply(&channel, change);
        self.queue.push_back((channel, change));
        if !self.writing {
            self.flush(cx);
        }
        cx.notify();
    }
    fn flush(&mut self, cx: &mut Context<Self>) {
        let Some((channel, change)) = self.queue.pop_front() else {
            self.writing = false;
            cx.notify();
            return;
        };
        self.writing = true;
        let path = self.path.clone();
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = spawn_core(async move {
                tokio::task::spawn_blocking(move || {
                    match channel_preferences::write_change(&path, &channel, change) {
                        Ok(saved) => (Some(saved), None),
                        Err(error) => (channel_preferences::load(&path).ok(), Some(error)),
                    }
                })
                .await
                .unwrap_or_else(|_| (None, Some("Could not save channel preferences.".into())))
            })
            .await;
            this.update(cx, |this, cx| {
                let (saved, error) = result;
                if let Some(mut saved) = saved {
                    replay(&mut saved, &this.queue);
                    this.saved = saved;
                }
                if let Some(error) = error {
                    this.error = Some(error);
                }
                this.flush(cx);
            })
            .ok();
        }));
    }
}
#[cfg(test)]
mod tests {
    use super::{replay, Change, ChannelPreferences, VecDeque};
    use stonks_core::channel_preferences::NotificationMode;
    #[test]
    fn save_ack_or_rollback_preserves_newer_optimistic_edits() {
        let mut saved = ChannelPreferences::default();
        saved.apply("a", Change::Notifications(Some(NotificationMode::All)));
        let queue = VecDeque::from([
            (
                "a".into(),
                Change::Notifications(Some(NotificationMode::Mention)),
            ),
            ("b".into(), Change::Mute(Some(None))),
        ]);
        replay(&mut saved, &queue);
        assert_eq!(
            saved.channel_notification_modes["a"],
            NotificationMode::Mention
        );
        assert!(saved.is_muted("b", i64::MAX));
    }
}
