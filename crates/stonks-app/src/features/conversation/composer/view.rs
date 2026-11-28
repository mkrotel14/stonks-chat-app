//! Composer layout and UI event bindings.
use super::{state::SubmitError, Composer};
use gpui_kit::{component::input::Textarea, *};
use stonks_ui::{
    components::{Button, ButtonVariant},
    i18n::t,
    icons::Icon,
    Theme,
};

const COMPOSER_ROW_HEIGHT: f32 = 52.;

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let spelling = self.spelling_overlay(theme.danger.into(), window, cx);
        let names = self
            .state
            .active_channel()
            .map(|channel| self.remote_typing.names(channel, self.now()))
            .unwrap_or_default();
        let emoji = self.emoji_button(window, cx);
        let composer = cx.entity().downgrade();
        let mentions = if self.input.read(cx).selected_range().is_empty() {
            self.mention_popup(window, &theme, cx)
        } else {
            None
        };
        div()
            .id("message-composer")
            .flex()
            .flex_col()
            .gap(px(theme.space1))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.stop_typing(cx);
                }
            }))
            .children(
                self.state
                    .picker_error()
                    .map(|error| div().text_color(theme.danger).child(error.to_owned())),
            )
            .children(self.attachment_notice().map(|notice| {
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_sm))
                    .child(notice)
            }))
            .children(self.state.error().and_then(|error| match error {
                SubmitError::Music(error) => {
                    Some(div().text_color(theme.danger).child(error.clone()))
                }
                SubmitError::Send(_) => None,
            }))
            .children(self.state.error().and_then(|error| {
                let SubmitError::Send(error) = error else {
                    return None;
                };
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.space2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(theme.danger)
                                .child(error.to_owned()),
                        )
                        .child(
                            Button::new("composer-retry", theme)
                                .variant(ButtonVariant::Quiet)
                                .label(t("common.retry"))
                                .disabled(!self.can_send())
                                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                        ),
                )
            }))
            .children((!names.is_empty()).then(|| {
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(
                        t(if names.len() == 1 {
                            "messages.typingOne"
                        } else {
                            "messages.typingMany"
                        })
                        .replace("{names}", &names.join(", ")),
                    )
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .relative()
                    .children(mentions)
                    // Keep the same corners when the reply header makes the surface taller.
                    .rounded(px(COMPOSER_ROW_HEIGHT / 2. + 1.))
                    .border_1()
                    .border_color(theme.outline)
                    .bg(theme.surface_raised)
                    .children(self.state.reply().map(|reply| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme.space2))
                            .mx(px(theme.space3))
                            .px(px(theme.space2))
                            .pt(px(theme.space3))
                            .pb(px(theme.space2))
                            .border_b_1()
                            .border_color(theme.outline)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(theme.space1))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(theme.font_size_md))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(format!(
                                                "{} {}",
                                                t("messages.replyingTo"),
                                                reply.author
                                            )),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(theme.font_size_sm))
                                            .text_color(theme.text_muted)
                                            .child(
                                                reply.excerpt.clone().unwrap_or_else(|| {
                                                    t("messages.attachmentLower")
                                                }),
                                            ),
                                    ),
                            )
                            .child(
                                Button::new("composer-cancel-reply", theme)
                                    .variant(ButtonVariant::Close)
                                    .tooltip(t("messages.cancelReply"))
                                    .leading(Icon::X.render_stroke(theme.text_muted, 18., 1.8))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.cancel_reply(window, cx) {
                                            this.focus(window, cx);
                                        }
                                    })),
                            )
                    }))
                    .children(
                        (!self.state.files().is_empty())
                            .then(|| self.attachment_previews(theme, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .min_h(px(COMPOSER_ROW_HEIGHT))
                            .px(px(6.))
                            .py(px(5.))
                            .child(
                                div().w(px(38.)).flex_shrink_0().child(
                                    Button::new("composer-upload", theme)
                                        .variant(ButtonVariant::Icon)
                                        .tooltip(t(if self.upload_allowed() {
                                            "messages.attachFiles"
                                        } else {
                                            "messages.noUploadPermission"
                                        }))
                                        .leading(Icon::Plus.render_stroke(theme.text, 22., 1.7))
                                        .disabled(!self.can_pick())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_files(window, cx)
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .relative()
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.mentions.open =
                                                this.input.read(cx).selected_range().is_empty();
                                            this.mentions.selected = 0;
                                            cx.notify();
                                        }),
                                    )
                                    .child(
                                        Textarea::new(&self.input)
                                            .on_paste(move |item, window, cx| {
                                                composer
                                                    .update(cx, |this, cx| {
                                                        this.paste_image(item, window, cx)
                                                    })
                                                    .unwrap_or(false)
                                            })
                                            .appearance(false)
                                            .bordered(false)
                                            .aria_label("Message")
                                            .h(px(38.))
                                            .p_0()
                                            .text_size(px(theme.font_size_md))
                                            .line_height(px(
                                                theme.font_size_md * theme.line_height_normal
                                            ))
                                            .disabled(
                                                self.state.active_channel().is_none()
                                                    || self.state.sending_here(),
                                            ),
                                    )
                                    .children(spelling),
                            )
                            .child(div().w(px(38.)).flex_shrink_0().child(emoji))
                            .child(
                                div().w(px(40.)).flex_shrink_0().child(
                                    Button::new("composer-send", theme)
                                        .variant(ButtonVariant::Primary)
                                        .icon_only(true)
                                        .tooltip(t("messages.send"))
                                        .leading(Icon::SendHorizontal.render_stroke(
                                            theme.accent_text,
                                            22.,
                                            1.7,
                                        ))
                                        .disabled(!self.can_send())
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.send(window, cx)
                                            }),
                                        ),
                                ),
                            ),
                    ),
            )
    }
}
