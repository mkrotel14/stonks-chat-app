//! Rendering and interactions for a measured timeline message row.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::messages::models::TimelineMessage;
use stonks_ui::{
    components::{Button, ButtonVariant},
    i18n::{format_day_ms, format_time_ms, t},
    icons::Icon,
    markdown::{self, MentionContext, SpoilerHover},
    Theme,
};

use super::super::message_actions::policy::{MenuAnchor, ToolbarEvent};
use super::super::{
    attachment_url, media_player, message_timestamp, ulid_timestamp_ms, Images, MediaPlayers,
    OpenImage, Reactions, RevealMedia, Timeline, ViewerImage,
};
use super::{author, embeds, layout, navigation, system};

pub(super) struct RowRenderer {
    pub(super) theme: Theme,
    pub(super) users: Vec<(String, String)>,
    pub(super) channels: Vec<(String, String)>,
    pub(super) roles: Vec<(String, String, Option<String>)>,
    pub(super) author_headers: HashMap<String, author::Header>,
    pub(super) on_spoiler_hover: SpoilerHover,
    pub(super) inline_actions: markdown::InlineActions,
    pub(super) inline_media: markdown::InlineMedia,
    pub(super) available_width: f32,
    pub(super) display_embeds: bool,
    pub(super) compact: bool,
    pub(super) reduced_motion: bool,
    pub(super) on_image: OpenImage,
    pub(super) reveal_media: RevealMedia,
}

impl RowRenderer {
    pub(super) fn render(
        &self,
        this: &Timeline,
        item: usize,
        cx: &mut Context<Timeline>,
    ) -> AnyElement {
        let Self {
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
        } = self;
        let (theme, available_width, display_embeds, compact, reduced_motion) = (
            *theme,
            *available_width,
            *display_embeds,
            *compact,
            *reduced_motion,
        );
        let index = item - 1;
        let Some(message) = this.messages.get(index) else {
            return div().into_any_element();
        };
        let mentions = MentionContext {
            users: users,
            channels: channels,
            roles: roles,
        };
        let header = index == 0 || {
            let previous = &this.messages[index - 1];
            !layout::grouped(
                previous.author_id == message.author_id,
                previous.system,
                message.system,
                ulid_timestamp_ms(&message.id)
                    .zip(ulid_timestamp_ms(&previous.id))
                    .map(|(now, before)| now.saturating_sub(before)),
            )
        };
        let avatar = this.images.update(cx, |images, cx| {
            images.get(message.author_avatar_url.as_deref(), 80, cx)
        });
        let author = author_headers
            .get(&message.author_id)
            .cloned()
            .unwrap_or_default();
        let role_icon = this.images.update(cx, |images, cx| {
            images.get(author.role_icon_url.as_deref(), 34, cx)
        });
        let attachment_images = message
            .attachments
            .iter()
            .map(|attachment| {
                if attachment.kind == "Image"
                    && (!attachment.filename.starts_with("SPOILER_")
                        || this
                            .revealed_spoilers
                            .contains(&format!("media-{}-{}", message.id, attachment.id)))
                {
                    this.images.update(cx, |images, cx| {
                        let image = images.get(attachment_url(attachment), 1280, cx);
                        (image, images.failed(attachment_url(attachment), 1280))
                    })
                } else {
                    (None, false)
                }
            })
            .collect::<Vec<_>>();
        let row = message_row(
            MessageRender {
                theme: &theme,
                mentions: &mentions,
                revealed: &this.revealed_spoilers,
                on_spoiler_hover: on_spoiler_hover.clone(),
                inline_actions: inline_actions.clone(),
                inline_media: inline_media.clone(),
                available_width,
                images: this.images.clone(),
                display_embeds,
                compact,
                author,
                role_icon,
                highlighted: (this.navigation.highlighted.as_ref() == Some(&message.id))
                    .then_some(this.navigation.revision),
                reduced_motion,
                on_image: on_image.clone(),
                reactions: this.reactions.clone(),
                add_reaction: this.add_reaction_button(message, theme, cx),
                reveal_media: reveal_media.clone(),
                players: this.players.clone(),
            },
            message,
            header,
            index,
            avatar,
            &attachment_images,
            this.reply_previews(message, theme, cx),
            this.action_ui
                .update(cx, |ui, cx| ui.editor(&message.id, cx)),
            cx,
        );
        let painted_target = cx.entity().downgrade();
        let painted_id = message.id.clone();
        let paint_generation = this.generation;
        let row = row.relative().child(
            canvas(
                move |bounds, _, cx| {
                    painted_target
                        .update(cx, |this, _| {
                            if this.generation == paint_generation {
                                this.painted_rows.insert(painted_id.clone(), bounds);
                            }
                        })
                        .ok();
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        let id = message.id.clone();
        let menu_open = this
            .action_target
            .as_ref()
            .is_some_and(|(target, _)| target == &id)
            && this.action_menu.read(cx).is_open();
        let reaction_open = this.action_ui.read(cx).reacting(&id);
        let toolbar = if reaction_open || menu_open || this.hovered_message.as_ref() == Some(&id) {
            let timeline = cx.entity().downgrade();
            let target = id.clone();
            this.message_actions(message, cx).toolbar(
                &id,
                theme,
                px(0.),
                menu_open,
                reaction_open,
                move |event, window, cx| {
                    timeline
                        .update(cx, |this, cx| match event {
                            ToolbarEvent::Action(action, bounds) => {
                                this.dispatch_action(&target, action, bounds, window, cx)
                            }
                            ToolbarEvent::More(bounds) => this.open_message_menu(
                                &target,
                                MenuAnchor::Toolbar(bounds),
                                window,
                                cx,
                            ),
                        })
                        .ok();
                },
            )
        } else {
            None
        };
        let hover_id = id.clone();
        let row = if message.system {
            row
        } else {
            row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.hovered_message.as_ref() != Some(&hover_id) {
                    this.hovered_message = Some(hover_id.clone());
                    cx.notify();
                } else if !hovered && this.hovered_message.as_ref() == Some(&hover_id) {
                    this.hovered_message = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_message_menu(&id, MenuAnchor::Pointer(event.position), window, cx);
                }),
            )
            .children(toolbar)
        };
        let day = ulid_timestamp_ms(&message.id).map(|ms| format_day_ms(ms as i64));
        let previous_day = index
            .checked_sub(1)
            .and_then(|index| ulid_timestamp_ms(&this.messages[index].id))
            .map(|ms| format_day_ms(ms as i64));
        let row = if day.is_some()
            && day != previous_day
            && (index > 0
                || layout::first_day_divider(this.has_more, this.channel_intro.name.as_deref()))
        {
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.space3))
                        .mx(px(32.))
                        .my(px(20.))
                        .child(div().flex_1().h(px(1.)).bg(theme.outline))
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_muted)
                                .child(day.unwrap_or_default().to_uppercase()),
                        )
                        .child(div().flex_1().h(px(1.)).bg(theme.outline)),
                )
                .child(row)
                .into_any_element()
        } else {
            row.into_any_element()
        };
        if index + 1 == this.messages.len() {
            if let Some(error) = this.mutation_error.as_deref().or(this.error.as_deref()) {
                return div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(row)
                    .child(layout::error(&theme, error))
                    .into_any_element();
            }
        }
        row
    }
}

