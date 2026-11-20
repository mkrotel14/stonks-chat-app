//! Server selection screen — mirrors
//! `src/features/server-selection/components/ServerPicker.tsx`: deployment
//! address field, official shortcut, probe + save, events for the host.

use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::server::{ServerSelection, OFFICIAL_DEPLOYMENT};
use stonks_ui::components::{Button, ButtonVariant, Feedback, FeedbackTone};
use stonks_ui::Theme;

use crate::runtime::spawn_core;

pub enum ServerPickerEvent {
    Selected(ServerSelection),
    Cancelled,
}

pub struct ServerPicker {
    config_path: PathBuf,
    url: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    _subscription: Subscription,
}

impl EventEmitter<ServerPickerEvent> for ServerPicker {}

impl ServerPicker {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        config_path: PathBuf,
        current: Option<&ServerSelection>,
    ) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://chat.example.com"));
        if let Some(current) = current {
            url.update(cx, |state, cx| {
                state.set_value(current.deployment_url.clone(), window, cx);
            });
        }
        let subscription = cx.subscribe_in(&url, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.connect(cx);
            }
        });
        Self {
            config_path,
            url,
            busy: false,
            error: None,
            _subscription: subscription,
        }
    }

    /// Re-arm the form each time the picker opens: clear busy/error and
    /// prefill the current deployment URL.
    pub fn reset(
        &mut self,
        current: Option<&ServerSelection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.error = None;
        let value = current
            .map(|server| server.deployment_url.clone())
            .unwrap_or_default();
        self.url.update(cx, |state, cx| {
            state.set_value(value, window, cx);
        });
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().to_string();
        self.connect_with(url, cx);
    }

    fn connect_with(&mut self, url: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let config_path = self.config_path.clone();
        cx.spawn(async move |this, cx| {
            let result = spawn_core(async move {
                let selection = stonks_core::server::probe_server(&url).await?;
                stonks_core::server::save_selection(&config_path, &selection)?;
                Ok::<_, stonks_core::error::CommandError>(selection)
            })
            .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(selection) => cx.emit(ServerPickerEvent::Selected(selection)),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn heading(theme: &Theme, title: String, description: String) -> Div {
        div()
            .mb(px(theme.space4))
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .mb(px(theme.space3))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(theme.font_size_md))
                    .text_color(theme.text_muted)
                    .child(description),
            )
    }
}

impl Render for ServerPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        div()
            .flex()
            .flex_col()
            .w(px(380.))
            .child(Self::heading(
                &theme,
                "Server settings".into(),
                "Official or self-hosted Stoat address".into(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(Input::new(&self.url))
                    .when_some(self.error.clone(), |form, error| {
                        form.child(Feedback::new(theme, FeedbackTone::Error, error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                Button::new("use-official", theme)
                                    .variant(ButtonVariant::Secondary)
                                    .label("Use official Stoat")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.connect_with(OFFICIAL_DEPLOYMENT.into(), cx);
                                    })),
                            )
                            .child(
                                Button::new("connect", theme)
                                    .variant(ButtonVariant::Primary)
                                    .label(if self.busy { "Checking…" } else { "Connect" })
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                            ),
                    )
                    .child(
                        Button::new("picker-back", theme)
                            .variant(ButtonVariant::Quiet)
                            .label("← Back")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(ServerPickerEvent::Cancelled);
                            })),
                    ),
            )
    }
}
