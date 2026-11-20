use super::{actions::Mutation, extra::Dialog, *};
use stonks_core::uploads::UploadFile;
use stonks_ui::i18n::t_vars;
pub(super) struct EmojiState {
    pub emoji_name: Entity<InputState>,
    pub rename_emoji: Entity<InputState>,
    pub emoji_file: Option<UploadFile>,
    pub emoji_preview: Option<Arc<RenderImage>>,
    pub preview_task: Option<Task<()>>,
}

impl ServerSettings {
    pub(super) fn emoji_creator(&self, emoji: &ServerEmoji) -> String {
        emoji
            .creator_id
            .as_ref()
            .map(|id| {
                t_vars(
                    "serverSettings.createdBy",
                    &[("name", self.extra.users.get(id).unwrap_or(id))],
                )
            })
            .unwrap_or_else(|| t("serverSettings.customEmoji"))
    }
    pub(super) fn emoji_image(
        &self,
        emoji: &ServerEmoji,
        size: f32,
        theme: Theme,
        cx: &mut App,
    ) -> AnyElement {
        let url = media::file_url(self.autumn.as_deref(), "emojis", &emoji.id);
        let image = self
            .images
            .update(cx, |images, cx| images.get(url.as_deref(), size as u32, cx));
        div()
            .size(px(size))
            .max_w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(match image {
                Some(image) => img(image)
                    .id(ElementId::Name(
                        format!("settings-emoji-{}-{size}", emoji.id).into(),
                    ))
                    .size(px(size))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
                    .text_color(theme.text_muted)
                    .child(":")
                    .into_any_element(),
            })
            .into_any_element()
    }
    pub(super) fn render_emojis(
        &self,
        theme: Theme,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = div()
            .flex()
            .flex_col()
            .child(self.emoji_create(theme, content_width, cx));
        let mut rows = self.resource_status(rows, theme, cx);
        if let Some(emojis) = &self.emojis {
            let mut grid = div().flex().flex_wrap().gap(px(theme.space2));
            let columns = ((content_width + theme.space2) / (230. + theme.space2))
                .floor()
                .max(1.);
            let card_width = ((content_width - (columns - 1.) * theme.space2) / columns).max(1.);
            if emojis.is_empty() {
                grid = grid
                    .child(Self::resource_empty(&theme, "No custom emojis yet.").w(px(card_width)));
            }
            for emoji in emojis {
                let preview = emoji.clone();
                let delete = emoji.clone();
                grid = grid.child(
                    div()
                        .flex()
                        .w(px(card_width))
                        .max_w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(theme.space2))
                        .p(px(10.))
                        .rounded(px(theme.radius_sm))
                        .bg(theme.surface_interactive)
                        .child(
                            div().size(px(42.)).flex_shrink_0().child(
                                Button::new(
                                    ElementId::Name(format!("preview-emoji-{}", emoji.id).into()),
                                    theme,
                                )
                                .variant(ButtonVariant::EmojiPreview)
                                .tooltip(format!("Preview :{}:", emoji.name))
                                .leading(self.emoji_image(emoji, 34., theme, cx))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_dialog(Dialog::Emoji(preview.clone()), window, cx)
                                    },
                                )),
                            ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .flex_1()
                                .gap(px(theme.space1))
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(theme.font_size_md))
                                        .child(format!(":{}:", emoji.name)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_color(theme.text_muted)
                                        .text_size(px(theme.font_size_sm))
                                        .child(self.emoji_creator(emoji)),
                                ),
                        )
                        .child(
                            Button::new(
                                ElementId::Name(format!("delete-emoji-{}", emoji.id).into()),
                                theme,
                            )
                            .variant(ButtonVariant::DangerQuiet)
                            .label(t("serverSettings.delete"))
                            .disabled(self.busy || !self.allowed(1 << 4))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.open_dialog(Dialog::Emoji(delete.clone()), window, cx)
                                },
                            )),
                        ),
                );
            }
            rows = rows.child(grid);
        }
        rows.into_any_element()
    }
    fn emoji_create(&self, theme: Theme, content_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let can = self.allowed(1 << 4);
        let slots = 100usize.saturating_sub(self.emojis.as_ref().map_or(0, Vec::len));
        let preview = self
            .extra
            .emoji
            .emoji_preview
            .as_ref()
            .map(|image| {
                img(image.clone())
                    .id("emoji-upload-preview")
                    .size(px(80.))
                    .rounded(px(theme.radius_pill))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
            .unwrap_or_else(|| div().text_size(px(26.)).child("＋").into_any_element());
        let picker = div()
            .size(px(82.))
            .flex_shrink_0()
            .rounded(px(theme.radius_pill))
            .border_1()
            .border_dashed()
            .border_color(theme.outline)
            .child(
                Button::new("choose-emoji-file", theme)
                    .variant(ButtonVariant::Secondary)
                    .icon_only(true)
                    .icon_button_size(80.)
                    .tooltip(t("serverSettings.chooseEmoji"))
                    .leading(preview)
                    .disabled(self.busy || !can)
                    .on_click(cx.listener(|this, _, window, cx| this.pick_emoji(window, cx))),
            );
        let field = div().w_full().min_w_0().child(input_field(
            &theme,
            &t("serverSettings.emojiName"),
            &t_vars("serverSettings.emojiSlots", &[("count", slots)]),
            SettingsInput::new(&self.extra.emoji.emoji_name, theme)
                .label(t("serverSettings.emojiName"))
                .disabled(self.busy || !can),
        ));
        let button = Button::new("create-emoji", theme)
            .label(t("serverSettings.create"))
            .disabled(
                self.busy
                    || self.loading
                    || !can
                    || slots == 0
                    || self
                        .extra
                        .emoji
                        .emoji_name
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                    || self.extra.emoji.emoji_file.is_none(),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(file) = this.extra.emoji.emoji_file.clone() {
                    this.mutate(
                        Mutation::CreateEmoji(
                            this.extra.emoji.emoji_name.read(cx).value().to_string(),
                            file,
                        ),
                        window,
                        cx,
                    );
                }
            }));
        let form = div()
            .flex()
            .flex_col()
            .gap(px(theme.space3))
            .mt(px(theme.space5))
            .mb(px(theme.space6));
        if content_width <= 340. {
            return form
                .items_start()
                .child(picker)
                .child(field)
                .child(button)
                .into_any_element();
        }
        let row = div()
            .flex()
            .items_end()
            .gap(px(theme.space3))
            .child(picker)
            .child(div().min_w_0().flex_1().child(field));
        if content_width <= 560. {
            form.child(row)
                .child(div().pl(px(82. + theme.space3)).child(button))
                .into_any_element()
        } else {
            form.child(row.child(div().self_center().flex_shrink_0().child(button)))
                .into_any_element()
        }
    }
    pub(super) fn pick_emoji(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.allowed(1 << 4) {
            return;
        }
        self.busy = true;
        self.extra.picker_generation = self.extra.picker_generation.wrapping_add(1);
        let generation = self.extra.picker_generation;
        let task =
            crate::shared::file_picker::pick_image(window, cx, move |this, result, _, cx| {
                if this.extra.picker_generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(Some(file)) => {
                        if let Err(error) = file.validate(2_000_000) {
                            this.error = Some(error.to_string());
                        } else {
                            this.busy = true;
                            this.extra.emoji.emoji_file = None;
                            this.extra.emoji.emoji_preview = None;
                            let bytes = file.bytes.clone();
                            let task = cx.spawn(async move |this, cx| {
                                let frames =
                                    spawn_core(crate::shared::images::preview_frames(bytes, 82))
                                        .await;
                                this.update(cx, |this, cx| {
                                    if this.extra.picker_generation != generation {
                                        return;
                                    }
                                    this.busy = false;
                                    if let Some(frames) = frames {
                                        this.extra.emoji.emoji_preview =
                                            Some(Arc::new(RenderImage::new(frames)));
                                        this.extra.emoji.emoji_file = Some(file);
                                    } else {
                                        this.error = Some(t("crop.failed"));
                                    }
                                    cx.notify();
                                })
                                .ok();
                            });
                            this.extra.emoji.preview_task = Some(task);
                            this.error = None;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        self.extra.picker = Some(task);
        cx.notify();
    }
}

impl ServerSettings {
    pub(super) fn emoji_dialog(
        &self,
        emoji: &ServerEmoji,
        theme: Theme,
        cx: &mut Context<Self>,
        mut content: Div,
        mut footer: Div,
    ) -> (Div, Div) {
        content = content
            .child(self.emoji_image(emoji, 180., theme, cx))
            .child(
                div()
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child(self.emoji_creator(emoji)),
            )
            .child(input_field(
                &theme,
                &t("serverSettings.emojiName"),
                "",
                SettingsInput::new(&self.extra.emoji.rename_emoji, theme)
                    .label(t("serverSettings.emojiName"))
                    .disabled(self.busy),
            ));
        let id = emoji.id.clone();
        let rename = id.clone();
        footer = footer
            .child(self.dialog_cancel(theme, cx))
            .child(
                Button::new("delete-emoji-confirm", theme)
                    .variant(ButtonVariant::Danger)
                    .label(t("serverSettings.deleteEmoji"))
                    .disabled(self.busy || !self.allowed(1 << 4))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.mutate(Mutation::DeleteEmoji(id.clone()), window, cx)
                    })),
            )
            .child(
                Button::new("rename-emoji-confirm", theme)
                    .label(t("serverSettings.saveName"))
                    .disabled(
                        self.busy
                            || !self.allowed(1 << 4)
                            || self
                                .extra
                                .emoji
                                .rename_emoji
                                .read(cx)
                                .value()
                                .trim()
                                .is_empty()
                            || self.extra.emoji.rename_emoji.read(cx).value().trim() == emoji.name,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.mutate(
                            Mutation::RenameEmoji(
                                rename.clone(),
                                this.extra.emoji.rename_emoji.read(cx).value().to_string(),
                            ),
                            window,
                            cx,
                        )
                    })),
            );
        (content, footer)
    }
}
