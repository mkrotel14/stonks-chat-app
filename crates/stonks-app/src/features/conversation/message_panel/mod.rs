//! Read-only channel search and pins, with owned cancellable requests.
use crate::shared::images::Images;
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::sync::Arc;
use stonks_core::{
    connection::GatewayEvent,
    http::HttpPolicy,
    messages::{models::MessageSearchRequest, search},
};
use stonks_ui::{
    components::select::{IndexPath, SearchableVec},
    components::{
        avatar, Button, ButtonVariant, Feedback, FeedbackTone, SettingsInput, UiCloseButton,
        UiSelect, UiSelectEvent, UiSelectOption, UiSelectState,
    },
    i18n::{format_day_ms, format_time_ms, t, tp},
    Theme,
};
mod state;
use state::Results;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelMode {
    Search,
    Pins,
}
pub enum PanelEvent {
    Jump(String),
    Close,
}
struct Request {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub struct MessagePanel {
    policy: Arc<HttpPolicy>,
    images: Entity<Images>,
    focus: FocusHandle,
    mode: Option<PanelMode>,
    channel: Option<String>,
    input: Entity<InputState>,
    sort: Entity<UiSelectState>,
    last_query: String,
    last_sort: SharedString,
    results: Results,
    request: Option<Request>,
    generation: u64,
    loading: bool,
    error: Option<String>,
    retry_append: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<PanelEvent> for MessagePanel {}
impl MessagePanel {
    pub fn new(
        policy: Arc<HttpPolicy>,
        images: Entity<Images>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t("messages.searchChannel"))
                .validate(|value, _| value.len() <= 64)
        });
        let sort = cx.new(|cx| {
            UiSelectState::new(
                SearchableVec::new(vec![
                    UiSelectOption::new("Relevance", t("messages.sortRelevance")),
                    UiSelectOption::new("Latest", t("messages.sortLatest")),
                    UiSelectOption::new("Oldest", t("messages.sortOldest")),
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe(&images, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
                InputEvent::PressEnter { .. } => this.run(false, cx),
                InputEvent::Change => this.input_changed(cx),
                _ => {}
            }),
            cx.subscribe(&sort, |this, _, event, cx| {
                if matches!(event, UiSelectEvent::Confirm(_)) {
                    this.sort_changed(cx);
                }
            }),
        ];
        Self {
            policy,
            images,
            focus: cx.focus_handle(),
            mode: None,
            channel: None,
            input,
            sort,
            last_query: String::new(),
            last_sort: "Relevance".into(),
            results: Results::default(),
            request: None,
            generation: 0,
            loading: false,
            error: None,
            retry_append: false,
            _subscriptions: subscriptions,
        }
    }
    pub fn mode(&self) -> Option<PanelMode> {
        self.mode
    }
    pub fn open(
        &mut self,
        mode: PanelMode,
        channel_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset(window, cx);
        self.mode = Some(mode);
        self.channel = Some(channel_id);
        if mode == PanelMode::Pins {
            self.run(false, cx);
        }
        self.focus(window, cx);
        cx.notify();
    }
    /// Move focus into the open overlay before the next keyboard event.
    /// Pins focuses its container so Enter cannot reach the composer beneath it.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            Some(PanelMode::Search) => self.input.update(cx, |input, cx| input.focus(window, cx)),
            Some(PanelMode::Pins) => self.focus.focus(window, cx),
            None => {}
        }
    }
    pub fn set_channel(&mut self, channel_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.channel.as_ref() == Some(&channel_id) {
            return;
        }
        self.invalidate(cx);
        self.channel = Some(channel_id);
        self.last_query.clear();
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if self.mode == Some(PanelMode::Pins) {
            self.run(false, cx);
        }
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.mode.is_none() {
            return;
        }
        self.invalidate(cx);
        self.images.update(cx, |images, _| {
            images.set_sources("search", std::iter::empty())
        });
        self.mode = None;
        cx.emit(PanelEvent::Close);
        cx.notify();
    }
    /// Clear all identity-owned state without emitting a user dismissal.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate(cx);
        self.mode = None;
        self.images.update(cx, |images, _| {
            images.set_sources("search", std::iter::empty())
        });
        self.channel = None;
        self.last_query.clear();
        self.last_sort = "Relevance".into();
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.sort.update(cx, |sort, cx| {
            sort.set_selected_value(&SharedString::from("Relevance"), window, cx)
        });
    }
    fn input_changed(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        if self.last_query != value {
            self.last_query = value;
            self.invalidate(cx);
        }
    }
    fn sort_changed(&mut self, cx: &mut Context<Self>) {
        let value = self
            .sort
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| "Relevance".into());
        if self.last_sort != value {
            self.last_sort = value;
            self.invalidate(cx);
        }
    }
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.generation = self.generation.wrapping_add(1);
        self.results = Results::default();
        self.loading = false;
        self.error = None;
        cx.notify();
    }
    fn run(&mut self, append: bool, cx: &mut Context<Self>) {
        self.input_changed(cx);
        self.sort_changed(cx);
        if self.loading || (append && !self.results.has_more) {
            return;
        }
        let (Some(mode), Some(channel)) = (self.mode, self.channel.clone()) else {
            return;
        };
        let query = self.input.read(cx).value().trim().to_owned();
        if mode == PanelMode::Search && query.is_empty() {
            return;
        }
        let sort = if mode == PanelMode::Pins {
            "Latest".into()
        } else {
            self.sort
                .read(cx)
                .selected_value()
                .map(ToString::to_string)
                .unwrap_or_else(|| "Relevance".into())
        };
        if !append {
            self.request = None;
            self.generation = self.generation.wrapping_add(1);
            self.results = Results::default();
        }
        let before = append.then(|| self.results.cursor.clone()).flatten();
        if append && before.is_none() {
            return;
        }
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        self.retry_append = append;
        self.results.pending.clear();
        cx.notify();
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let request = MessageSearchRequest {
                channel_id: channel.clone(),
                query: (mode == PanelMode::Search).then_some(query),
                pinned: mode == PanelMode::Pins,
                before: before.clone(),
                sort,
            };
            let Ok(result) = spawn_core(Abortable::new(
                async move { search::search_messages(&policy, &request).await },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.mode != Some(mode)
                    || this.channel.as_ref() != Some(&channel)
                {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page) => this.results.merge(
                        page,
                        &channel,
                        before.as_deref(),
                        mode == PanelMode::Pins,
                    ),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        });
        self.request = Some(Request { abort, _task: task });
    }
    /// The host forwards session gateway events; edits arriving during a fetch
    /// are replayed over the page before it is made visible.
    pub fn apply_gateway_event(&mut self, event: &GatewayEvent, cx: &mut Context<Self>) {
        if self.mode.is_none()
            || event
                .data
                .get("channel")
                .and_then(serde_json::Value::as_str)
                != self.channel.as_deref()
        {
            return;
        }
        if !matches!(event.event_type.as_str(), "MessageDelete" | "MessageUpdate") {
            return;
        }
        if self.loading {
            self.results.pending.push(event.clone());
        }
        self.results
            .apply(event, self.mode == Some(PanelMode::Pins));
        cx.notify();
    }
}

