//! The same Twemoji sprite/index used by Tauri's EmojiGlyph.
use crate::Theme;
use gpui_kit::*;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

fn coordinates(emoji: &str) -> Option<[u16; 2]> {
    static INDEX: OnceLock<HashMap<String, [u16; 2]>> = OnceLock::new();
    let key = emoji
        .chars()
        .map(|character| format!("{:04X}", character as u32))
        .collect::<Vec<_>>()
        .join("-");
    INDEX
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../assets/emoji/twitter-index.json"))
                .expect("bundled emoji index is valid")
        })
        .get(&key)
        .copied()
}
fn sheet() -> Arc<Image> {
    thread_local! {static SHEET:Arc<Image>=Arc::new(Image::from_bytes(ImageFormat::Png,include_bytes!("../../assets/emoji/twitter-32.png").to_vec()));}
    SHEET.with(Clone::clone)
}

/// Longest artwork match preserves ZWJ, variation selectors and skin tones as
/// one inline object. Unsupported characters retain their original text.
pub fn emoji_spans(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    static PREFIXES: OnceLock<HashMap<char, Vec<String>>> = OnceLock::new();
    let prefixes = PREFIXES.get_or_init(|| {
        let index: HashMap<String, [u16; 2]> =
            serde_json::from_str(include_str!("../../assets/emoji/twitter-index.json"))
                .expect("bundled emoji index is valid");
        let mut prefixes: HashMap<char, Vec<String>> = HashMap::new();
        for key in index.keys() {
            let emoji = key
                .split('-')
                .map(|part| u32::from_str_radix(part, 16).ok().and_then(char::from_u32))
                .collect::<Option<String>>();
            if let Some(emoji) = emoji {
                if let Some(first) = emoji.chars().next() {
                    prefixes.entry(first).or_default().push(emoji);
                }
            }
        }
        for values in prefixes.values_mut() {
            values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        }
        prefixes
    });
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(first) = text[cursor..].chars().next() else {
            break;
        };
        if let Some(emoji) = prefixes.get(&first).and_then(|values| {
            values
                .iter()
                .find(|emoji| text[cursor..].starts_with(emoji.as_str()))
        }) {
            spans.push((cursor..cursor + emoji.len(), emoji.clone()));
            cursor += emoji.len();
        } else {
            cursor += first.len_utf8();
        }
    }
    spans
}

#[derive(IntoElement)]
pub struct EmojiGlyph {
    id: ElementId,
    emoji: SharedString,
    size: f32,
    custom: Option<Arc<RenderImage>>,
}
impl EmojiGlyph {
    pub fn new(id: impl Into<ElementId>, emoji: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            emoji: emoji.into(),
            size: 20.,
            custom: None,
        }
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    pub fn custom_image(mut self, image: Option<Arc<RenderImage>>) -> Self {
        self.custom = image;
        self
    }
}
impl RenderOnce for EmojiGlyph {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut glyph = div()
            .id(self.id)
            .role(Role::Image)
            .aria_label(self.emoji.clone())
            .relative()
            .size(px(self.size))
            .flex_shrink_0()
            .overflow_hidden();
        if let Some([x, y]) = coordinates(&self.emoji) {
            let scale = self.size / 32.;
            glyph = glyph.child(
                img(sheet())
                    .absolute()
                    .left(px(-(f32::from(x) * 34. + 1.) * scale))
                    .top(px(-(f32::from(y) * 34. + 1.) * scale))
                    .w(px(2108. * scale))
                    .h(px(2108. * scale)),
            );
        } else if let Some(image) = self.custom {
            glyph = glyph.child(img(image).size_full().object_fit(ObjectFit::Contain));
        } else {
            glyph = glyph
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(self.size))
                .line_height(relative(1.))
                .child(self.emoji);
        }
        glyph
    }
}

