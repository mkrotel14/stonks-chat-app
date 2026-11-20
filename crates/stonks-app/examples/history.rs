//! Rendered prepend/append regression check with variable-height rows.
//! `cargo run --manifest-path crates/stonks-app/Cargo.toml --example history`
//! Writes /tmp/stonks-history-check.txt after checking actual painted bounds.
use gpui_kit::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use stonks_core::messages::{models::TimelineMessage, normalize::normalize_message};
use stonks_ui::{fonts, Theme};

#[path = "../src/features/conversation/timeline/history/paging.rs"]
mod paging;

fn message(n: usize) -> TimelineMessage {
    normalize_message(&serde_json::json!({"_id": format!("{n:026}"), "channel": "channel", "author": "author", "content": format!("Message {n}")}), &HashMap::new(), &HashMap::new(), None).unwrap()
}

struct Preview {
    messages: Vec<TimelineMessage>,
    list: ListState,
    phase: usize,
    anchor_top: Pixels,
    settling: bool,
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.phase < 4 {
            let view = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                view.update(cx, |this, cx| {
                    // Frame callbacks run before drawing. Allow the mutation
                    // from the preceding callback to receive a layout pass.
                    if this.settling {
                        this.settling = false;
                        cx.notify();
                        return;
                    }
                    match this.phase {
                        0 => this.list.scroll_to(ListOffset { item_ix: 1, offset_in_item: px(13.) }),
                        1 => {
                            this.anchor_top = this.list.bounds_for_item(1).expect("oldest row is visible").top();
                            let before = this.messages[0].id.clone();
                            let merge = paging::merge_page(&mut this.messages, (0..50).map(message).rev().collect(), "channel", Some(&before), &HashSet::new());
                            assert_eq!(merge.inserted, 50);
                            assert_eq!(merge.oldest, Some(format!("{:026}", 0)));
                            this.list.splice(1..1, merge.inserted);
                            this.list.remeasure_items(51..52);
                        }
                        2 => {
                            let top = this.list.bounds_for_item(51).expect("original row stays visible").top();
                            assert!((top - this.anchor_top).abs() < px(1.), "prepend moved original row: {top:?} vs {:?}", this.anchor_top);
                            assert!(!this.list.is_following_tail());
                            let end = this.messages.len() + 1;
                                let mut live = message(100);
                                paging::apply_edit(&mut live, &serde_json::json!({"content": "New live message"}));
                                this.messages.push(live);
                            this.list.splice(end..end, 1);
                        }
                        3 => {
                            let top = this.list.bounds_for_item(51).expect("original row stays visible after live append").top();
                            assert!((top - this.anchor_top).abs() < px(1.));
                            std::fs::write("/tmp/stonks-history-check.txt", "PASS: 50 variable-height rows prepended; original row pixel position preserved; live append did not jump to bottom.\n").unwrap();
                        }
                        _ => unreachable!(),
                    }
                    this.phase += 1;
                    this.settling = true;
                    cx.notify();
                }).ok();
            });
        }
        let theme = Theme::dark();
        let messages = Arc::new(self.messages.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .text_color(theme.text)
            .font_family(fonts::FAMILY)
            .child(div().p(px(theme.space3)).child(if self.phase >= 4 {
                "PASS — row position preserved across prepend and live append"
            } else {
                "Checking variable-height history paging…"
            }))
            .child(
                list(self.list.clone(), move |index, _, _| {
                    if index == 0 {
                        return div()
                            .h(px(34.))
                            .child("Beginning of history")
                            .into_any_element();
                    }
                    let message = &messages[index - 1];
                    let n = message.id.parse::<usize>().unwrap();
                    div()
                        .h(px(40. + (n % 5) as f32 * 35.))
                        .w_full()
                        .p(px(theme.space3))
                        .border_b_1()
                        .border_color(theme.outline)
                        .child(message.content.clone().unwrap())
                        .into_any_element()
                })
                .flex_1()
                .min_h_0()
                .w_full(),
            )
    }
}
fn main() {
    gpui_kit::application().run(|cx| {
        fonts::register(cx);
        cx.open_window(
            WindowOptions {
                app_id: Some("stonks-history-preview".into()),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| {
                    let list = ListState::new(51, ListAlignment::Bottom, px(300.));
                    list.set_follow_mode(FollowMode::Tail);
                    Preview {
                        messages: (50..100).map(message).collect(),
                        list,
                        phase: 0,
                        anchor_top: px(0.),
                        settling: false,
                    }
                })
            },
        )
        .expect("history preview window");
        cx.activate(true);
    });
}
