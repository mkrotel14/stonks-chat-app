use super::*;
use gpui_kit::prelude::FluentBuilder;
use stonks_ui::{
    components::{Button, ButtonVariant, SettingsInput, UiCloseButton, UiDialog, UiRange},
    i18n::t_vars,
    Theme,
};

impl MusicCard {
    /// `width` is the containing dock width, not the window width (Tauri container queries).
    /// The calling view should observe the entity and mount `render_playlist` in its overlay layer.
    pub(crate) fn render_card(
        &self,
        entity: &Entity<Self>,
        layout: MusicLayout,
        width: Pixels,
        theme: Theme,
        cx: &App,
    ) -> AnyElement {
        let s = &self.snapshot;
        if !s.active && s.error.is_none() {
            return div().into_any_element();
        }
        let narrow = width <= px(220.);
        let inline = layout == MusicLayout::Inline;
        let compact = layout == MusicLayout::Compact;
        let horizontal = layout == MusicLayout::Horizontal;
        let mut card = div()
            .id(("shared-music", entity.entity_id()))
            .aria_label(t("voice.sharedMusicPlayer"))
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .gap(px(if inline || compact {
                4.
            } else if horizontal {
                8.
            } else {
                9.
            }));
        if !inline {
            card = card
                .p(px(if compact {
                    theme.space1
                } else if horizontal {
                    9.
                } else if width <= px(170.) {
                    8.
                } else {
                    11.
                }))
                .border_1()
                .border_color(theme.outline)
                .rounded(px(theme.radius_md))
                .bg(theme.surface_raised)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_md.color.into(),
                    offset: point(px(0.), px(theme.shadow_md.dy)),
                    blur_radius: px(theme.shadow_md.blur),
                    spread_radius: px(0.),
                    inset: false,
                }]);
        }
        if !s.active {
            card = card.child(
                row(10.)
                    .child(art(None, "!", 38., theme.accent, theme.accent_text, theme))
                    .child(
                        column(2.)
                            .child(caption(t("voice.musicError"), theme))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(t("voice.playbackFailed")),
                            ),
                    ),
            );
        } else {
            let size = if inline || narrow && !compact {
                27.
            } else {
                34.
            };
            let (pause, icon, label) = if s.paused {
                (MusicCommand::Resume, PLAY, "Resume")
            } else {
                (MusicCommand::Pause, PAUSE, "Pause")
            };
            let pause = self.command_button(
                entity,
                icon_button("music-pause", label, icon, size, false, theme),
                pause,
            );
            let card_handle = entity.clone();
            let playlist = icon_button(
                "music-playlist",
                "Open playlist",
                PLAYLIST,
                size,
                self.playlist_open,
                theme,
            )
            .accessibility_label(format!("Open playlist, {} songs queued", s.queue.len()))
            .expanded(self.playlist_open)
            .on_click(move |_, window, cx| {
                card_handle.update(cx, |this, cx| {
                    this.volume_open = false;
                    this.page = 0;
                    this.url
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    this.opener = window.focused(cx);
                    this.playlist_open = true;
                    this.dialog_focus.focus(window, cx);
                    cx.notify();
                })
            });
            let playlist = div().relative().child(playlist).when(
                !compact && !inline && !s.queue.is_empty(),
                |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(px(-5.))
                            .right(px(-5.))
                            .min_w(px(16.))
                            .h(px(16.))
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(theme.surface_raised)
                            .rounded(px(theme.radius_sm))
                            .bg(theme.surface_selected)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(theme.font_size_xs))
                            .child(s.queue.len().to_string()),
                    )
                },
            );
            if compact {
                card = card
                    .items_center()
                    .child(art(
                        s.artwork.as_ref(),
                        "♫",
                        70.,
                        theme.accent,
                        theme.accent_text,
                        theme,
                    ))
                    .child(
                        row(theme.space1)
                            .justify_center()
                            .child(pause)
                            .child(playlist),
                    );
            } else {
                let seek = self.render_seek(entity, theme, !inline);
                let skip = self.command_button(
                    entity,
                    icon_button("music-skip", "Skip song", SKIP, size, false, theme),
                    MusicCommand::Skip,
                );
                let volume = self.render_volume(entity, size, theme, cx);
                if inline {
                    card = card.child(
                        column(5.)
                            .child(track(self.display_title(), theme.font_size_sm))
                            .children(seek)
                            .child(
                                row(3.)
                                    .child(volume)
                                    .child(pause)
                                    .child(skip)
                                    .child(playlist),
                            ),
                    );
                } else {
                    let status = if s.loading || s.seeking {
                        "voice.loading"
                    } else if s.paused {
                        "voice.paused"
                    } else {
                        "voice.nowPlaying"
                    };
                    let elapsed = music_time(s.elapsed_seconds);
                    let time = s
                        .duration_seconds
                        .filter(|d| *d > 0)
                        .map_or(elapsed.clone(), |duration| {
                            format!("{elapsed} / {}", music_time(duration))
                        });
                    let now = row(10.)
                        .flex_1()
                        .child(art(
                            s.artwork.as_ref(),
                            "♫",
                            38.,
                            theme.accent,
                            theme.accent_text,
                            theme,
                        ))
                        .child(
                            column(2.)
                                .flex_1()
                                .child(
                                    caption(t(status), theme)
                                        .line_height(relative(theme.line_height_normal)),
                                )
                                .child(
                                    track(self.display_title(), theme.font_size_sm)
                                        .line_height(relative(theme.line_height_normal)),
                                )
                                .child(
                                    subtitle(time, theme)
                                        .line_height(relative(theme.line_height_normal)),
                                ),
                        );
                    let controls = row(if narrow {
                        4.
                    } else if horizontal {
                        5.
                    } else {
                        10.
                    })
                    .flex_shrink_0()
                    .child(volume)
                    .child(
                        row(if narrow { 4. } else { 5. })
                            .child(pause)
                            .child(skip)
                            .child(playlist),
                    );
                    card = if horizontal && !narrow {
                        card.child(row(10.).child(now).child(controls))
                    } else {
                        card.child(now).child(
                            controls
                                .when(narrow, |el| el.justify_center())
                                .when(!narrow, |el| el.justify_between()),
                        )
                    };
                    card = card.children(seek);
                }
            }
        }
        card.when_some(s.error.as_ref(), |el, error| {
            el.child(
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.danger)
                    .child(error.clone()),
            )
        })
        .into_any_element()
    }

    fn render_seek(&self, entity: &Entity<Self>, theme: Theme, slim: bool) -> Option<AnyElement> {
        let duration = self.snapshot.duration_seconds.filter(|d| *d > 1)?;
        let position = self
            .preview
            .unwrap_or(self.snapshot.elapsed_seconds.min(duration - 1));
        let enabled = self.seek_enabled();
        let entity = entity.clone();
        Some(
            div()
                .id("music-seek-wrapper")
                .w_full()
                .min_w_0()
                .capture_key_down(move |event, window, cx| {
                    let target = match event.keystroke.key.as_str() {
                        "left" | "down" => position.saturating_sub(1),
                        "right" | "up" => position.saturating_add(1).min(duration - 1),
                        "home" => 0,
                        "end" => duration - 1,
                        _ => return,
                    };
                    if enabled {
                        if target != position {
                            entity.update(cx, |this, cx| {
                                this.seek.update(cx, |slider, cx| {
                                    slider.set_value(target as f32, window, cx)
                                });
                                this.commit_seek(target, window, cx);
                            });
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                })
                .child(
                    UiRange::new(&self.seek, theme)
                        .slim(slim)
                        .label(format!(
                            "Seek shared music, {} of {}",
                            music_time(position),
                            music_time(duration)
                        ))
                        .disabled(!enabled),
                )
                .into_any_element(),
        )
    }

    fn render_volume(
        &self,
        entity: &Entity<Self>,
        size: f32,
        theme: Theme,
        cx: &App,
    ) -> impl IntoElement {
        let SliderValue::Single(volume) = self.volume.read(cx).value() else {
            unreachable!("music volume uses a single thumb")
        };
        let enabled = self.actions.volume.is_some();
        let toggle = entity.clone();
        let outside = entity.clone();
        div()
            .id("music-volume-anchor")
            .relative()
            .flex_shrink_0()
            .child(
                icon_button(
                    "music-volume",
                    "Your music volume",
                    VOLUME,
                    size,
                    self.volume_open,
                    theme,
                )
                .accessibility_label(format!("Your music volume: {volume:.0}%"))
                .disabled(!enabled)
                .expanded(self.volume_open)
                .on_click(move |_, _, cx| {
                    toggle.update(cx, |this, cx| {
                        if this.actions.volume.is_some() && this.snapshot.active {
                            this.volume_open = !this.volume_open;
                            cx.notify();
                        }
                    })
                }),
            )
            .when(self.volume_open && enabled, |anchor| {
                anchor.child(
                    deferred(
                        div()
                            .id("music-volume-dismiss-region")
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .pb(px(39.))
                            .on_mouse_down_out(move |_, _, cx| {
                                outside.update(cx, |this, cx| {
                                    this.volume_open = false;
                                    cx.notify();
                                })
                            })
                            .child(
                                row(theme.space2)
                                    .id("music-volume-popup")
                                    .w(px(160.))
                                    .p(px(theme.space2))
                                    .occlude()
                                    .rounded(px(theme.radius_sm))
                                    .border_1()
                                    .border_color(theme.outline)
                                    .bg(theme.surface_raised)
                                    .child(
                                        div().min_w_0().flex_1().child(
                                            UiRange::new(&self.volume, theme)
                                                .label("Your music volume"),
                                        ),
                                    )
                                    .child(subtitle(format!("{volume:.0}%"), theme)),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }

    /// Mount at the parent window's overlay root, not inside the dock/card bounds.
    pub(crate) fn render_playlist(
        &self,
        entity: &Entity<Self>,
        theme: Theme,
        cx: &App,
    ) -> Option<AnyElement> {
        if !self.playlist_open {
            return None;
        }
        let s = &self.snapshot;
        let close = entity.clone();
        let header_close = entity.clone();
        let submit = entity.clone();
        let disabled = s.busy || self.actions.request.is_none();
        let url = self.url.read(cx).value();
        let mut form = column(theme.space2)
            .px(px(theme.space5))
            .pb(px(theme.space4))
            .child(
                row(theme.space2)
                    .child(
                        div().min_w_0().flex_1().child(
                            SettingsInput::new(&self.url, theme)
                                .label(t("voice.songUrl"))
                                .disabled(self.actions.request.is_none()),
                        ),
                    )
                    .child(
                        Button::new("music-add", theme)
                            .label(t("common.add"))
                            .disabled(disabled || request_url(&url).is_none())
                            .on_click(move |_, window, cx| {
                                submit.update(cx, |this, cx| this.request(window, cx))
                            }),
                    ),
            );
        if let Some(error) = &s.request_error {
            form = form.child(
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        } else if s.busy {
            form = form.child(subtitle(t("voice.resolvingTrack"), theme));
        }
        let current = row(theme.space3)
            .mx(px(theme.space5))
            .mb(px(theme.space4))
            .p(px(theme.space3))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_md))
            .bg(theme.canvas)
            .child(art(
                s.artwork.as_ref(),
                "♫",
                52.,
                theme.canvas,
                theme.text,
                theme,
            ))
            .child(
                column(theme.space1)
                    .flex_1()
                    .child(caption(t("voice.nowPlaying"), theme))
                    .child(track(
                        if s.title.is_empty() {
                            t("voice.resolvingTrack")
                        } else {
                            s.title.clone()
                        },
                        theme.font_size_md,
                    ))
                    .child(subtitle(
                        s.requester_name
                            .as_ref()
                            .map_or_else(|| t("voice.sharedWithVoice"), |name| added_by(name)),
                        theme,
                    )),
            );
        let heading = row(theme.space2)
            .justify_between()
            .px(px(theme.space5))
            .pb(px(theme.space2))
            .child(caption(t("voice.upNext"), theme))
            .child(
                div()
                    .min_w(px(21.))
                    .h(px(21.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.surface_selected)
                    .text_size(px(theme.font_size_xs))
                    .child(s.queue.len().to_string()),
            );
        let mut queue = column(0.);
        if s.queue.is_empty() {
            queue = queue.child(
                div()
                    .mx(px(theme.space5))
                    .mb(px(theme.space4))
                    .p(px(theme.space6))
                    .border_1()
                    .border_color(theme.outline)
                    .rounded(px(theme.radius_sm))
                    .text_center()
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child(t("voice.emptyQueue")),
            );
        } else {
            let start = self.page * PAGE_SIZE;
            queue = queue.child(
                column(0.)
                    .px(px(theme.space3))
                    .pb(px(theme.space2))
                    .children(s.queue.iter().enumerate().skip(start).take(PAGE_SIZE).map(
                        |(index, item)| {
                            let remove = self.command_button(
                                entity,
                                icon_button(
                                    ("music-remove", index),
                                    &format!("Remove {}", item.title),
                                    CLOSE,
                                    34.,
                                    false,
                                    theme,
                                )
                                .accessibility_label(format!(
                                    "Remove {} from playlist",
                                    item.title
                                )),
                                MusicCommand::Remove(item.id.clone()),
                            );
                            row(theme.space3)
                                .p(px(theme.space2))
                                .rounded(px(theme.radius_sm))
                                .hover(|el| el.bg(theme.surface_interactive))
                                .child(art(
                                    item.artwork.as_ref(),
                                    (index + 1).to_string(),
                                    40.,
                                    theme.canvas,
                                    theme.text_muted,
                                    theme,
                                ))
                                .child(
                                    column(theme.space1)
                                        .flex_1()
                                        .child(track(item.title.clone(), theme.font_size_md))
                                        .child(subtitle(
                                            item.requester_name.as_ref().map_or_else(
                                                || item.source_url.clone(),
                                                |name| added_by(name),
                                            ),
                                            theme,
                                        )),
                                )
                                .child(remove)
                        },
                    )),
            );
            let pages = page_count(s.queue.len());
            if pages > 1 {
                let previous = entity.clone();
                let next = entity.clone();
                queue = queue.child(
                    row(theme.space3)
                        .justify_center()
                        .pt(px(theme.space2))
                        .pb(px(theme.space3))
                        .child(
                            icon_button(
                                "music-previous-page",
                                "Previous playlist page",
                                LEFT,
                                34.,
                                false,
                                theme,
                            )
                            .disabled(self.page == 0)
                            .on_click(move |_, _, cx| {
                                previous.update(cx, |this, cx| {
                                    this.page = this.page.saturating_sub(1);
                                    cx.notify();
                                })
                            }),
                        )
                        .child(subtitle(format!("{} / {pages}", self.page + 1), theme))
                        .child(
                            icon_button(
                                "music-next-page",
                                "Next playlist page",
                                RIGHT,
                                34.,
                                false,
                                theme,
                            )
                            .disabled(self.page + 1 >= pages)
                            .on_click(move |_, _, cx| {
                                next.update(cx, |this, cx| {
                                    this.page = (this.page + 1)
                                        .min(page_count(this.snapshot.queue.len()) - 1);
                                    cx.notify();
                                })
                            }),
                        ),
                );
            }
        }
        let footer = row(0.)
            .justify_end()
            .px(px(theme.space5))
            .pt(px(theme.space3))
            .pb(px(theme.space4))
            .border_t_1()
            .border_color(theme.outline)
            .child(
                self.command_button(
                    entity,
                    Button::new("music-stop", theme)
                        .variant(ButtonVariant::DangerSubtle)
                        .label(t("voice.stopMusic")),
                    MusicCommand::Stop,
                ),
            );
        Some(
            UiDialog::new(
                ("music-playlist-dialog", entity.entity_id()),
                theme,
                t("voice.playlist"),
            )
            .width(500.)
            .viewport_inset(16.)
            .padding(0.)
            .gap(0.)
            .focus(&self.dialog_focus)
            .show_close(false)
            .title_content(
                row(theme.space2)
                    .justify_between()
                    .px(px(theme.space5))
                    .pt(px(theme.space5))
                    .pb(px(theme.space4))
                    .child(
                        div()
                            .text_size(px(theme.font_size_xl))
                            .font_weight(FontWeight::BOLD)
                            .child(t("voice.playlist")),
                    )
                    .child(UiCloseButton::new("music-playlist-close", theme).on_click(
                        move |_, window, cx| {
                            header_close.update(cx, |this, cx| this.close_playlist(window, cx))
                        },
                    )),
            )
            .on_close(move |_, window, cx| {
                close.update(cx, |this, cx| this.close_playlist(window, cx))
            })
            .child(form)
            .child(current)
            .child(heading)
            .child(queue)
            .child(footer)
            .into_any_element(),
        )
    }

    fn display_title(&self) -> String {
        let s = &self.snapshot;
        if let Some(artist) = s.artist.as_ref().filter(|artist| !artist.is_empty()) {
            format!("{} - {artist}", s.title)
        } else if s.title.is_empty() {
            t("voice.resolvingTrack")
        } else {
            s.title.clone()
        }
    }

    fn command_button(
        &self,
        entity: &Entity<Self>,
        button: Button,
        command: MusicCommand,
    ) -> Button {
        let enabled = self.command_enabled(&command);
        let entity = entity.clone();
        button.disabled(!enabled).on_click(move |_, window, cx| {
            let card = entity.read(cx);
            if card.command_enabled(&command) {
                if let Some(handler) = card.actions.control.clone() {
                    handler(command.clone(), window, cx);
                }
            }
        })
    }
}

fn row(gap: f32) -> Div {
    div().flex().min_w_0().items_center().gap(px(gap))
}
fn column(gap: f32) -> Div {
    div().flex().flex_col().min_w_0().gap(px(gap))
}
fn caption(text: String, theme: Theme) -> Div {
    subtitle(text, theme).font_weight(FontWeight::EXTRA_BOLD)
}
fn subtitle(text: String, theme: Theme) -> Div {
    div()
        .truncate()
        .text_size(px(theme.font_size_xs))
        .text_color(theme.text_muted)
        .child(text)
}
fn track(text: String, size: f32) -> Div {
    div()
        .truncate()
        .text_size(px(size))
        .font_weight(FontWeight::BOLD)
        .child(text)
}
fn added_by(name: &str) -> String {
    t_vars("voice.addedBy", &[("name", name)])
}

fn art(
    image: Option<&Arc<RenderImage>>,
    fallback: impl Into<SharedString>,
    size: f32,
    bg: Rgba,
    fg: Rgba,
    theme: Theme,
) -> Div {
    div()
        .size(px(size))
        .max_w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(px(theme.radius_sm))
        .bg(bg)
        .text_color(fg)
        .text_size(px(if size == 40. {
            theme.font_size_xs
        } else {
            theme.font_size_xl
        }))
        .when_some(image.cloned(), |el, image| {
            el.child(img(image).size_full().object_fit(ObjectFit::Cover))
        })
        .when(image.is_none(), |el| el.child(fallback.into()))
}

// Exact Tauri glyph paths, local to this port. Button feedback comes from the shared primitive.
const PLAY: &str = "m8 5 11 7-11 7Z";
const PAUSE: &str = "M8 5v14M16 5v14";
const SKIP: &str = "m6 5 10 7-10 7ZM18 5v14";
const VOLUME: &str = "M5 9v6h4l5 4V5L9 9ZM17 9a4 4 0 0 1 0 6M19.5 6.5a8 8 0 0 1 0 11";
const PLAYLIST: &str = "M4 6h11M4 11h11M4 16h7M19 13v7M16 17l3 3 3-3";
const CLOSE: &str = "m7 7 10 10M17 7 7 17";
const LEFT: &str = "m15 5-7 7 7 7";
const RIGHT: &str = "m9 5 7 7-7 7";

fn icon_button(
    id: impl Into<ElementId>,
    label: &str,
    path: &'static str,
    size: f32,
    selected: bool,
    theme: Theme,
) -> Button {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static GLYPHS: RefCell<HashMap<(&'static str, u32), Arc<Image>>> = RefCell::default();
    }
    let color = if selected {
        theme.accent_text
    } else {
        theme.text
    };
    let rgba = u32::from(color);
    let image = GLYPHS.with(|glyphs| glyphs.borrow_mut().entry((path, rgba)).or_insert_with(|| {
        Arc::new(Image::from_bytes(ImageFormat::Svg, format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#{:06x}" stroke-opacity="{}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="{path}"/></svg>"##, rgba >> 8, color.a).into_bytes()))
    }).clone());
    Button::new(id, theme)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        })
        .icon_button_size(size)
        .tooltip(label.to_owned())
        .accessibility_label(label.to_owned())
        .leading(img(image).size(px(20.)))
}