impl Timeline {
    fn reply_previews(
        &self,
        message: &TimelineMessage,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        message
            .reply_ids
            .iter()
            .filter(|id| stonks_core::messages::normalize::valid_id(id))
            .enumerate()
            .map(|(index, id)| {
                let mut preview = div()
                    .id(ElementId::Name(
                        format!("reply-preview-{}-{index}", message.id).into(),
                    ))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .max_w(px(680.))
                    .min_h(px(layout::REPLY_PREVIEW_HEIGHT))
                    .pr(px(6.))
                    .text_size(px(11.5))
                    .text_color(theme.text)
                    .child(
                        div()
                            .absolute()
                            .left(px(-29.))
                            .bottom(px(4.))
                            .w(px(21.))
                            .h(px(13.))
                            .border_t(px(1.))
                            .border_l(px(1.))
                            .border_color(theme.text_muted)
                            .rounded_tl(px(7.)),
                    );
                if let Some(target) = self.action_message(id) {
                    let avatar = self.images.update(cx, |images, cx| {
                        images.get(target.author_avatar_url.as_deref(), 80, cx)
                    });
                    let excerpt = reply_excerpt(
                        target.content.as_deref(),
                        target
                            .attachments
                            .first()
                            .map(|file| file.filename.as_str()),
                    );
                    preview = preview
                        .child(div().flex_shrink_0().child(stonks_ui::components::avatar(
                            &theme,
                            &target.author_name,
                            16.,
                            avatar,
                        )))
                        .child(
                            div()
                                .min_w_0()
                                .max_w(relative(0.5))
                                .truncate()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.link)
                                .child(target.author_name.clone()),
                        )
                        .children(excerpt.map(|excerpt| div().min_w_0().truncate().child(excerpt)));
                } else {
                    preview = preview.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .italic()
                            .text_color(theme.text_muted)
                            .child("Original message unavailable"),
                    );
                }
                if self.can_read_history {
                    let id = id.clone();
                    preview =
                        preview
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.jump_to(id.clone(), cx);
                            }));
                }
                preview.into_any_element()
            })
            .collect()
    }
}

