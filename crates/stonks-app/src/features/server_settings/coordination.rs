//! Page transitions, dismissal, and permission-driven cancellation.
use super::*;

impl ServerSettings {
    pub fn set_can_manage(&mut self, allowed: bool, cx: &mut Context<Self>) {
        if self.can_manage == allowed {
            return;
        }
        self.can_manage = allowed;
        if !allowed {
            self.extra.picker = None;
            self.extra.emoji.preview_task = None;
            self.extra.picker_generation = self.extra.picker_generation.wrapping_add(1);
            self.save_request = None;
            self.save_generation = self.save_generation.wrapping_add(1);
            self.busy = false;
            self.list_request = None;
            self.generation = self.generation.wrapping_add(1);
            self.loading = false;
            self.list_error = None;
            self.invites = None;
            self.error = None;
        }
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.dismiss_navigation(cx) || self.busy {
            return;
        }
        self.list_request = None;
        cx.emit(ServerSettingsEvent::Close);
    }
    fn dismiss_navigation(&mut self, cx: &mut Context<Self>) -> bool {
        if self.extra.dialog.is_some() {
            if !self.busy {
                self.extra.dialog = None;
                if !self.extra.frame_visible {
                    cx.emit(ServerSettingsEvent::Close);
                }
                self.extra.restore_focus = true;
                cx.notify();
            }
            return true;
        }
        if self.extra.frame_visible && self.compact_layout && !self.settings_index {
            if !self.busy {
                self.settings_index = true;
                self.extra.previous_focus =
                    Some(self.extra.navigation_focus[self.page.index()].clone());
                self.extra.restore_focus = true;
                cx.notify();
            }
            return true;
        }
        false
    }
    pub(super) fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            self.list_request = None;
            self.generation = self.generation.wrapping_add(1);
            self.loading = false;
            self.list_error = None;
            self.page = page;
            self.error = None;
            self.notice = None;
        }
        if self.compact_layout {
            self.extra.previous_focus = Some(self.extra.heading_focus.clone());
            self.extra.restore_focus = true;
        }
        self.settings_index = false;
        if (page == Page::Emojis && self.emojis.is_none())
            || (page == Page::Invites && self.invites.is_none())
        {
            self.load(cx);
        }
        cx.notify();
    }
}
