//! Timeline list layout, scroll positioning, and viewport rendering.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_ui::{
    components::{Button, ButtonVariant},
    i18n::t,
    markdown::{self, SpoilerHover},
    Theme,
};

use super::super::{attachment_url, OpenImage, RevealMedia, Timeline, TimelineEvent};
use super::row::RowRenderer;
use super::{author, embeds, layout, media, navigation};

impl Timeline {
    pub fn set_insets(&mut self, insets: f32, _cx: &mut Context<Self>) {
        if self.insets != insets {
            self.insets = insets;
            self.list.remeasure();
        }
    }
    pub(in super::super) fn distance_to_latest(&self) -> Option<f32> {
        layout::distance_to_latest(&self.list, self.messages.len(), self.composer_inset)
    }
    pub(in super::super) fn focus_message(
        &mut self,
        id: String,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let revision = self.navigation.begin(id);
        let generation = self.generation;
        self.list.pause_following_tail();
        self.list.scroll_to(ListOffset {
            item_ix: index + 1,
            offset_in_item: px(0.),
        });
        self.pinned_to_bottom = false;
        self.navigation.clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(navigation::HIGHLIGHT_DURATION)
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation && this.navigation.revision == revision {
                    this.navigation.highlighted = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
    fn center_message(&mut self, id: &str, reduced_motion: bool, cx: &mut Context<Self>) {
        let Some(row) = self.painted_rows.get(id).copied() else {
            return;
        };
        let viewport = self.list.viewport_bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        self.navigation.center_pending = None;
        let distance = navigation::center_distance(row, viewport);
        if reduced_motion || distance.abs() < 0.5 {
            navigation::scroll_center_step(&self.list, distance);
            cx.notify();
            return;
        }
        let generation = self.generation;
        let revision = self.navigation.revision;
        self.navigation.scroll = Some(cx.spawn(async move |this, cx| {
            let mut previous = 0.;
            for frame in 1..=navigation::SCROLL_FRAMES {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.generation != generation || this.navigation.revision != revision {
                            return false;
                        }
                        let next = navigation::smooth_offset(0., distance, frame);
                        navigation::scroll_center_step(&this.list, next - previous);
                        previous = next;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }
    pub fn set_composer_height(&mut self, height: f32, bottom_inset: f32, cx: &mut Context<Self>) {
        let inset = layout::COMPOSER_INSET + (height - 52.).max(0.) + bottom_inset;
        if (self.composer_inset - inset).abs() >= 0.5 {
            self.composer_inset = inset;
            cx.notify();
        }
    }

    pub(crate) fn can_jump_to_latest(&self) -> bool {
        self.jump_target.is_some() || !self.pinned_to_bottom
    }

    pub fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.navigation.stop_scroll();
        if self.jump_target.is_some() {
            let channel = self.channel_id.clone();
            self.reset(cx);
            if let Some(channel) = channel {
                self.activate_channel(channel, cx);
                self.fetch_page(None, cx);
            }
        } else {
            self.list.set_follow_mode(FollowMode::Tail);
            self.list.scroll_to_end();
            self.pinned_to_bottom = true;
            cx.notify();
        }
    }
    fn older_control(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        div().flex().justify_center().mb(px(theme.space4)).child(
            Button::new("history-older", *theme)
                .variant(ButtonVariant::Quiet)
                .disabled(self.loading)
                .label(t(if self.loading {
                    "common.loading"
                } else {
                    "timeline.loadOlder"
                }))
                .on_click(cx.listener(|this, _, _, cx| this.load_older(cx))),
        )
    }
    fn history_status(&self, theme: &Theme, viewport_width: f32, cx: &mut Context<Self>) -> Div {
        if self.has_more {
            return self.older_control(theme, cx);
        }
        if layout::show_intro(
            self.loading,
            self.has_more,
            self.messages.len(),
            self.channel_intro.name.as_deref(),
        ) {
            return layout::intro(theme, &self.channel_intro, viewport_width);
        }
        let status = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(theme.space1))
            .p(px(theme.space2))
            .text_size(px(theme.font_size_xs))
            .text_color(theme.text_muted);
        if self.loading {
            status.child(t("timeline.loadingOlder"))
        } else if let Some(error) = &self.error {
            status.child(error.clone()).child(
                Button::new("history-retry", *theme)
                    .variant(ButtonVariant::Quiet)
                    .label(t("common.retry"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.oldest_cursor.is_none() {
                            this.fetch_page(None, cx);
                        } else {
                            this.load_older(cx);
                        }
                    })),
            )
        } else {
            div().h(px(0.))
        }
    }
}

impl Render for Timeline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.painted_rows.clear();
        let theme = Theme::current(cx);
        let display_embeds = cx
            .global::<crate::preferences::NativePreferences>()
            .saved
            .display_embeds;
        let compact = cx
            .global::<crate::preferences::NativePreferences>()
            .saved
            .compact_mode;
        let reduced_motion = cx
            .global::<crate::preferences::NativePreferences>()
            .saved
            .reduced_motion;
        if let Some(target) = self.navigation.center_pending.clone() {
            let view = cx.entity().downgrade();
            let generation = self.generation;
            let revision = self.navigation.revision;
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |_, cx| {
                    view.update(cx, |this, cx| {
                        if this.generation == generation
                            && this.navigation.revision == revision
                            && this.navigation.center_pending.as_ref() == Some(&target)
                        {
                            this.center_message(&target, reduced_motion, cx);
                        }
                    })
                    .ok();
                })
            });
        }
        let timeline = cx.entity().downgrade();
        let generation = self.generation;
        // Frame callbacks run before painting. A second callback sees the
        // completed list layout, including its actual last-row geometry.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                timeline
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        let pinned = layout::within(
                            this.distance_to_latest(),
                            layout::PINNED_THRESHOLD,
                            this.list.is_following_tail(),
                        );
                        if pinned != this.pinned_to_bottom {
                            this.pinned_to_bottom = pinned;
                            cx.notify();
                        }
                        let ui = this.action_ui.read(cx);
                        if let Some((id, action)) = ui.target().zip(ui.action()) {
                            if !this.action_message(id).is_some_and(|message| {
                                this.message_actions(message, cx).contains(action)
                            }) {
                                this.clear_action_ui(window, cx);
                            }
                        }
                        let foreground = window.is_window_active();
                        let latest =
                            layout::newest_visible(&this.painted_rows, this.list.viewport_bounds())
                                .map(str::to_owned);
                        this.session.update(cx, |session, cx| {
                            session.observe_read_view(
                                this.channel_id.as_deref(),
                                latest.as_deref(),
                                foreground,
                                cx,
                            )
                        });
                    })
                    .ok();
            })
        });
        // Keep image ownership near the viewport as the history grows.
        let anchor = self.list.logical_scroll_top().item_ix.saturating_sub(1);
        let autumn = self
            .session
            .read(cx)
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.clone());
        let author_headers = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| {
                self.messages
                    .iter()
                    .map(|message| message.author_id.as_str())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .map(|id| {
                        (
                            id.to_owned(),
                            author::header(
                                snapshot,
                                self.channel_id.as_deref().unwrap_or_default(),
                                id,
                                autumn.as_deref(),
                            ),
                        )
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        if author_headers != self.author_headers {
            self.author_headers = author_headers.clone();
            self.list.remeasure_items(1..self.list.item_count());
        }
        let mut sources = self
            .messages
            .iter()
            .skip(anchor.saturating_sub(64))
            .take(192)
            .flat_map(|message| {
                message
                    .author_avatar_url
                    .iter()
                    .cloned()
                    .map(|url| (url, 80))
                    .chain(
                        message
                            .attachments
                            .iter()
                            .filter(|attachment| attachment.kind == "Image")
                            .filter_map(|attachment| attachment_url(attachment).map(str::to_owned))
                            .map(|url| (url, 1280)),
                    )
                    .chain(
                        message
                            .reactions
                            .iter()
                            .filter(|reaction| {
                                stonks_core::messages::normalize::valid_id(&reaction.emoji)
                            })
                            .filter_map(|reaction| {
                                stonks_core::media::file_url(
                                    autumn.as_deref(),
                                    "emojis",
                                    &reaction.emoji,
                                )
                            })
                            .map(|url| (url, 40)),
                    )
                    .chain(
                        message
                            .content
                            .as_deref()
                            .into_iter()
                            .flat_map(|content| {
                                markdown::custom_emoji_requests(
                                    content,
                                    &message.id,
                                    &self.revealed_spoilers,
                                )
                            })
                            .filter_map(|request| {
                                self.reactions
                                    .read(cx)
                                    .resolve_custom(request.id.as_deref(), request.name.as_deref())
                            })
                            .filter_map(|(id, _)| {
                                stonks_core::media::file_url(autumn.as_deref(), "emojis", &id)
                            })
                            .map(|url| (url, 64)),
                    )
                    .chain(
                        self.reactions.read(cx).avatar_sources(
                            message
                                .reactions
                                .iter()
                                .flat_map(|reaction| reaction.user_ids.iter().cloned()),
                            cx,
                        ),
                    )
                    .chain(
                        message
                            .embeds
                            .iter()
                            .filter(|_| display_embeds)
                            .flat_map(embeds::sources),
                    )
                    .chain(if display_embeds {
                        embeds::content_sources(message.content.as_deref())
                    } else {
                        Vec::new()
                    })
            })
            .collect::<Vec<_>>();
        sources.extend(
            author_headers
                .values()
                .filter_map(|header| header.role_icon_url.clone())
                .map(|url| (url, 34)),
        );
        self.images
            .update(cx, |images, _| images.set_sources("timeline", sources));
        let available_width = (f32::from(window.viewport_size().width) - self.insets).max(1.);
        let active_keys =
            media::playable_keys(&self.messages, display_embeds, &self.revealed_spoilers);
        self.players
            .update(cx, |players, cx| players.retain_keys(&active_keys, cx));
        let mentioned_user = self.mentioned_user_view(theme, cx);
        if self.messages.is_empty() {
            let state = layout::empty_state(
                self.channel_id.is_some(),
                self.loading,
                self.error.is_some(),
                self.can_read_history,
            );
            let mut empty = layout::state(&theme, state, self.error.as_deref());
            if state == layout::EmptyState::Failed {
                empty = empty.child(
                    Button::new("history-retry-initial", theme)
                        .label(t("common.retry"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.oldest_cursor.is_some() {
                                this.load_older(cx);
                            } else {
                                this.fetch_page(None, cx);
                            }
                        })),
                );
            }
            let empty = div()
                .id("timeline-empty")
                .size_full()
                .pt(px(layout::TOP_PADDING))
                .pb(px(self.composer_inset))
                .px(px(24.))
                .overflow_y_scroll()
                .when(
                    self.can_read_history && self.has_more && !self.loading && self.error.is_none(),
                    |view| view.child(self.older_control(&theme, cx)),
                )
                .child(empty);
            return div()
                .relative()
                .size_full()
                .child(empty)
                .children(mentioned_user)
                .into_any_element();
        }

        // GPUI's measured list handles variable heights and preserves its
        // logical anchor on prepend; uniform_list cannot represent these rows.
        let mut users = Vec::new();
        let mut channels = Vec::new();
        let mut roles = Vec::new();
        if let Some(snapshot) = self.session.read(cx).snapshot.as_ref() {
            users = snapshot
                .users
                .iter()
                .chain(std::iter::once(&snapshot.current_user))
                .map(|user| {
                    (
                        user.id.clone(),
                        user.display_name
                            .clone()
                            .unwrap_or_else(|| user.username.clone()),
                    )
                })
                .collect();
            channels = snapshot
                .channels
                .iter()
                .map(|channel| {
                    (
                        channel.id.clone(),
                        channel.name.clone().unwrap_or_else(|| {
                            if channel.channel_type == "SavedMessages" {
                                "saved-notes".into()
                            } else {
                                "conversation".into()
                            }
                        }),
                    )
                })
                .collect();
            let server_id = snapshot
                .channels
                .iter()
                .find(|channel| Some(&channel.id) == self.channel_id.as_ref())
                .and_then(|channel| channel.server.as_ref());
            if let Some(server) = snapshot
                .servers
                .iter()
                .find(|server| Some(&server.id) == server_id)
            {
                roles = server
                    .roles
                    .iter()
                    .map(|(id, role)| (id.clone(), role.name.clone(), role.colour.clone()))
                    .collect();
            }
        }
        // History can contain authors absent from the gateway snapshot.
        for message in &self.messages {
            if !users.iter().any(|(id, _)| id == &message.author_id) {
                users.push((message.author_id.clone(), message.author_name.clone()));
            }
        }
        let timeline = cx.entity().downgrade();
        let on_spoiler_hover: SpoilerHover = Rc::new(move |key, hovered, cx| {
            timeline
                .update(cx, |this, cx| {
                    let changed = if hovered {
                        this.revealed_spoilers.insert(key.to_owned())
                    } else {
                        this.revealed_spoilers.remove(key)
                    };
                    if changed {
                        cx.notify();
                    }
                })
                .ok();
        });
        let target = cx.entity().downgrade();
        let reveal_media: RevealMedia = Rc::new(move |key, cx| {
            target
                .update(cx, |this, cx| {
                    this.revealed_spoilers.insert(key);
                    this.list.remeasure_items(1..this.list.item_count());
                    cx.notify();
                })
                .ok();
        });
        let target = cx.entity().downgrade();
        let on_image: OpenImage = Rc::new(move |image, _, cx| {
            target
                .update(cx, |_, cx| cx.emit(TimelineEvent::OpenImage(image)))
                .ok();
        });
        let timeline = cx.entity().downgrade();
        let user_target = timeline.clone();
        let channel_target = timeline.clone();
        let inline_actions = markdown::InlineActions {
            user: Some(Rc::new(move |id, window, cx| {
                user_target
                    .update(cx, |this, cx| this.open_mentioned_user(id, window, cx))
                    .ok();
            })),
            channel: Some(Rc::new(move |id, _, cx| {
                channel_target
                    .update(cx, |this, cx| {
                        if this.session.read(cx).can_view_channel(id) {
                            cx.emit(TimelineEvent::NavigateChannel(id.to_owned()));
                        }
                    })
                    .ok();
            })),
        };
        let reaction_images = self.images.clone();
        let reaction_metadata = self.reactions.clone();
        let inline_autumn = autumn.clone();
        let inline_media = markdown::InlineMedia {
            custom: Some(Rc::new(move |request, visible, cx| {
                let (id, name) = reaction_metadata
                    .read(cx)
                    .resolve_custom(request.id.as_deref(), request.name.as_deref())?;
                let url = stonks_core::media::file_url(inline_autumn.as_deref(), "emojis", &id)?;
                let image = if visible {
                    reaction_images.update(cx, |images, cx| images.get(Some(&url), 64, cx))
                } else {
                    None
                };
                Some(markdown::InlineEmoji {
                    label: format!(":{name}:"),
                    image: image.map(ImageSource::from),
                })
            })),
        };
        let renderer = RowRenderer {
            theme,
            users,
            channels,
            roles,
            author_headers,
            on_spoiler_hover,
            inline_actions,
            inline_media,
            available_width,
            display_embeds,
            compact,
            reduced_motion,
            on_image,
            reveal_media,
        };
        let rows = list(self.list.clone(), move |item, window, cx| {
            timeline
                .update(cx, |this, cx| {
                    if item == 0 {
                        return this
                            .history_status(&theme, f32::from(window.viewport_size().width), cx)
                            .into_any_element();
                    }
                    renderer.render(this, item, cx)
                })
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .pt(px(layout::TOP_PADDING))
        .pb(px(self.composer_inset));
        div()
            .size_full()
            .relative()
            .child(rows)
            .when(
                layout::jump_visible(
                    self.messages.len(),
                    self.jump_target.is_some(),
                    self.pinned_to_bottom,
                ),
                |root| {
                    root.child(
                        div()
                            .absolute()
                            .right(px(24.))
                            .bottom(px(layout::jump_bottom(self.composer_inset)))
                            // Keep clicks off the rows behind the button without blocking list scroll.
                            .block_mouse_except_scroll()
                            .child(
                                Button::new("timeline-jump-latest", theme)
                                    .variant(ButtonVariant::Secondary)
                                    .label(t("messages.jumpLatest"))
                                    .trailing(div().child("↓"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.jump_to_latest(cx)),
                                    ),
                            ),
                    )
                },
            )
            .children(mentioned_user)
            .into_any_element()
    }
}
