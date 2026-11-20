//! Account Settings → Soundboard library.
use super::*;
use stonks_ui::components::{Switch, UiDialog};

impl Voice {
    pub(crate) fn soundboard_settings_page(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        let panel = self.soundboard.as_ref().filter(|panel| !panel.quick)?;
        let theme = Theme::current(cx);
        let busy = panel.work.is_some();
        let control = |id: &'static str, label: &str| {
            Button::new(id, theme)
                .label(t(label))
                .variant(ButtonVariant::Secondary)
                .disabled(busy)
        };
        let mut body = div().flex().flex_col().gap(px(theme.space3));
        body = body.child(self.soundboard_volume_control(panel, theme));
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(theme.space2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child(t("soundboard.localSounds")),
                        )
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .child(t("soundboard.description")),
                        ),
                )
                .child(
                    control("sound-import", "soundboard.addSound")
                        .variant(ButtonVariant::Primary)
                        .disabled(busy || panel.edit.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                ),
        );
        if let Some(error) = &panel.error {
            body = body.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if let Some(library) = &panel.library {
            if library.clips.is_empty() {
                body = body.child(
                    div()
                        .p(px(theme.space3))
                        .child(t("soundboard.empty"))
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .child(t("soundboard.emptyDescription")),
                        ),
                );
            }
            for clip in &library.clips {
                let id = &clip.id;
                let editing = matches!(&panel.edit, Some(Edit::Rename(edit_id)) if edit_id == id);
                let selected = library.join_clip_id.as_ref() == Some(id);
                let locked = busy || panel.edit.is_some();
                let icon_button = |action, icon: Icon, label: String| {
                    Button::new(SharedString::from(format!("sound-{action}-{id}")), theme)
                        .icon_only(true)
                        .variant(ButtonVariant::Secondary)
                        .leading(icon.render(theme.text, 16.))
                        .tooltip(label.clone())
                        .accessibility_label(label)
                        .disabled(locked)
                };
                let preview = id.clone();
                let join = id.clone();
                let rename = clip.clone();
                let delete = clip.clone();
                let mut row = div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap(px(theme.space2))
                    .p(px(theme.space3))
                    .rounded(px(theme.radius_md))
                    .border_1()
                    .border_color(theme.outline)
                    .bg(theme.surface_raised)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(theme.space2))
                            .child(
                                icon_button(
                                    "preview",
                                    Icon::Play,
                                    t_vars("soundboard.preview", &[("name", &clip.name)]),
                                )
                                .disabled(locked || panel.playback.is_some())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.play_sound(preview.clone(), true, cx)
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(100.))
                                    .when(editing, |copy| {
                                        copy.child(
                                            SettingsInput::new(&panel.name, theme)
                                                .label(t("common.rename"))
                                                .disabled(busy),
                                        )
                                    })
                                    .when(!editing, |copy| {
                                        copy.child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::BOLD)
                                                .child(clip.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(theme.font_size_xs))
                                                .text_color(theme.text_muted)
                                                .child(t_vars(
                                                    "soundboard.seconds",
                                                    &[(
                                                        "seconds",
                                                        &format!(
                                                            "{:.1}",
                                                            clip.duration_ms as f64 / 1000.
                                                        ),
                                                    )],
                                                )),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(theme.space2))
                                    .text_size(px(theme.font_size_xs))
                                    .child(t("soundboard.joiningSound"))
                                    .child(
                                        Switch::new(
                                            SharedString::from(format!("sound-join-{id}")),
                                            theme,
                                        )
                                        .checked(selected)
                                        .disabled(locked)
                                        .label(t("soundboard.joiningSound"))
                                        .on_change(
                                            cx.listener(move |this, next: &bool, window, cx| {
                                                this.change(
                                                    Change::Join(next.then(|| join.clone())),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    ),
                            )
                            .child(if editing {
                                control("sound-save-name", "common.save").on_click(cx.listener(
                                    |this, _, window, cx| this.save_sound_name(window, cx),
                                ))
                            } else {
                                Button::new(SharedString::from(format!("sound-rename-{id}")), theme)
                                    .label(t("common.rename"))
                                    .variant(ButtonVariant::Secondary)
                                    .disabled(locked)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(panel) = &mut this.soundboard {
                                            panel.edit = Some(Edit::Rename(rename.id.clone()));
                                            panel.name.update(cx, |input, cx| {
                                                input.set_value(rename.name.clone(), window, cx);
                                                input.focus(window, cx);
                                            });
                                            cx.notify();
                                        }
                                    }))
                            })
                            .child(
                                icon_button("delete", Icon::Trash, t("common.delete"))
                                    .variant(ButtonVariant::DangerSubtle)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(panel) = &mut this.soundboard {
                                            let focus = cx.focus_handle();
                                            focus.focus(window, cx);
                                            panel.edit = Some(Edit::Delete {
                                                clip: delete.clone(),
                                                focus,
                                            });
                                            cx.notify();
                                        }
                                    })),
                            ),
                    );
                if editing {
                    row = row.child(div().flex().justify_end().child(
                        control("sound-cancel-edit", "common.cancel").on_click(
                            cx.listener(|this, _, window, cx| this.dismiss_soundboard(window, cx)),
                        ),
                    ));
                }
                body = body.child(row);
            }
        } else if busy {
            body = body.child(t("common.loading"));
        } else {
            body = body.child(control("sound-settings-retry", "common.retry").on_click(
                cx.listener(|this, _, window, cx| this.change(Change::Refresh, window, cx)),
            ));
        }
        let delete_dialog = if let Some(Edit::Delete { clip, focus }) = &panel.edit {
            let id = clip.id.clone();
            let mut dialog = UiDialog::new(
                "sound-delete-confirmation",
                theme,
                t_vars("soundboard.deleteConfirm", &[("name", &clip.name)]),
            )
            .description(t("soundboard.deleteDescription"))
            .focus(focus)
            .busy(busy)
            .on_close(cx.listener(|this, _, window, cx| this.dismiss_soundboard(window, cx)));
            if let Some(error) = &panel.error {
                dialog = dialog.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
            }
            Some(
                dialog.child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(theme.space2))
                        .child(control("sound-delete-cancel", "common.cancel").on_click(
                            cx.listener(|this, _, window, cx| this.dismiss_soundboard(window, cx)),
                        ))
                        .child(
                            control("sound-confirm-delete", "common.delete")
                                .variant(ButtonVariant::DangerSubtle)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.change(Change::Delete(id.clone()), window, cx)
                                })),
                        ),
                ),
            )
        } else {
            None
        };
        Some((
            body.into_any_element(),
            delete_dialog.map(IntoElement::into_any_element),
        ))
    }
}
