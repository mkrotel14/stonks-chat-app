//! Offline source presentation + real variable-height ListState. No account,
//! preferences, gateway or HTTP access. --compact / --light adjust the host.
//! Buttons provide scrolling without wheel/axis automation.
use gpui_kit::{prelude::FluentBuilder, *};
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    theme::CurrentTheme,
    Theme,
};
#[path = "../src/features/conversation/timeline/presentation/author.rs"]
#[allow(dead_code)]
mod author;
#[path = "../src/features/conversation/timeline/presentation/layout.rs"]
#[allow(dead_code)]
mod layout;
#[path = "../src/features/conversation/timeline/presentation/mentioned_user.rs"]
#[allow(dead_code)]
mod mentioned_user;
#[path = "../src/features/conversation/timeline/presentation/navigation.rs"]
#[allow(dead_code)]
mod navigation;
use layout::{ChannelIntro, EmptyState, COMPOSER_INSET, TOP_PADDING};
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Beginning,
    History,
    Empty,
    NoHistory,
    Loading,
    Failed,
}
struct Preview {
    mode: Mode,
    list: ListState,
    pinned: bool,
    description: bool,
    profile: bool,
    profile_focus: FocusHandle,
    compact_mode: bool,
    navigation: navigation::State,
    painted: std::collections::HashMap<usize, Bounds<Pixels>>,
}
impl Preview {
    fn count(&self) -> usize {
        match self.mode {
            Mode::Beginning => 3,
            Mode::History => 80,
            _ => 0,
        }
    }
    fn activate(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.list.reset(self.count() + 1);
        self.list.set_follow_mode(FollowMode::Tail);
        self.pinned = true;
        self.profile = false;
        self.navigation.reset();
        self.painted.clear();
        cx.notify();
    }
    fn latest(&mut self, cx: &mut Context<Self>) {
        self.navigation.stop_scroll();
        self.list.set_follow_mode(FollowMode::Tail);
        self.list.scroll_to_end();
        self.pinned = true;
        cx.notify();
    }
    fn page(&mut self, distance: f32, cx: &mut Context<Self>) {
        self.navigation.stop_scroll();
        let current = if self.list.is_following_tail() {
            -(self.list.max_offset_for_scrollbar().y + px(TOP_PADDING + COMPOSER_INSET))
        } else {
            self.list.scroll_px_offset_for_scrollbar().y
        };
        self.list
            .set_offset_from_scrollbar(point(px(0.), current - px(distance)));
        cx.notify();
    }
    fn focus_message(&mut self, index: usize, cx: &mut Context<Self>) {
        let revision = self.navigation.begin(index.to_string());
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        self.pinned = false;
        self.navigation.clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(navigation::HIGHLIGHT_DURATION)
                .await;
            this.update(cx, |this, cx| {
                if this.navigation.revision == revision {
                    this.navigation.highlighted = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
    fn center_message(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.painted.get(&index).copied() else {
            return;
        };
        let viewport = self.list.viewport_bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        self.navigation.center_pending = None;
        let distance = navigation::center_distance(row, viewport);
        let revision = self.navigation.revision;
        self.navigation.scroll = Some(cx.spawn(async move |this, cx| {
            for frame in 1..=navigation::SCROLL_FRAMES {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                if !this
                    .update(cx, |this, cx| {
                        if this.navigation.revision != revision {
                            return false;
                        }
                        let step = navigation::smooth_offset(0., distance, frame)
                            - navigation::smooth_offset(0., distance, frame - 1);
                        navigation::scroll_center_step(&this.list, step);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        }));
    }
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let count = self.count();
        self.painted.clear();
        if let Some(target) = self
            .navigation
            .center_pending
            .as_deref()
            .and_then(|id| id.parse::<usize>().ok())
        {
            let view = cx.entity().downgrade();
            let revision = self.navigation.revision;
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |_, cx| {
                    view.update(cx, |this, cx| {
                        if this.navigation.revision == revision
                            && this.navigation.center_pending.is_some()
                        {
                            this.center_message(target, cx);
                        }
                    })
                    .ok();
                })
            });
        }
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                view.update(cx, |this, cx| {
                    let pinned = layout::within(
                        layout::distance_to_latest(&this.list, this.count(), COMPOSER_INSET),
                        layout::PINNED_THRESHOLD,
                        this.list.is_following_tail(),
                    );
                    if pinned != this.pinned {
                        this.pinned = pinned;
                        cx.notify();
                    }
                })
                .ok();
            })
        });
        let intro = ChannelIntro {
            name: Some("General".into()),
            description: self.description.then(|| {
                "A channel description replaces the beginning-of-channel fallback.".into()
            }),
        };
        let width = f32::from(window.viewport_size().width);
        let compact_mode = self.compact_mode;
        let highlighted = self.navigation.highlighted.clone();
        let revision = self.navigation.revision;
        let view = cx.entity().downgrade();
        let content = if count > 0 {
            list(self.list.clone(), move |index, _, _| {
                if index == 0 {
                    return layout::intro(&theme, &intro, width).into_any_element();
                }
                let header = index % 3 == 1;
                let copy = if index == count {
                    "LATEST MESSAGE — this line must remain above the floating composer marker."
                        .to_owned()
                } else {
                    format!(
                        "Synthetic history row {index}. {}",
                        "Variable-height content. ".repeat(index % 4 + 1)
                    )
                };
                let body = div()
                    .flex_1()
                    .min_w_0()
                    .pt(px(1.))
                    .when(header, |body| {
                        body.child(author::view(
                            theme,
                            "Offline Reader".into(),
                            author::Header {
                                pronouns: Some("they/them".into()),
                                bot: true,
                                role_icon_url: None,
                            },
                            None,
                            format!("Today at 12:{:02}", index % 60),
                        ))
                    })
                    .child(stonks_ui::markdown::render_with_options(
                        &copy,
                        &format!("fixture-message-{index}"),
                        &theme,
                        &stonks_ui::markdown::MentionContext {
                            users: &[],
                            channels: &[],
                            roles: &[],
                        },
                        &std::collections::HashSet::new(),
                        std::rc::Rc::new(|_, _, _| {}),
                        stonks_ui::markdown::InlineActions::default(),
                        stonks_ui::markdown::InlineMedia::default(),
                        stonks_ui::markdown::RenderOptions {
                            compact: compact_mode,
                        },
                    ));
                let target = view.clone();
                layout::row(layout::ReadRow {
                    id: format!("fixture-{index}"),
                    theme,
                    name: "Offline Reader".into(),
                    avatar: None,
                    header,
                    compact: compact_mode,
                    time: "12:00".into(),
                    content: body.into_any_element(),
                    reply_count: 0,
                    highlight: (highlighted.as_deref() == Some(index.to_string().as_str())).then(
                        || {
                            navigation::highlight(
                                theme,
                                &format!("fixture-{index}"),
                                revision,
                                false,
                            )
                        },
                    ),
                })
                .relative()
                .child(
                    canvas(
                        move |bounds, _, cx| {
                            target
                                .update(cx, |this, _| {
                                    if this.navigation.revision == revision {
                                        this.painted.insert(index, bounds);
                                    }
                                })
                                .ok();
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element()
            })
            .size_full()
            .pt(px(TOP_PADDING))
            .pb(px(COMPOSER_INSET))
            .into_any_element()
        } else {
            let state = match self.mode {
                Mode::Empty => EmptyState::Empty,
                Mode::NoHistory => EmptyState::NoHistory,
                Mode::Loading => EmptyState::Loading,
                Mode::Failed => EmptyState::Failed,
                _ => EmptyState::NoChannel,
            };
            let mut empty = layout::state(
                &theme,
                state,
                Some("Offline synthetic failure; Retry changes only this fixture."),
            );
            if state == EmptyState::Failed {
                empty =
                    empty.child(Button::new("fixture-retry", theme).label("Retry").on_click(
                        cx.listener(|this, _, _, cx| this.activate(Mode::Beginning, cx)),
                    ));
            }
            div()
                .id("fixture-state-scroll")
                .size_full()
                .pt(px(TOP_PADDING))
                .pb(px(COMPOSER_INSET))
                .px(px(24.))
                .overflow_y_scroll()
                .child(empty)
                .into_any_element()
        };
        let profile = self.profile.then(|| {
            let user=serde_json::from_value(serde_json::json!({"_id":"fixture-user","username":"offline_reader","discriminator":"0042","display_name":"Offline Reader","pronouns":"they/them","status":{"text":"Reading synthetic history"},"bot":{"owner":"fixture"}})).expect("synthetic profile");
            mentioned_user::render(mentioned_user::Profile::new(&user,None),None,theme,&self.profile_focus,
                cx.listener(|this,_,_,cx|{this.profile=false;cx.notify();}))
        });
        let channel = div()
            .id("reading-channel")
            .relative()
            .flex_1()
            .min_h_0()
            .mx(px(8.))
            .mb(px(8.))
            .rounded(px(32.))
            .overflow_hidden()
            .bg(theme.surface)
            .child(content)
            .when(layout::jump_visible(count, false, self.pinned), |channel| {
                channel.child(
                    div()
                        .absolute()
                        .right(px(24.))
                        .bottom(px(layout::jump_bottom(COMPOSER_INSET)))
                        .child(
                            Button::new("fixture-jump", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(stonks_ui::i18n::t("messages.jumpLatest"))
                                .trailing(div().child("↓"))
                                .on_click(cx.listener(|this, _, _, cx| this.latest(cx))),
                        ),
                )
            })
            // This labelled test obstruction is deliberately not a second
            // composer implementation; its54px host bounds match the source.
            .child(
                div()
                    .absolute()
                    .left(px(8.))
                    .right(px(8.))
                    .bottom(px(8.))
                    .h(px(54.))
                    .rounded(px(theme.radius_pill))
                    .bg(theme.surface_raised)
                    .occlude()
                    .flex()
                    .items_center()
                    .px(px(20.))
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child("Floating composer overlay marker · 54px"),
            )
            .children(profile)
            .child(
                gpui_kit::base::RoundedFrameCover::uniform(px(32.)).backdrop(theme.canvas.into()),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(fonts::FAMILY)
            .bg(theme.canvas)
            .text_color(theme.text)
            .child(
                div().flex().flex_wrap().gap(px(6.)).p(px(8.)).children(
                    [
                        ("Beginning", Mode::Beginning),
                        ("Long history", Mode::History),
                        ("Empty", Mode::Empty),
                        ("No history access", Mode::NoHistory),
                        ("Loading", Mode::Loading),
                        ("Failure", Mode::Failed),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, mode))| {
                        Button::new(("fixture-mode", index), theme)
                            .variant(ButtonVariant::Secondary)
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.activate(mode, cx)))
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .p(px(8.))
                    .child(
                        Button::new("fixture-page-up", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Read older ↑")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page(-400., cx);
                            })),
                    )
                    .child(
                        Button::new("fixture-page-down", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Read newer ↓")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page(400., cx);
                            })),
                    )
                    .child(
                        Button::new("fixture-top", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Oldest message")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.list.scroll_to(ListOffset::default());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("fixture-description", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Toggle description")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.description = !this.description;
                                this.list.remeasure_items(0..1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("fixture-compact-mode", theme)
                            .variant(ButtonVariant::Quiet)
                            .label(if self.compact_mode {
                                "Compact: on"
                            } else {
                                "Compact: off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.compact_mode = !this.compact_mode;
                                this.list.remeasure();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("fixture-center", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Center message 40")
                            .disabled(count < 40)
                            .on_click(cx.listener(|this, _, _, cx| this.focus_message(40, cx))),
                    )
                    .child(
                        Button::new("fixture-mention-profile", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("Mention profile")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.profile = !this.profile;
                                cx.notify();
                            })),
                    ),
            )
            .child(channel)
    }
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            let light = std::env::args().any(|arg| arg == "--light");
            cx.set_global(CurrentTheme(if light {
                Theme::light()
            } else {
                Theme::dark()
            }));
            let compact = std::env::args().any(|arg| arg == "--compact");
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-reading-preview".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Offline timeline reading states".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if compact {
                            size(px(560.), px(650.))
                        } else {
                            size(px(1000.), px(850.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|cx| {
                        let list = ListState::new(4, ListAlignment::Bottom, px(300.));
                        list.set_follow_mode(FollowMode::Tail);
                        Preview {
                            mode: Mode::Beginning,
                            list,
                            pinned: true,
                            description: false,
                            profile: false,
                            profile_focus: cx.focus_handle(),
                            compact_mode: std::env::args().any(|arg| arg == "--compact-mode"),
                            navigation: Default::default(),
                            painted: std::collections::HashMap::new(),
                        }
                    })
                },
            )
            .expect("offline reading fixture");
            cx.activate(true);
        });
}
