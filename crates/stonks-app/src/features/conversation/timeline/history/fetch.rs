//! History requests and revision-checked page application.

use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::{Context, FollowMode, Task};
use stonks_core::messages::service;

use crate::runtime::spawn_core;

use super::super::reactions;
use super::super::Timeline;
use super::paging;

pub(in super::super) struct HistoryRequest {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for HistoryRequest {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl Timeline {
    pub(in super::super) fn load_older(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more || !self.can_read_history {
            return;
        }
        if let Some(before) = self.oldest_cursor.clone() {
            self.list.pause_following_tail();
            self.pinned_to_bottom = false;
            self.fetch_page(Some(before), cx);
        }
    }

    pub(in super::super) fn fetch_page(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        if self.loading || !self.can_read_history {
            return;
        }
        let Some(channel_id) = self.channel_id.clone() else {
            return;
        };
        let generation = self.generation;
        let nearby = if before.is_none() {
            self.jump_target.clone()
        } else {
            None
        };
        self.loading = true;
        self.reaction_events.clear();
        self.pending_edits.clear();
        self.error = None;
        self.list.remeasure_items(0..1);
        cx.notify();
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let query = stonks_core::messages::models::MessageQuery {
                channel_id: channel_id.clone(),
                before: before.clone(),
                nearby: nearby.clone(),
            };
            let Ok(result) = spawn_core(Abortable::new(
                async move { service::fetch_messages(&policy, &query).await },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation || this.channel_id.as_ref() != Some(&channel_id) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(mut page) => {
                        for message in &mut page.messages {
                            for event in &this.reaction_events {
                                reactions::apply_event(message, event);
                            }
                            if let Some(edit) = this.pending_edits.get(&message.id) {
                                edit.apply(message);
                            }
                        }
                        let merge = paging::merge_page(
                            &mut this.messages,
                            page.messages,
                            &channel_id,
                            before.as_deref(),
                            &this.deleted_messages,
                        );
                        this.has_more = merge.can_page_older(page.has_more, nearby.is_some());
                        if let Some(oldest) = merge.oldest {
                            this.oldest_cursor = Some(oldest);
                        }
                        if before.is_none() {
                            this.list.reset(this.messages.len() + 1);
                            this.list.set_follow_mode(FollowMode::Tail);
                            this.pinned_to_bottom = true;
                            if let Some(target) = nearby.as_ref() {
                                if let Some(index) = this
                                    .messages
                                    .iter()
                                    .position(|message| &message.id == target)
                                {
                                    this.focus_message(target.clone(), index, cx);
                                }
                            }
                        } else if merge.inserted > 0 {
                            // Splicing shifts the logical row anchor, keeping
                            // its pixel offset even with variable-height rows.
                            this.list.splice(1..1, merge.inserted);
                            let next = merge.inserted + 1;
                            if next < this.list.item_count() {
                                this.list.remeasure_items(next..next + 1);
                            }
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                this.pending_edits.clear();
                this.reaction_events.clear();
                this.list.remeasure_items(0..1);
                cx.notify();
            })
            .ok();
        });
        self.request = Some(HistoryRequest { abort, _task: task });
    }
}
