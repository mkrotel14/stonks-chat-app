//! Source EmojiPicker.tsx, shared by composition and reactions. Artwork and
//! searchable catalog are the same bundled Twemoji data as the Tauri UI.
use super::{
    Button, ButtonVariant, EmojiGlyph, SettingsInput, UiLink, UiSelect, UiSelectEvent,
    UiSelectOption, UiSelectState,
};
use crate::Theme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{rc::Rc, sync::Arc};
mod catalog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmoji {
    pub id: String,
    pub name: String,
    pub server_name: Option<String>,
    pub url: Option<String>,
}
impl CustomEmoji {
    fn label(&self) -> String {
        format!(
            ":{}:{}",
            self.name,
            self.server_name
                .as_ref()
                .map(|name| format!(" · {name}"))
                .unwrap_or_default()
        )
    }
}
pub enum EmojiPickerEvent {
    Select { value: String, custom: bool },
}
type ImageProvider = Rc<dyn Fn(&str, &mut App) -> Option<Arc<RenderImage>>>;
pub struct UiEmojiPicker {
    search: Entity<InputState>,
    categories: Entity<UiSelectState>,
    category: String,
    limit: usize,
    custom: Vec<CustomEmoji>,
    disabled: bool,
    images: Option<ImageProvider>,
    scroll: ScrollHandle,
    custom_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<EmojiPickerEvent> for UiEmojiPicker {}
impl UiEmojiPicker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search emoji"));
        let category = "Smileys & Emotion".to_string();
        let options: Vec<_> = catalog::categories()
            .into_iter()
            .map(|name| UiSelectOption::new(name.clone(), name))
            .collect();
        let index = catalog::categories()
            .iter()
            .position(|name| name == &category)
            .unwrap_or(0);
        let categories = cx.new(|cx| {
            UiSelectState::new(
                options.into(),
                Some(super::select::IndexPath::default().row(index)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.limit = 192;
                    this.scroll.set_offset(point(px(0.), px(0.)));
                    cx.notify();
                }
            }),
            cx.subscribe_in(&categories, window, |this, _, event, window, cx| {
                if let UiSelectEvent::Confirm(Some(value)) = event {
                    this.category = value.to_string();
                    this.search
                        .update(cx, |search, cx| search.set_value("", window, cx));
                    this.limit = 192;
                    this.scroll.set_offset(point(px(0.), px(0.)));
                    cx.notify();
                }
            }),
        ];
        Self {
            search,
            categories,
            category,
            limit: 192,
            custom: Vec::new(),
            disabled: false,
            images: None,
            scroll: ScrollHandle::new(),
            custom_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }
    pub fn set_custom(&mut self, custom: Vec<CustomEmoji>, cx: &mut Context<Self>) {
        if self.custom != custom {
            self.custom = custom;
            cx.notify();
        }
    }
    pub fn set_image_provider(
        &mut self,
        provider: impl Fn(&str, &mut App) -> Option<Arc<RenderImage>> + 'static,
    ) {
        self.images = Some(Rc::new(provider));
    }
    pub fn set_disabled(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.disabled != value {
            self.disabled = value;
            cx.notify();
        }
    }
    pub fn refresh_images(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }
    pub fn search_focus(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.category = "Smileys & Emotion".into();
        let index = catalog::categories()
            .iter()
            .position(|name| name == &self.category)
            .unwrap_or(0);
        self.categories.update(cx, |select, cx| {
            select.set_selected_index(
                Some(super::select::IndexPath::default().row(index)),
                window,
                cx,
            )
        });
        self.limit = 192;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.custom_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
}
impl Render for UiEmojiPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let query = self.search.read(cx).value().trim().to_lowercase();
        let matches = catalog::matches(&query, &self.category);
        let visible = matches.iter().take(self.limit).copied().collect::<Vec<_>>();
        let custom: Vec<_> = self
            .custom
            .iter()
            .filter(|emoji| query.is_empty() || emoji.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let images = self.images.clone();
        let custom_rows: Vec<_> = custom
            .chunks(8)
            .enumerate()
            .map(|(row, entries)| {
                let cells: Vec<_> = (0..8)
                    .map(|column| {
                        let Some(emoji) = entries.get(column) else {
                            return div().flex_1().min_w_0().into_any_element();
                        };
                        let id = emoji.id.clone();
                        let image = emoji
                            .url
                            .as_deref()
                            .and_then(|url| images.as_ref().and_then(|provider| provider(url, cx)));
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_start()
                            .child(
                                Button::new(("picker-custom", row * 8 + column), theme)
                                    .variant(ButtonVariant::Icon)
                                    .tooltip(emoji.label())
                                    .disabled(self.disabled)
                                    .leading(
                                        EmojiGlyph::new(
                                            "custom-glyph",
                                            format!(":{}:", emoji.name),
                                        )
                                        .size(24.)
                                        .custom_image(image),
                                    )
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(EmojiPickerEvent::Select {
                                            value: id.clone(),
                                            custom: true,
                                        })
                                    })),
                            )
                            .into_any_element()
                    })
                    .collect();
                div().flex().gap(px(theme.space1)).children(cells)
            })
            .collect();
        let unicode_rows: Vec<_> = visible
            .chunks(8)
            .map(|entries| {
                let cells: Vec<_> = (0..8)
                    .map(|column| {
                        let Some(&index) = entries.get(column) else {
                            return div().flex_1().min_w_0().into_any_element();
                        };
                        let entry = &catalog::entries()[index];
                        let emoji = entry.emoji.clone();
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_start()
                            .child(
                                Button::new(("picker-emoji", index), theme)
                                    .variant(ButtonVariant::Icon)
                                    .tooltip(entry.label.clone())
                                    .disabled(self.disabled)
                                    .leading(
                                        EmojiGlyph::new("unicode-glyph", entry.emoji.clone())
                                            .size(24.),
                                    )
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(EmojiPickerEvent::Select {
                                            value: emoji.clone(),
                                            custom: false,
                                        })
                                    })),
                            )
                            .into_any_element()
                    })
                    .collect();
                div().flex().gap(px(theme.space1)).children(cells)
            })
            .collect();
        div()
            .id("emoji-picker-content")
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(theme.space2))
            .child(
                div()
                    .flex()
                    .gap(px(theme.space2))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(SettingsInput::new(&self.search, theme).label("Search emoji")),
                    )
                    .child(
                        div()
                            .w(px(125.))
                            .flex_shrink_0()
                            .child(UiSelect::new(&self.categories, theme).label("Emoji category")),
                    ),
            )
            .when(!custom.is_empty(), |pane| {
                pane.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(theme.space1))
                        .child(
                            div()
                                .text_size(px(theme.font_size_xs))
                                .text_color(theme.text_muted)
                                .child("SERVER EMOJI"),
                        )
                        .child(
                            div()
                                .id("emoji-picker-custom-grid")
                                .flex()
                                .flex_col()
                                .gap(px(theme.space1))
                                .max_h(px(72.))
                                .p(px(2.))
                                .overflow_y_scroll()
                                .track_scroll(&self.custom_scroll)
                                .children(custom_rows),
                        ),
                )
            })
            .child(
                div()
                    .id("emoji-picker-grid")
                    .flex()
                    .flex_col()
                    .gap(px(theme.space1))
                    .max_h(px(238.))
                    .p(px(2.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(unicode_rows),
            )
            .when(matches.len() > self.limit, |pane| {
                pane.child(
                    Button::new("emoji-show-more", theme)
                        .variant(ButtonVariant::Quiet)
                        .label(format!("Show more ({})", matches.len() - self.limit))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.limit = this.limit.saturating_add(192);
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .text_size(px(theme.font_size_xs))
                    .child(UiLink::new(
                        "emoji-attribution",
                        theme,
                        "Twemoji · CC-BY 4.0",
                        "https://github.com/jdecked/twemoji",
                    )),
            )
    }
}
