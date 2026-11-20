//! Composer-owned picker lifecycle and source custom-emoji permission scope.
pub(super) mod tokens;

use super::{Composer, Request};
use crate::runtime::spawn_core;
use crate::{
    session::{Session, SessionEvent},
    shared::images::Images,
};
use futures_util::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use gpui_kit::*;
use std::ops::Range;
use stonks_ui::{
    components::{
        Button, ButtonVariant, CustomEmoji, EmojiPickerEvent, UiCloseButton, UiEmojiPicker,
    },
    i18n::t,
    icons::Icon,
    Theme,
};

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    deployment: String,
    user: String,
    servers: Vec<(String, String)>,
    autumn: Option<String>,
}
pub(super) struct EmojiState {
    pub picker: Entity<UiEmojiPicker>,
    pub open: bool,
    trigger_focus: FocusHandle,
    pub custom: Vec<CustomEmoji>,
    selection: Option<Range<usize>>,
    context: Option<(Entity<Session>, Entity<Images>)>,
    scope: Option<Scope>,
    generation: u64,
    request: Option<Request>,
    _subscriptions: Vec<Subscription>,
}
impl EmojiState {
    pub fn new(picker: Entity<UiEmojiPicker>, trigger_focus: FocusHandle) -> Self {
        Self {
            picker,
            open: false,
            trigger_focus,
            custom: Vec::new(),
            selection: None,
            context: None,
            scope: None,
            generation: 0,
            request: None,
            _subscriptions: Vec::new(),
        }
    }
}
impl Composer {
    pub fn set_emoji_context(
        &mut self,
        session: Entity<Session>,
        images: Entity<Images>,
        cx: &mut Context<Self>,
    ) {
        let image_source = images.clone();
        self.emoji.picker.update(cx, |picker, _| {
            picker.set_image_provider(move |url, cx| {
                image_source.update(cx, |images, cx| images.get(Some(url), 48, cx))
            })
        });
        self.emoji._subscriptions = vec![
            cx.observe(&session, |this, _, cx| this.refresh_emojis(cx)),
            cx.subscribe(&session, |this, _, event, cx| {
                let SessionEvent::Gateway(event) = event;
                if matches!(
                    event.event_type.as_str(),
                    "EmojiCreate" | "EmojiUpdate" | "EmojiDelete"
                ) {
                    this.emoji.scope = None;
                    this.refresh_emojis(cx);
                }
            }),
            cx.observe(&images, |this, _, cx| {
                if this.mentions.open {
                    cx.notify();
                }
                if this.emoji.open {
                    this.emoji
                        .picker
                        .update(cx, |picker, cx| picker.refresh_images(cx));
                }
            }),
        ];
        self.emoji.context = Some((session, images));
        self.refresh_emojis(cx);
    }
    pub(super) fn mention_image(
        &self,
        url: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<std::sync::Arc<RenderImage>> {
        let (_, images) = self.emoji.context.as_ref()?;
        images.update(cx, |images, cx| images.get(url, 30, cx))
    }

    /// Server settings CRUD changes metadata without changing channel/session
    /// identity. The host routes its local emoji-change notification here.
    pub fn invalidate_emojis(&mut self, cx: &mut Context<Self>) {
        self.emoji.scope = None;
        self.refresh_emojis(cx);
    }

    pub(super) fn reset_emojis(&mut self, cx: &mut Context<Self>) {
        self.emoji.request = None;
        self.emoji.scope = None;
        self.emoji.generation = self.emoji.generation.wrapping_add(1);
        self.emoji.open = false;
        self.emoji.selection = None;
        self.emoji.custom.clear();
        self.emoji
            .picker
            .update(cx, |picker, cx| picker.set_custom(Vec::new(), cx));
        if let Some((_, images)) = &self.emoji.context {
            images.update(cx, |images, _| {
                images.set_sources("composer-emoji", Vec::new())
            });
        }
    }
    pub(super) fn refresh_emojis(&mut self, cx: &mut Context<Self>) {
        let scope = self.emoji.context.as_ref().and_then(|(session, _)| {
            let session = session.read(cx);
            let snapshot = session.snapshot.as_ref()?;
            let deployment = session.server.as_ref()?;
            let channel = self.state.active_channel()?;
            let server = snapshot
                .channels
                .iter()
                .find(|entry| entry.id == channel)
                .and_then(|channel| channel.server.as_deref());
            let external = session.can_channel_permission(channel, 1 << 41);
            let mut ids = Vec::new();
            if let Some(server) = server {
                ids.push(server.to_owned());
            }
            if server.is_none() || external {
                for server in &snapshot.servers {
                    if !ids.contains(&server.id) {
                        ids.push(server.id.clone());
                    }
                }
            }
            Some(Scope {
                deployment: deployment.deployment_url.clone(),
                user: snapshot.current_user.id.clone(),
                servers: ids
                    .into_iter()
                    .map(|id| {
                        let name = snapshot
                            .servers
                            .iter()
                            .find(|server| server.id == id)
                            .map_or_else(String::new, |server| server.name.clone());
                        (id, name)
                    })
                    .collect(),
                autumn: deployment.endpoints.autumn.clone(),
            })
        });
        if self.emoji.scope == scope {
            return;
        }
        self.reset_emojis(cx);
        self.emoji.scope = scope.clone();
        let Some(scope) = scope else {
            cx.notify();
            return;
        };
        let policy = self.policy.clone();
        let generation = self.emoji.generation;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let Ok(custom) = spawn_core(Abortable::new(
                async move {
                    let mut groups =
                        futures_util::stream::iter(scope.servers.into_iter().enumerate().map(
                            |(index, (id, name))| {
                                let policy = policy.clone();
                                let autumn = scope.autumn.clone();
                                async move {
                                    let emojis =
                                        stonks_core::server_settings::fetch_emojis(&policy, &id)
                                            .await
                                            .unwrap_or_default();
                                    (
                                        index,
                                        emojis
                                            .into_iter()
                                            .map(|emoji| {
                                                let url = autumn.as_ref().map(|base| {
                                                    format!(
                                                        "{}/emojis/{}",
                                                        base.trim_end_matches('/'),
                                                        emoji.id
                                                    )
                                                });
                                                CustomEmoji {
                                                    id: emoji.id,
                                                    name: emoji.name,
                                                    server_name: (!name.is_empty())
                                                        .then(|| name.clone()),
                                                    url,
                                                }
                                            })
                                            .collect::<Vec<_>>(),
                                    )
                                }
                            },
                        ))
                        .buffer_unordered(4)
                        .collect::<Vec<_>>()
                        .await;
                    groups.sort_by_key(|(index, _)| *index);
                    groups
                        .into_iter()
                        .flat_map(|(_, emojis)| emojis)
                        .collect::<Vec<_>>()
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.emoji.generation != generation {
                    return;
                }
                this.emoji.custom = custom.clone();
                this.emoji
                    .picker
                    .update(cx, |picker, cx| picker.set_custom(custom, cx));
                this.update_emoji_images(cx);
                cx.notify();
            })
            .ok();
        });
        self.emoji.request = Some(Request { abort, _task: task });
        cx.notify();
    }
    pub(super) fn close_emoji_picker(&mut self, cx: &mut Context<Self>) {
        self.emoji.open = false;
        self.emoji.selection = None;
        self.update_emoji_images(cx);
    }
    fn update_emoji_images(&self, cx: &mut Context<Self>) {
        if let Some((_, images)) = &self.emoji.context {
            let sources = if self.emoji.open {
                self.emoji
                    .custom
                    .iter()
                    .filter_map(|emoji| emoji.url.as_ref().map(|url| (url.clone(), 48)))
                    .collect()
            } else {
                Vec::new()
            };
            images.update(cx, |images, _| {
                images.set_sources("composer-emoji", sources)
            });
        }
    }
    pub(super) fn select_emoji(
        &mut self,
        event: &EmojiPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_channel().is_none() || self.state.sending_here() || !self.emoji.open {
            return;
        }
        let EmojiPickerEvent::Select { value, custom } = event;
        let text = if *custom {
            let Some(emoji) = self.emoji.custom.iter().find(|emoji| &emoji.id == value) else {
                return;
            };
            tokens::display_token(emoji, &self.emoji.custom)
        } else {
            value.clone()
        };
        let selection = self.emoji.selection.take();
        self.emoji.open = false;
        self.input.update(cx, |input, cx| {
            if let Some(selection) = selection {
                input.set_selected_range(selection, cx);
            }
            // replace is the installed input's undoable selection replacement,
            // unlike insert (which only inserts at the active caret).
            input.focus(window, cx);
        });
        self.insert_text(text, window, cx);
        self.update_emoji_images(cx);
        cx.notify();
    }
    pub(super) fn emoji_button(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let disabled = self.state.active_channel().is_none() || self.state.sending_here();
        self.emoji
            .picker
            .update(cx, |picker, cx| picker.set_disabled(disabled, cx));
        let picker = self.emoji.picker.clone();
        let composer = cx.entity().downgrade();
        let trigger = Button::new("composer-emoji", theme)
            .variant(ButtonVariant::Icon)
            .tooltip(t("messages.emojiTab"))
            .track_focus(&self.emoji.trigger_focus)
            .expanded(self.emoji.open)
            .selected(self.emoji.open)
            .disabled(disabled)
            .leading(Icon::Smile.render_stroke(theme.text, 22., 1.7));
        if disabled {
            return trigger.into_any_element();
        }
        let focus = picker.read(cx).search_focus(cx);
        let trigger_focus = self.emoji.trigger_focus.clone();
        let height = (f32::from(window.viewport_size().height) - 180.).clamp(1., 520.);
        let width = (f32::from(window.viewport_size().width) - 46.).clamp(1., 390.);
        gpui_kit::base::Popover::new("composer-emoji-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .open(self.emoji.open)
            .track_focus(&focus)
            .trigger_with(move |_, _, _| {
                // HTML buttons focus on pointer down before a popover captures
                // the previous focus. Preserve that source lifecycle explicitly.
                div()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        trigger_focus.focus(window, cx)
                    })
                    .child(trigger)
                    .into_any_element()
            })
            .on_open_change(cx.listener(|this, open: &bool, window, cx| {
                if *open {
                    this.emoji.selection = Some(this.input.read(cx).selected_range());
                    this.emoji
                        .picker
                        .update(cx, |picker, cx| picker.reset(window, cx));
                }
                this.emoji.open = *open;
                this.update_emoji_images(cx);
                cx.notify();
            }))
            .content(move |_, _, _| {
                div()
                    .w(px(width))
                    .max_h(px(height))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.outline)
                    .rounded(px(theme.radius_lg))
                    .bg(theme.surface_raised)
                    .text_color(theme.text)
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_md.color.into(),
                        offset: point(px(0.), px(theme.shadow_md.dy)),
                        blur_radius: px(theme.shadow_md.blur),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pt(px(9.))
                            .px(px(10.))
                            .pb(px(7.))
                            .border_b_1()
                            .border_color(theme.outline)
                            .child(
                                Button::new("composer-emoji-tab", theme)
                                    .variant(ButtonVariant::Quiet)
                                    .selected(true)
                                    .label(t("messages.emojiTab")),
                            )
                            .child(UiCloseButton::new("composer-picker-close", theme).on_click(
                                move |_, window, cx| {
                                    composer
                                        .update(cx, |this, cx| {
                                            this.emoji.open = false;
                                            this.emoji.trigger_focus.focus(window, cx);
                                            this.update_emoji_images(cx);
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("composer-emoji-pane")
                            .min_h_0()
                            .min_h(px(260.))
                            .overflow_y_scroll()
                            .p(px(theme.space3))
                            .child(picker),
                    )
            })
            .into_any_element()
    }
}
