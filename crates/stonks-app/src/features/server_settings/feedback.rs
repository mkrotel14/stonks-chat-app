//! Source createAutoDismiss: independent, entity-owned notice/error deadlines.
use super::*;
use std::time::Duration;
impl ServerSettings {
    pub(super) fn sync_feedback(&mut self, cx: &mut Context<Self>) {
        if self.extra.last_notice != self.notice {
            self.extra.last_notice = self.notice.clone();
            self.extra.notice_timer = None;
            if let Some(expected) = self.notice.clone() {
                self.extra.notice_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(4500))
                        .await;
                    this.update(cx, |this, cx| {
                        if this.notice.as_ref() == Some(&expected) {
                            this.notice = None;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
        }
        if self.extra.last_error != self.error {
            self.extra.last_error = self.error.clone();
            self.extra.error_timer = None;
            if let Some(expected) = self.error.clone() {
                self.extra.error_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(7000))
                        .await;
                    this.update(cx, |this, cx| {
                        if this.error.as_ref() == Some(&expected) {
                            this.error = None;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
        }
    }
}
