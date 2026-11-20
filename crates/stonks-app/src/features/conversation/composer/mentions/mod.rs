//! Tauri composer mention completion, editor bindings, and wire spans.
use super::Composer;
use gpui_kit::*;
use model::{Candidate, Kind};
use std::ops::Range;
use stonks_core::connection::{AppSnapshot, ClientChannel};
use stonks_ui::{
    components::{avatar, Button, ButtonVariant},
    Theme,
};

mod model;
pub(super) use model::{reconcile, replace_ranges, serialize, DraftMention};

#[derive(Default)]
pub(super) struct Mentions {
    candidates: Vec<Candidate>,
    pub open: bool,
    pub selected: usize,
    scroll: ScrollHandle,
}

impl Composer {
    pub(crate) fn sync_mentions(
        &mut self,
        channel: &ClientChannel,
        snapshot: &AppSnapshot,
        autumn: Option<&str>,
        everyone: bool,
        roles: bool,
        cx: &mut Context<Self>,
    ) {
        let candidates = model::candidates(channel, snapshot, autumn, everyone, roles);
        if candidates != self.mentions.candidates {
            self.mentions.candidates = candidates;
            self.mentions.selected = 0;
            cx.notify();
        }
    }

    fn mention_completion(&self, cx: &App) -> Option<(Range<usize>, Vec<&Candidate>)> {
        if !self.mentions.open || self.state.active_channel().is_none() || self.state.sending_here()
        {
            return None;
        }
        let input = self.input.read(cx);
        let caret = input.selected_range().start;
        if self
            .state
            .draft_mentions()
            .iter()
            .any(|mention| mention.range.start < caret && caret <= mention.range.end)
        {
            return None;
        }
        model::complete(input.value().as_ref(), caret, &self.mentions.candidates)
    }

    pub(super) fn mention_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_focused(window, cx)
            || key.modifiers.control
            || key.modifiers.platform
            || key.modifiers.alt
        {
            return;
        }
        let Some((_, matches)) = self.mention_completion(cx) else {
            return;
        };
        let count = matches.len();
        let selected = if self.mentions.selected < count {
            self.mentions.selected
        } else {
            0
        };
        match key.key.as_str() {
            "up" => {
                self.mentions.selected = (selected + count - 1) % count;
                self.mentions.scroll.scroll_to_item(self.mentions.selected);
            }
            "down" => {
                self.mentions.selected = (selected + 1) % count;
                self.mentions.scroll.scroll_to_item(self.mentions.selected);
            }
            "escape" => self.mentions.open = false,
            "tab" | "enter" if key.key == "tab" || !key.modifiers.shift => {
                let token = matches[selected].token.clone();
                self.choose_mention(&token, window, cx);
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }

    fn choose_mention(&mut self, token: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.state.edit(self.input.read(cx).value().to_string());
        let Some((range, matches)) = self.mention_completion(cx) else {
            return;
        };
        let Some(candidate) = matches
            .into_iter()
            .find(|candidate| candidate.token == token)
            .cloned()
        else {
            return;
        };
        let label = candidate.display();
        let inserted = format!("{label} ");
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range.clone(), cx);
            input.replace(inserted.clone(), window, cx);
            input.focus(window, cx);
        });
        let text = self.input.read(cx).value().to_string();
        let mention = (candidate.kind != Kind::Music).then(|| DraftMention {
            range: range.start..range.start + label.len(),
            label,
            token: candidate.token,
        });
        self.spellcheck.invalidate();
        self.state
            .insert_mention(text, range, inserted.len(), mention);
        self.mentions.open = false;
        self.mentions.selected = 0;
        self.changed(cx);
        cx.notify();
    }

    pub(super) fn mention_popup(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.editor_focused(window, cx) {
            return None;
        }
        let (_, matches) = self.mention_completion(cx)?;
        let matches: Vec<_> = matches.into_iter().cloned().collect();
        let selected = if self.mentions.selected < matches.len() {
            self.mentions.selected
        } else {
            0
        };
        let rows = matches
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let label = candidate.display();
                let glyph = match candidate.kind {
                    Kind::Role => "●",
                    Kind::Music => "♫",
                    _ => "@",
                };
                let tint = candidate
                    .colour
                    .as_deref()
                    .and_then(|value| {
                        let hex = value.strip_prefix('#')?;
                        (hex.len() == 6)
                            .then(|| u32::from_str_radix(hex, 16).ok().map(rgb))
                            .flatten()
                    })
                    .unwrap_or(theme.text);
                let image = self.mention_image(candidate.image.as_deref(), cx);
                let icon = if image.is_some() {
                    avatar(theme, &label, 30., image)
                } else {
                    div()
                        .size(px(30.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(theme.radius_pill))
                        .bg(theme.surface_interactive)
                        .text_color(tint)
                        .text_size(px(theme.font_size_md))
                        .child(glyph)
                };
                div()
                    .id(("mention-option", index))
                    .flex_shrink_0()
                    .role(Role::ListBoxOption)
                    .aria_selected(index == selected)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.mentions.selected != index {
                            this.mentions.selected = index;
                            cx.notify();
                        }
                    }))
                    .child(
                        Button::new(("composer-mention", index), *theme)
                            .variant(ButtonVariant::Nav)
                            .selected(index == selected)
                            .accessibility_label(label.clone())
                            .leading(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(theme.space2))
                                    .min_w_0()
                                    .child(icon)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .min_w_0()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .font_weight(FontWeight::BOLD)
                                                    .child(label),
                                            )
                                            .children(candidate.detail.map(|detail| {
                                                div()
                                                    .truncate()
                                                    .text_size(px(theme.font_size_xs))
                                                    .text_color(theme.text_muted)
                                                    .child(detail)
                                            })),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_mention(&candidate.token, window, cx)
                            })),
                    )
            })
            .collect::<Vec<_>>();
        Some(
            div()
                .absolute()
                .left(px(44.))
                .right(px(44.))
                .bottom(relative(1.))
                .pb(px(8.))
                .child(
                    deferred(
                        div()
                            .id("composer-mention-completions")
                            .role(Role::ListBox)
                            .aria_label("Mention suggestions")
                            .occlude()
                            .flex()
                            .flex_col()
                            .max_h(px(310.))
                            .overflow_y_scroll()
                            .track_scroll(&self.mentions.scroll)
                            .p(px(6.))
                            .rounded(px(theme.radius_md))
                            .border_1()
                            .border_color(theme.outline)
                            .bg(theme.surface_raised)
                            .shadow(vec![BoxShadow {
                                color: theme.shadow_md.color.into(),
                                offset: point(px(0.), px(theme.shadow_md.dy)),
                                blur_radius: px(theme.shadow_md.blur),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .children(rows),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }
}