struct MessageRender<'a> {
    theme: &'a Theme,
    mentions: &'a MentionContext<'a>,
    revealed: &'a HashSet<String>,
    on_spoiler_hover: SpoilerHover,
    inline_actions: markdown::InlineActions,
    inline_media: markdown::InlineMedia,
    available_width: f32,
    images: Entity<Images>,
    display_embeds: bool,
    compact: bool,
    author: author::Header,
    role_icon: Option<Arc<RenderImage>>,
    highlighted: Option<u64>,
    reduced_motion: bool,
    on_image: OpenImage,
    reactions: Entity<Reactions>,
    add_reaction: Option<AnyElement>,
    reveal_media: RevealMedia,
    players: Entity<MediaPlayers>,
}

fn message_row(
    render: MessageRender<'_>,
    message: &TimelineMessage,
    header: bool,
    _index: usize,
    avatar: Option<Arc<RenderImage>>,
    attachment_images: &[(Option<Arc<RenderImage>>, bool)],
    reply_previews: Vec<AnyElement>,
    editor: Option<AnyElement>,
    cx: &mut App,
) -> Stateful<Div> {
    let MessageRender {
        theme,
        mentions,
        revealed,
        on_spoiler_hover,
        inline_actions,
        inline_media,
        available_width,
        images,
        display_embeds,
        compact,
        author,
        role_icon,
        highlighted,
        reduced_motion,
        on_image,
        reactions,
        add_reaction,
        reveal_media,
        players,
    } = render;
    if let Some(event) = system::render(message, theme) {
        return div()
            .id(ElementId::Name(format!("system-{}", message.id).into()))
            .w_full()
            .relative()
            .children(highlighted.map(|revision| {
                navigation::highlight(*theme, &message.id, revision, reduced_motion)
            }))
            .child(event);
    }
    let mut row = div()
        .id(ElementId::Name(format!("message-{}", message.id).into()))
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .pt(px(1.))
        .text_size(px(15.5))
        .children(reply_previews);
    if header {
        row = row.child(author::view(
            *theme,
            message.author_name.clone(),
            author,
            role_icon,
            message_timestamp(message),
        ));
    }
    if let Some(editor) = editor {
        row = row.child(editor);
    } else if let Some(content) = message
        .content
        .clone()
        .filter(|content| !content.is_empty())
    {
        row = row.child(
            div()
                .text_color(if message.system {
                    theme.text_muted
                } else {
                    theme.text
                })
                .when(message.system, |content| content.italic())
                .child(markdown::render_with_options(
                    &content,
                    &message.id,
                    theme,
                    mentions,
                    revealed,
                    on_spoiler_hover,
                    inline_actions,
                    inline_media,
                    markdown::RenderOptions { compact },
                )),
        );
    }
    for (index, attachment) in message.attachments.iter().enumerate() {
        let spoiler_key = format!("media-{}-{}", message.id, attachment.id);
        if attachment.filename.starts_with("SPOILER_") && !revealed.contains(&spoiler_key) {
            let reveal = reveal_media.clone();
            row = row.child(stonks_ui::components::media::concealed_media(
                ElementId::Name(spoiler_key.clone().into()),
                theme,
                move |_, _, cx| reveal(spoiler_key.clone(), cx),
            ));
            continue;
        }
        if attachment.kind == "Image" {
            let viewer_url = attachment
                .original_url
                .as_deref()
                .or(attachment.url.as_deref())
                .map(str::to_owned);
            let preview_url = attachment_url(attachment).map(str::to_owned);
            let image_name = attachment.filename.clone();
            let dims = attachment.width.zip(attachment.height);
            let open = on_image.clone();
            row = row.child(
                stonks_ui::components::media::image_opener(
                    ElementId::Name(format!("open-image-{}-{}", message.id, attachment.id).into()),
                    theme,
                    t("messages.viewImage"),
                )
                .self_start()
                .on_click(move |_, window, cx| {
                    if let Some(url) = &viewer_url {
                        let mut image = ViewerImage::new(url.clone(), image_name.clone());
                        if let Some(preview) = &preview_url {
                            image = image.preview(preview.clone());
                        }
                        if let Some((w, h)) = dims {
                            if let Ok((w, h)) =
                                u32::try_from(w).and_then(|w| u32::try_from(h).map(|h| (w, h)))
                            {
                                image = image.dimensions(w, h);
                            }
                        }
                        open(image, window, cx);
                        cx.stop_propagation();
                    }
                })
                .child(stonks_ui::components::image_attachment(
                    theme,
                    if attachment_images
                        .get(index)
                        .is_some_and(|(_, failed)| *failed)
                    {
                        &attachment.filename
                    } else {
                        ""
                    },
                    attachment_images
                        .get(index)
                        .and_then(|(image, _)| image.clone()),
                    (attachment.width, attachment.height),
                    available_width,
                )),
            );
            if attachment_images
                .get(index)
                .is_some_and(|(_, failed)| *failed)
            {
                let images = images.downgrade();
                let url = attachment_url(attachment).map(str::to_owned);
                row = row
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(theme.font_size_xs))
                            .child(t("messages.imageFailed")),
                    )
                    .child(
                        Button::new(
                            ElementId::Name(format!("retry-image-{}", attachment.id).into()),
                            *theme,
                        )
                        .variant(ButtonVariant::Quiet)
                        .label(t("common.retry"))
                        .on_click(move |_, _, cx| {
                            if let Some(url) = &url {
                                images
                                    .update(cx, |images, cx| images.retry(url, 1280, cx))
                                    .ok();
                            }
                        }),
                    );
            }
            continue;
        }
        if let Some(player) = media_player::render_attachment(
            &players,
            &format!("{}-attachment-{}", message.id, attachment.id),
            attachment,
            available_width,
            cx,
        ) {
            row = row.child(div().mt(px(theme.space2)).child(player));
            continue;
        }
        let url = attachment
            .original_url
            .as_deref()
            .or(attachment.url.as_deref())
            .map(str::to_owned);
        row = row.child(
            Button::new(
                ElementId::Name(format!("download-{}-{}", message.id, attachment.id).into()),
                *theme,
            )
            .variant(ButtonVariant::Quiet)
            .leading(Icon::Download.render(theme.text_muted, 20.))
            .label(attachment.filename.clone())
            .on_click(move |_, window, cx| {
                if let Some(url) = &url {
                    stonks_ui::external_links::request(url, window, cx);
                }
            }),
        );
    }
    let content_url = message.content.as_deref().map(str::trim);
    let has_content_embed = message
        .embeds
        .iter()
        .any(|embed| embed.kind != "None" && embed.url.as_deref() == content_url);
    if display_embeds && !has_content_embed {
        if let Some(media) = embeds::render_content(
            message.content.as_deref(),
            &format!("{}-content", message.id),
            theme,
            available_width,
            &images,
            &players,
            &on_image,
            cx,
        ) {
            row = row.child(media);
        }
    }
    for (index, embed) in message.embeds.iter().enumerate().filter(|_| display_embeds) {
        if let Some(card) = embeds::render(
            embed,
            &format!("{}-embed-{index}", message.id),
            theme,
            available_width,
            &images,
            &on_image,
            cx,
        ) {
            row = row.child(card);
        }
    }
    if let Some(edited) = message.edited_at.as_ref().filter(|_| !message.system) {
        let _ = edited;
        row = row.child(
            div()
                .text_size(px(theme.font_size_xs))
                .text_color(theme.text_muted)
                .child(t("common.edited")),
        );
    }
    if !message.reactions.is_empty() {
        row = row.child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap(px(theme.space1))
                .mt(px(theme.space2))
                .children(message.reactions.iter().enumerate().map(|(index, _)| {
                    Reactions::pill(&reactions, message, index, &images, *theme, cx)
                }))
                .children(add_reaction),
        );
    }

    let time = ulid_timestamp_ms(&message.id)
        .map(|millis| format_time_ms(millis as i64, false))
        .unwrap_or_default();
    layout::row(layout::ReadRow {
        id: message.id.clone(),
        theme: *theme,
        name: message.author_name.clone(),
        avatar,
        header,
        compact,
        time,
        reply_count: message.reply_ids.len(),
        content: row.into_any_element(),
        highlight: highlighted
            .map(|revision| navigation::highlight(*theme, &message.id, revision, reduced_motion)),
    })
}

fn reply_excerpt(content: Option<&str>, attachment: Option<&str>) -> Option<String> {
    if let Some(text) = content.map(str::trim).filter(|text| !text.is_empty()) {
        // Don't expose spoiler text in a plain-text preview.
        return (!text.contains("||")).then(|| {
            text.chars()
                .take(120)
                .collect::<String>()
                .replace(['\r', '\n'], " ")
        });
    }
    Some(
        attachment
            .map(str::to_owned)
            .unwrap_or_else(|| t("messages.attachment")),
    )
}

#[cfg(test)]
mod tests {
    use super::reply_excerpt;

    #[test]
    fn reply_preview_excerpt_is_single_line_bounded_and_spoiler_safe() {
        assert_eq!(
            reply_excerpt(Some(" first\nsecond "), None).as_deref(),
            Some("first second")
        );
        assert_eq!(
            reply_excerpt(Some("é".repeat(121).as_str()), None)
                .unwrap()
                .chars()
                .count(),
            120
        );
        assert_eq!(reply_excerpt(Some("visible ||hidden||"), None), None);
        assert_eq!(
            reply_excerpt(Some(""), Some("file.png")).as_deref(),
            Some("file.png")
        );
    }
}
