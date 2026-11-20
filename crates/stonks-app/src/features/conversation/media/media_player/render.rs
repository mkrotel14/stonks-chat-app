use super::*;
use stonks_ui::components::media_player::{
    audio_surface, file_header, video_placeholder, video_size, UiMediaPoster,
};
impl MediaPlayer {
    fn download_button(&self, theme: Theme, cx: &mut Context<Self>) -> Button {
        Button::new("media-download", theme)
            .variant(ButtonVariant::Icon)
            .tooltip(t(if self.spec.kind == MediaKind::Audio {
                "media.downloadAudio"
            } else {
                "media.downloadVideo"
            }))
            .leading(Icon::Download.render(theme.text_muted, 20.))
            .disabled(!self.enabled || self.downloading)
            .on_click(cx.listener(|this, _, _, cx| this.download(cx)))
    }
    pub(super) fn controls(
        &self,
        theme: Theme,
        video: bool,
        fullscreen: bool,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loading = self.current.status == PlaybackStatus::Loading;
        let label = if self.current.status == PlaybackStatus::Playing {
            "media.pause"
        } else {
            "media.play"
        };
        let mut controls = div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .items_center()
            .gap(px(10.));
        controls = controls.child(
            Button::new("media-toggle", theme)
                .variant(ButtonVariant::Primary)
                .icon_only(true)
                .tooltip(t(label))
                .leading(
                    (if self.current.status == PlaybackStatus::Playing {
                        Icon::Pause
                    } else {
                        Icon::Play
                    })
                    .render(theme.accent_text, 20.),
                )
                .disabled(!self.enabled || loading)
                .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
        );
        if !video || width >= 260. {
            controls = controls.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(format!(
                        "{} / {}",
                        clock(self.current.position),
                        clock(self.current.duration)
                    )),
            );
        }
        controls = controls.child(
            div().flex_1().min_w(px(70.)).child(
                UiRange::new(&self.seek, theme)
                    .label(t(if video {
                        "media.videoPosition"
                    } else {
                        "media.audioPosition"
                    }))
                    .disabled(loading || self.current.duration <= 0.),
            ),
        );
        if !video || width >= 380. {
            let focused = self.volume_focus.contains_focused(window, cx);
            let expanded = self.volume_hover || focused;
            controls = controls.child(
                div()
                    .id("media-volume")
                    .flex()
                    .w(px(if expanded { 98. } else { 28. }))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .items_center()
                    .gap(px(6.))
                    .text_color(theme.text_muted)
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.volume_hover != *hovered {
                            this.volume_hover = *hovered;
                            cx.emit(PlayerEvent::LayoutChanged);
                            cx.notify();
                        }
                    }))
                    .child(
                        div().size(px(20.)).flex_shrink_0().child(
                            (if self.preferred_volume <= 0. {
                                Icon::VolumeX
                            } else {
                                Icon::Volume2
                            })
                            .render(theme.text_muted, 20.),
                        ),
                    )
                    .child(
                        div().w(px(64.)).flex_shrink_0().child(
                            UiRange::new(&self.volume, theme)
                                .track_focus(&self.volume_focus)
                                .label(t(if video {
                                    "media.videoVolume"
                                } else {
                                    "media.audioVolume"
                                })),
                        ),
                    ),
            );
        }
        if video && width >= 340. {
            controls = controls.child(self.download_button(theme, cx));
        }
        if video {
            controls = controls.child(
                Button::new("media-fullscreen", theme)
                    .variant(ButtonVariant::Icon)
                    .tooltip(t(if fullscreen {
                        "media.exitFullscreen"
                    } else {
                        "media.fullscreen"
                    }))
                    .leading(
                        (if fullscreen {
                            Icon::Minimize
                        } else {
                            Icon::Maximize
                        })
                        .render(theme.text, 20.),
                    )
                    .disabled(self.current.frame.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if fullscreen {
                            this.exit_fullscreen(cx);
                        } else {
                            this.open_fullscreen(window, cx);
                        }
                    })),
            );
        }
        controls.into_any_element()
    }
    fn feedback(&self, theme: Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(error) = self.error() {
            let button = Button::new("media-retry", theme)
                .variant(ButtonVariant::DangerSubtle)
                .label(t("common.retry"))
                .leading(
                    div()
                        .min_w_0()
                        .flex_1()
                        .font_weight(FontWeight::NORMAL)
                        .text_size(px(theme.font_size_sm))
                        .child(error.to_owned()),
                )
                .disabled(self.current.status == PlaybackStatus::Loading)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.current.status == PlaybackStatus::Failed {
                        this.toggle(cx);
                    } else {
                        this.download(cx);
                    }
                }));
            return Some(
                div()
                    .mt(px(theme.space2))
                    .w_full()
                    .min_w_0()
                    .child(button)
                    .into_any_element(),
            );
        }
        if self.downloading || self.saved {
            return Some(
                div()
                    .mt(px(theme.space2))
                    .text_size(px(theme.font_size_xs))
                    .text_color(if self.saved {
                        theme.success
                    } else {
                        theme.text_muted
                    })
                    .child(t(if self.saved {
                        "media.saved"
                    } else {
                        "media.downloading"
                    }))
                    .into_any_element(),
            );
        }
        None
    }
}
impl Render for MediaPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus_wired {
            self.focus_wired = true;
            self._subscriptions
                .push(cx.on_focus_in(&self.volume_focus, window, |_, _, cx| {
                    cx.emit(PlayerEvent::LayoutChanged);
                    cx.notify();
                }));
            self._subscriptions
                .push(cx.on_focus_out(&self.volume_focus, window, |_, _, _, cx| {
                    cx.emit(PlayerEvent::LayoutChanged);
                    cx.notify();
                }));
        }
        self.sync_sliders(window, cx);
        let theme = Theme::current(cx);
        let mut root = div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_shrink_0()
            .w(px(self.spec.width))
            .max_w_full();
        if self.spec.kind == MediaKind::Audio {
            let detail = self
                .spec
                .size
                .map(|size| format!("{:.2} MB", size as f64 / 1_000_000.))
                .unwrap_or_default();
            let mut audio = audio_surface(&theme, self.spec.width)
                .child(file_header(
                    &theme,
                    Icon::Music,
                    &self.spec.title,
                    &detail,
                    self.download_button(theme, cx),
                ))
                .child(div().mt(px(10.)).child(self.controls(
                    theme,
                    false,
                    false,
                    self.spec.width,
                    window,
                    cx,
                )));
            if let Some(feedback) = self.feedback(theme, cx) {
                audio = audio.child(feedback);
            }
            return root.child(audio);
        }
        let active = !matches!(
            self.current.status,
            PlaybackStatus::Idle | PlaybackStatus::Failed
        );
        if !active {
            if self.spec.poster {
                root = root.child(
                    UiMediaPoster::new(
                        "media-poster",
                        theme,
                        self.spec.width,
                        t("media.playVideo"),
                    )
                    .image(self.poster.clone())
                    .disabled(!self.enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                );
            } else {
                let detail = self
                    .spec
                    .size
                    .map(|size| format!("{:.2} MB", size as f64 / 1_000_000.))
                    .unwrap_or_default();
                let placeholder = video_placeholder(&theme, self.spec.width)
                    .child(
                        div()
                            .size(px(32.))
                            .flex_shrink_0()
                            .rounded(px(theme.radius_sm))
                            .bg(theme.surface_selected)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::Play.render(theme.accent, 20.)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(theme.font_size_sm))
                                    .child(self.spec.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(theme.font_size_xs))
                                    .text_color(theme.text_muted)
                                    .child(detail),
                            ),
                    )
                    .child(
                        Button::new("media-preview", theme)
                            .label(t("media.preview"))
                            .disabled(!self.enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    )
                    .child(self.download_button(theme, cx));
                root = root.child(placeholder);
            }
        } else {
            let dimensions = self
                .current
                .frame
                .as_ref()
                .map(|frame| {
                    let size = frame.size(0);
                    (size.width.0.max(1) as u64, size.height.0.max(1) as u64)
                })
                .or(self.spec.dimensions);
            let (width, height) = video_size(self.spec.width, dimensions, self.spec.embedded);
            let mut video = div()
                .relative()
                .flex_shrink_0()
                .w(px(width))
                .h(px(height))
                .overflow_hidden()
                .rounded(px(theme.radius_sm))
                .bg(theme.media_surface);
            if let Some(frame) = &self.current.frame {
                video = video.child(
                    img(frame.clone())
                        .id("native-video-frame")
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded(px(theme.radius_sm)),
                );
            } else {
                video = video.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(Theme::dark().text_muted)
                        .text_size(px(theme.font_size_sm))
                        .child(t("media.loading")),
                );
            }
            // Video chrome remains a dark media surface in either application theme.
            let mut chrome = Theme::dark();
            chrome.accent = theme.accent;
            chrome.accent_text = theme.accent_text;
            chrome.focus = theme.focus;
            chrome.focus_ring = theme.focus_ring;
            if self.fullscreen.is_none() {
                video = video.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .px(px(theme.space2))
                        .py(px(theme.space1))
                        .bg(theme.media_chrome)
                        .child(self.controls(chrome, true, false, self.spec.width, window, cx)),
                );
            }
            root = root.child(video);
        }
        if let Some(feedback) = self.feedback(theme, cx) {
            root = root.child(feedback);
        }
        root
    }
}