impl Render for MessagePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let pins = self.mode == Some(PanelMode::Pins);
        let sources = self
            .results
            .messages
            .iter()
            .filter_map(|message| message.author_avatar_url.clone().map(|url| (url, 64)))
            .collect::<Vec<_>>();
        self.images
            .update(cx, |images, _| images.set_sources("search", sources));
        let mut panel = div()
            .id("message-panel")
            .flex()
            .flex_col()
            .h_full()
            .w(px(390_f32.min(
                (f32::from(window.viewport_size().width) - 24.).max(1.),
            )))
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.text)
            .text_size(px(theme.font_size_sm))
            .rounded_tl(px(theme.radius_lg))
            .rounded_bl(px(theme.radius_lg))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .min_h(px(66.))
                    .px(px(theme.space4))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(theme.space2))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(theme.font_size_sm))
                                    .child(t(if pins {
                                        "messages.pinned"
                                    } else {
                                        "messages.search"
                                    })),
                            )
                            .children((!self.results.messages.is_empty()).then(|| {
                                div()
                                    .px(px(theme.space1))
                                    .min_h(px(19.))
                                    .rounded(px(theme.radius_pill))
                                    .bg(theme.surface_interactive)
                                    .text_size(px(theme.font_size_xs))
                                    .text_color(theme.text_muted)
                                    .child(tp::<&str>(
                                        "messages.results",
                                        self.results.messages.len() as i64,
                                        &[],
                                    ))
                            })),
                    )
                    .child(
                        UiCloseButton::new("message-panel-close", theme)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            );
        if !pins {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .px(px(theme.space4))
                    .pb(px(theme.space4))
                    .child(SettingsInput::new(&self.input, theme))
                    .child(
                        div()
                            .flex()
                            .gap(px(theme.space2))
                            .child(div().flex_1().min_w_0().child(
                                UiSelect::new(&self.sort, theme).label(t("messages.sortLabel")),
                            ))
                            .child(
                                Button::new("message-search-submit", theme)
                                    .label(t("messages.searchAction"))
                                    .disabled(
                                        self.loading
                                            || self.input.read(cx).value().trim().is_empty(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.run(false, cx))),
                            ),
                    ),
            );
        }
        let mut results = div()
            .id("message-panel-results")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(theme.space3))
            .py(px(theme.space4))
            .gap(px(theme.space2));
        if self.loading && self.results.messages.is_empty() {
            results = results.child(
                div()
                    .text_color(theme.text_muted)
                    .child(t("common.loading")),
            );
        }
        if let Some(error) = &self.error {
            results = results
                .child(Feedback::new(theme, FeedbackTone::Error, error.clone()))
                .child(
                    Button::new("message-panel-retry", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("common.retry"))
                        .on_click(cx.listener(|this, _, _, cx| this.run(this.retry_append, cx))),
                );
        }
        if !self.loading
            && self.error.is_none()
            && self.results.messages.is_empty()
            && (pins || !self.input.read(cx).value().trim().is_empty())
        {
            results = results.child(
                div()
                    .px(px(theme.space2))
                    .py(px(theme.space4))
                    .text_size(px(theme.font_size_sm))
                    .text_color(theme.text_muted)
                    .child(t(if pins {
                        "messages.noPins"
                    } else {
                        "messages.noResults"
                    })),
            );
        }
        for message in &self.results.messages {
            let image = self.images.update(cx, |images, cx| {
                images.get(message.author_avatar_url.as_deref(), 64, cx)
            });
            let id = message.id.clone();
            let snippet = state::snippet(message);
            let timestamp = state::timestamp(&message.id)
                .map(|ms| format!("{} · {}", format_day_ms(ms), format_time_ms(ms, false)))
                .unwrap_or_default();
            results = results.child(
                Button::new(
                    ElementId::Name(format!("search-result-{}", message.id).into()),
                    theme,
                )
                .variant(ButtonVariant::SearchResult)
                .leading(
                    div()
                        .flex()
                        .w_full()
                        .items_start()
                        .gap(px(10.))
                        .child(avatar(&theme, &message.author_name, 32., image))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(theme.space1))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_baseline()
                                        .gap(px(theme.space2))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .text_size(px(theme.font_size_md))
                                                .child(message.author_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_color(theme.text_muted)
                                                .text_size(px(theme.font_size_xs))
                                                .child(timestamp),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(theme.text_muted)
                                        .text_size(px(theme.font_size_sm))
                                        .font_weight(FontWeight::NORMAL)
                                        .child(snippet),
                                ),
                        ),
                )
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PanelEvent::Jump(id.clone())))),
            );
        }
        if self.results.has_more {
            results = results.child(
                Button::new("message-panel-more", theme)
                    .variant(ButtonVariant::Secondary)
                    .label(t(if self.loading {
                        "common.loading"
                    } else {
                        "messages.loadMoreResults"
                    }))
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.run(true, cx))),
            );
        }
        panel
            .child(results)
            .focus_trap("message-panel-focus-trap", &self.focus)
    }
}
