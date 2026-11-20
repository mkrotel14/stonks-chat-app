//! Local setup does not request a portal session or open a capture stream.
use super::{ScreenShareOptions, ScreenShareState, Share, Voice, VoicePhase};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::voice::screen::ScreenQuality;
#[cfg(windows)]
use stonks_core::voice::screen::{sources, ScreenShareError, ScreenSource, ScreenSourceKind};
use stonks_ui::{
    components::{Button, ButtonVariant, Checkbox, UiCloseButton, UiDialog},
    i18n::t,
    theme::Theme,
};

#[cfg(target_os = "linux")]
use stonks_ui::icons::Icon;

pub(in crate::features::voice) struct Setup {
    pub options: ScreenShareOptions,

    #[cfg(windows)]
    sources: Option<Result<Vec<ScreenSource>, ScreenShareError>>,
    #[cfg(windows)]
    _discovery: Task<()>,
    focus: FocusHandle,
    opener: Option<FocusHandle>,
}

impl Voice {
    pub(in crate::features::voice) fn toggle_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.camera_setup_open() {
            return;
        }
        let Some(call) = self
            .call
            .as_mut()
            .filter(|call| !call.hidden && call.stop.is_some())
        else {
            return;
        };
        match call.screen.state {
            ScreenShareState::Idle | ScreenShareState::Failed(_)
                if call.screen.setup.is_none()
                    && call
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.phase == VoicePhase::Connected) =>
            {
                let focus = cx.focus_handle();
                let opener = window.focused(cx);
                focus.focus(window, cx);
                #[cfg(windows)]
                let discovery = cx.spawn_in(window, async move |this, cx| {
                    let result = crate::runtime::spawn_core(sources()).await;
                    let _ = cx.update(|_, cx| {
                        this.update(cx, |voice, cx| {
                            if let Some(setup) = voice
                                .call
                                .as_mut()
                                .and_then(|call| call.screen.setup.as_mut())
                            {
                                setup.sources = Some(result);
                                cx.notify();
                            }
                        })
                    });
                });
                call.screen.setup = Some(Setup {
                    #[cfg(target_os = "linux")]
                    options: cx
                        .global::<crate::preferences::NativePreferences>()
                        .saved
                        .screen_options(),
                    #[cfg(windows)]
                    options: ScreenShareOptions {
                        quality: cx
                            .global::<crate::preferences::NativePreferences>()
                            .saved
                            .screen_share_quality
                            .screen(),
                        ..Default::default()
                    },
                    #[cfg(windows)]
                    sources: None,
                    #[cfg(windows)]
                    _discovery: discovery,
                    focus,
                    opener,
                });
            }
            ScreenShareState::Starting | ScreenShareState::Sharing => {
                if call.screen.sharing.try_send(Share::Stop).is_err() {
                    return;
                }
                call.screen.state = ScreenShareState::Stopping;
            }
            _ => return,
        }
        cx.notify();
    }

    pub(crate) fn screen_setup_open(&self) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| !call.hidden && call.stop.is_some() && call.screen.setup.is_some())
    }

    pub(crate) fn dismiss_screen_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(setup) = self.call.as_mut().and_then(|call| call.screen.setup.take()) {
            if let Some(opener) = setup.opener {
                opener.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn confirm_screen_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(call) = self.call.as_mut().filter(|call| {
            !call.hidden
                && call.stop.is_some()
                && call
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.phase == VoicePhase::Connected)
                && matches!(
                    call.screen.state,
                    ScreenShareState::Idle | ScreenShareState::Failed(_)
                )
        }) else {
            return;
        };
        let Some(setup) = call.screen.setup.as_ref() else {
            return;
        };
        let options = setup.options;
        #[cfg(windows)]
        if options.selection.is_none() {
            return;
        }
        if call.screen.sharing.try_send(Share::Start(options)).is_ok() {
            call.screen.state = ScreenShareState::Starting;
            self.dismiss_screen_setup(window, cx);
        }
    }

    pub(crate) fn screen_setup_surface(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.screen_setup_open() {
            return None;
        }
        let setup = self.call.as_ref()?.screen.setup.as_ref()?;
        let theme = Theme::current(cx);
        let selected = setup.options.quality;
        let dismiss = cx.listener(|this, _, window, cx| this.dismiss_screen_setup(window, cx));
        let toggle_audio = cx.listener(|this, checked: &bool, _, cx| {
            if let Some(setup) = this
                .call
                .as_mut()
                .and_then(|call| call.screen.setup.as_mut())
            {
                setup.options.share_audio = *checked;

                cx.notify();
            }
        });
        let description = || {
            div()
                .text_size(px(theme.font_size_xs))
                .line_height(relative(theme.line_height_normal))
                .text_color(theme.text_muted)
                .max_w(px(390.))
        };
        let title = t("voice.screenShareSettings");
        let audio_label = t("settings.voice.shareAudio");
        let dialog =
            UiDialog::new("screen-share-setup", theme, title.clone())
                .width(760.)
                .padding(theme.space5)
                .focus(&setup.focus)
                .show_close(false)
                .on_close(cx.listener(|this, _, window, cx| this.dismiss_screen_setup(window, cx)))
                .title_content(
                    div()
                        .relative()
                        .pr(px(44.))
                        .flex()
                        .flex_col()
                        .gap(px(theme.space2))
                        .font_weight(FontWeight::BOLD)
                        .line_height(relative(theme.line_height_compact))
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .font_weight(FontWeight(800.))
                                .child(t("voice.screenShare")),
                        )
                        .child(div().text_size(px(theme.font_size_xl)).child(title))
                        .child(div().absolute().top_0().right_0().child(
                            UiCloseButton::new("screen-share-close", theme).on_click(dismiss),
                        )),
                );
        #[cfg(windows)]
        let dialog = dialog.child(self.screen_source_choices(setup, theme, cx));
        #[cfg(target_os = "linux")]
        let dialog = dialog.child(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .text_center()
                .min_h(px(250.))
                .p(px(theme.space3))
                .gap(px(theme.space3))
                .rounded(px(theme.radius_lg))
                .bg(theme.canvas)
                .child(Icon::MonitorUp.render_stroke(theme.accent, 48., 1.4))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(t("voice.systemPickerNext")),
                )
                .child(description().child(t("voice.systemPickerDescription"))),
        );
        let dialog = dialog.child(QualityChoices {
            theme,
            buttons: ScreenQuality::ALL.map(|quality| {
                Button::new(("screen-quality", quality as usize), theme)
                    .variant(if quality == selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .label(match quality {
                        ScreenQuality::Hd30 => "720p 30 FPS",
                        ScreenQuality::FullHd30 => "1080p 30 FPS",
                        ScreenQuality::FullHd60 => "1080p 60 FPS",
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(setup) = this
                            .call
                            .as_mut()
                            .and_then(|call| call.screen.setup.as_mut())
                        {
                            setup.options.quality = quality;
                            cx.notify();
                        }
                    }))
            }),
        });
        let dialog = dialog.child(
            div()
                .id("screen-share-audio-row")
                .role(Role::CheckBox)
                .aria_label(audio_label.clone())
                .aria_toggled(if setup.options.share_audio {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .tab_index(0)
                .rounded(px(theme.radius_sm))
                .focus_visible(move |style| {
                    style.shadow(vec![BoxShadow {
                        color: theme.focus_ring.into(),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(3.),
                        inset: false,
                    }])
                })
                .cursor_pointer()
                .on_click({
                    let checked = setup.options.share_audio;
                    move |_, window, cx| toggle_audio(&!checked, window, cx)
                })
                .flex()
                .items_center()
                .gap(px(theme.space3))
                .child(
                    Checkbox::new("screen-share-audio", theme).checked(setup.options.share_audio),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(theme.space1))
                        .text_size(px(theme.font_size_sm))
                        .child(div().font_weight(FontWeight::BOLD).child(audio_label))
                        .child(description().child(t(if cfg!(windows) {
                            "voice.shareAudioWindowsWarning"
                        } else {
                            "voice.shareAudioWarning"
                        }))),
                ),
        );

        Some(
            dialog
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(theme.space2))
                        .flex_wrap()
                        .child(
                            Button::new("screen-share-cancel", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.cancel"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dismiss_screen_setup(window, cx)
                                })),
                        )
                        .child(
                            Button::new("screen-share-picker", theme)
                                .label(t(if cfg!(windows) {
                                    "voice.shareSelected"
                                } else {
                                    "voice.openSystemPicker"
                                }))
                                .disabled({
                                    #[cfg(windows)]
                                    {
                                        setup.options.selection.is_none()
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        false
                                    }
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_screen_setup(window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    #[cfg(windows)]
    fn screen_source_choices(
        &self,
        setup: &Setup,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = t("voice.chooseWhatToShare");
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(theme.space2))
            .child(div().font_weight(FontWeight::BOLD).child(title.clone()))
            .child(
                div()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(t("voice.chooseWhatToShareDescription")),
            );
        match &setup.sources {
            None => content = content.child(t("common.loading")),
            Some(Err(error)) => content = content.child(error.to_string()),
            Some(Ok(sources)) => {
                content = content.child(
                    div()
                        .id("screen-share-sources")
                        .role(Role::Group)
                        .aria_label(title)
                        .max_h(px(250.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_wrap()
                        .gap(px(theme.space2))
                        .children(sources.iter().enumerate().map(|(index, source)| {
                            let selection = source.selection;
                            Button::new(("screen-share-source", index), theme)
                                .variant(if setup.options.selection == Some(selection) {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .selected(setup.options.selection == Some(selection))
                                .label(source.name.clone())
                                .trailing(div().text_size(px(theme.font_size_xs)).child(t(
                                    match selection.kind {
                                        ScreenSourceKind::Screen => "voice.entireDisplay",
                                        ScreenSourceKind::Window => "voice.applicationWindow",
                                    },
                                )))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(setup) = this
                                        .call
                                        .as_mut()
                                        .and_then(|call| call.screen.setup.as_mut())
                                    {
                                        setup.options.selection = Some(selection);
                                        cx.notify();
                                    }
                                }))
                        })),
                );
            }
        }
        content
    }
}

#[derive(IntoElement)]
struct QualityChoices {
    theme: Theme,
    buttons: [Button; 3],
}

impl RenderOnce for QualityChoices {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let compact = window.viewport_size().width <= px(400.);
        div()
            .id("screen-share-quality")
            .role(Role::Group)
            .aria_label(t("voice.nativeScreenQuality"))
            .flex()
            .when(compact, |row| row.flex_col())
            .gap(px(self.theme.space2))
            .children(self.buttons.into_iter().map(|button| {
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .when(!compact, |cell| cell.flex_1())
                    .child(button)
            }))
    }
}