/// Source reaction-tooltip identity. Presence indicators are deliberately absent,
/// matching the source's plain24px UiAvatar rows.
#[derive(Clone)]
pub struct ReactionPerson {
    pub name: SharedString,
    pub avatar: Option<Arc<RenderImage>>,
}
pub fn emoji_shortcode(emoji: &str) -> String {
    static NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
    let key = emoji
        .chars()
        .map(|character| format!("{:04X}", character as u32))
        .collect::<Vec<_>>()
        .join("-");
    let name = NAMES
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../assets/emoji/twitter-shortcodes.json"))
                .expect("bundled emoji names are valid")
        })
        .get(&key)
        .map(String::as_str)
        .unwrap_or(emoji);
    format!(":{name}:")
}
/// The source's rich UiTooltip surface; shared by pointer and keyboard triggers.
pub fn reaction_tooltip(theme: &Theme, code: &str, people: &[ReactionPerson]) -> Stateful<Div> {
    let shadow = theme.shadow_sm;
    div()
        .id("reaction-tooltip")
        .role(Role::Tooltip)
        .flex()
        .flex_col()
        .gap(px(theme.space2))
        .p(px(theme.space2))
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_sm))
        .bg(theme.surface_raised)
        .text_color(theme.text)
        .text_size(px(theme.font_size_md))
        .line_height(relative(1.5))
        .shadow(vec![BoxShadow {
            color: shadow.color.into(),
            offset: point(px(0.), px(shadow.dy)),
            blur_radius: px(shadow.blur),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(div().font_weight(FontWeight::BOLD).child(code.to_owned()))
        .child(
            div()
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_xs))
                .child(crate::i18n::t("messages.reactedBy").to_uppercase()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(theme.space2))
                .children(people.iter().map(|person| {
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap(px(theme.space2))
                        .child(super::avatar(
                            theme,
                            &person.name,
                            24.,
                            person.avatar.clone(),
                        ))
                        .child(person.name.clone())
                })),
        )
}
#[derive(Clone, IntoElement)]
pub struct ReactionPill {
    id: ElementId,
    theme: Theme,
    emoji: String,
    count: usize,
    selected: bool,
    custom: Option<Arc<RenderImage>>,
    tooltip: SharedString,
    code: String,
    people: Vec<ReactionPerson>,
    disabled: bool,
    on_click: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    avatar_resolver: Option<std::rc::Rc<dyn Fn(usize, &mut App) -> Option<Arc<RenderImage>>>>,
}
impl ReactionPill {
    pub fn new(id: impl Into<ElementId>, theme: Theme, emoji: String, count: usize) -> Self {
        let code = emoji_shortcode(&emoji);
        let tooltip = emoji.clone().into();
        Self {
            id: id.into(),
            theme,
            emoji,
            count,
            selected: false,
            custom: None,
            tooltip,
            code,
            people: vec![],
            disabled: false,
            on_click: None,
            avatar_resolver: None,
        }
    }
    pub fn avatar_resolver(
        mut self,
        resolve: impl Fn(usize, &mut App) -> Option<Arc<RenderImage>> + 'static,
    ) -> Self {
        self.avatar_resolver = Some(std::rc::Rc::new(resolve));
        self
    }
    pub fn selected(mut self, value: bool) -> Self {
        self.selected = value;
        self
    }
    pub fn custom_image(mut self, value: Option<Arc<RenderImage>>) -> Self {
        self.custom = value;
        self
    }
    pub fn tooltip(mut self, value: impl Into<SharedString>) -> Self {
        self.tooltip = value.into();
        self
    }
    pub fn shortcode(mut self, value: impl Into<String>) -> Self {
        self.code = value.into();
        self
    }
    pub fn people(mut self, value: Vec<ReactionPerson>) -> Self {
        self.people = value;
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(std::rc::Rc::new(callback));
        self
    }
}
struct ReactionView {
    pill: ReactionPill,
    focus: FocusHandle,
    hovered: bool,
    popup_hovered: bool,
    focused: bool,
    open: bool,
    dismissed: bool,
    timer: Option<Task<()>>,
    bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    _focus: Vec<Subscription>,
}
impl ReactionView {
    fn new(pill: ReactionPill, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus_in(&focus, window, |this, _, cx| {
                this.focused = true;
                this.dismissed = false;
                this.open = true;
                this.timer = None;
                cx.notify();
            }),
            cx.on_focus_out(&focus, window, |this, _, _, cx| {
                this.focused = false;
                this.dismissed = false;
                if !this.hovered && !this.popup_hovered {
                    this.open = false;
                }
                cx.notify();
            }),
        ];
        Self {
            pill,
            focus,
            hovered: false,
            popup_hovered: false,
            focused: false,
            open: false,
            dismissed: false,
            timer: None,
            bounds: Default::default(),
            _focus: subscriptions,
        }
    }
    fn hover(&mut self, hovered: bool, popup: bool, cx: &mut Context<Self>) {
        if popup {
            self.popup_hovered = hovered;
        } else {
            self.hovered = hovered;
        }
        self.timer = None;
        if self.hovered || self.popup_hovered {
            if self.dismissed {
                return;
            }
            if self.open || self.focused {
                self.open = true;
                cx.notify();
                return;
            }
            self.timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
                this.update(cx, |this, cx| {
                    if this.hovered && !this.dismissed {
                        this.open = true;
                        cx.notify();
                    }
                })
                .ok();
            }));
        } else {
            self.dismissed = false;
            if !self.focused {
                self.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(100))
                        .await;
                    this.update(cx, |this, cx| {
                        if !this.hovered && !this.popup_hovered && !this.focused {
                            this.open = false;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
        }
    }
}
impl RenderOnce for ReactionPill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            ReactionView::new(self.clone(), window, cx)
        });
        state.update(cx, |view, cx| {
            let changed = view.pill.count != self.count
                || view.pill.selected != self.selected
                || view.pill.disabled != self.disabled
                || view.pill.code != self.code
                || view.pill.emoji != self.emoji
                || view.pill.theme.text != self.theme.text
                || view.pill.theme.surface_selected != self.theme.surface_selected
                || view.pill.custom.as_ref().map(|image| image.id)
                    != self.custom.as_ref().map(|image| image.id)
                || (view.open && self.avatar_resolver.is_some())
                || view.pill.people.len() != self.people.len()
                || view.pill.people.iter().zip(&self.people).any(|(old, new)| {
                    old.name != new.name
                        || old.avatar.as_ref().map(|image| image.id)
                            != new.avatar.as_ref().map(|image| image.id)
                });
            view.pill = self;
            if changed {
                cx.notify();
            }
        });
        state
    }
}
impl Render for ReactionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::ElementExt;
        let pill = self.pill.clone();
        let bounds = self.bounds.clone();
        let mut root = div()
            .id("reaction-tooltip-trigger")
            .relative()
            .on_prepaint(move |value, _, _| bounds.set(value))
            .on_hover(cx.listener(|this, value, _, cx| this.hover(*value, false, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.open {
                    this.open = false;
                    this.dismissed = true;
                    this.timer = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                super::Button::new("reaction-button", pill.theme)
                    .variant(super::ButtonVariant::Quiet)
                    .track_focus(&self.focus)
                    .accessibility_label(pill.tooltip)
                    .selected(pill.selected)
                    .disabled(pill.disabled)
                    .leading(
                        EmojiGlyph::new("reaction-art", pill.emoji)
                            .size(18.)
                            .custom_image(pill.custom),
                    )
                    .label(pill.count.to_string())
                    .on_click(cx.listener(move |this, event, window, cx| {
                        this.open = false;
                        this.dismissed = true;
                        this.timer = None;
                        cx.stop_propagation();
                        if let Some(callback) = &pill.on_click {
                            callback(event, window, cx);
                        }
                        cx.notify();
                    })),
            );
        if self.open && !self.dismissed {
            let mut people = self.pill.people.clone();
            if let Some(resolve) = &self.pill.avatar_resolver {
                for (index, person) in people.iter_mut().enumerate() {
                    person.avatar = resolve(index, cx).or_else(|| person.avatar.clone());
                }
            }
            let anchor = self.bounds.get();
            root = root.child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomLeft)
                        .position(point(anchor.left(), anchor.top() - px(8.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            reaction_tooltip(&self.pill.theme, &self.pill.code, &people)
                                .id("reaction-tooltip-content")
                                .on_hover(
                                    cx.listener(|this, value, _, cx| this.hover(*value, true, cx)),
                                ),
                        ),
                )
                .with_priority(1000),
            );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::{coordinates, emoji_shortcode};
    #[test]
    fn same_source_artwork_covers_variation_tones_and_zwj_sequences() {
        for emoji in ["😺", "💀", "👍🏽", "👨‍👩‍👧‍👦", "🇧🇷", "1️⃣"]
        {
            assert!(coordinates(emoji).is_some(), "{emoji}");
        }
        assert_eq!(coordinates("❤️"), coordinates("❤"));
        assert!(coordinates("not an emoji").is_none());
    }
    #[test]
    fn shortcode_labels_match_source_artwork_names_and_unknown_fallback() {
        assert_eq!(emoji_shortcode("💀"), ":skull:");
        assert_eq!(emoji_shortcode("😺"), ":smiley_cat:");
        assert_eq!(emoji_shortcode("👍🏽"), ":+1:");
        assert_eq!(emoji_shortcode("❤️"), emoji_shortcode("❤"));
        assert_eq!(emoji_shortcode("unknown"), ":unknown:");
    }
}
