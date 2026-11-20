//! One keyboard/pointer reveal state per original spoiler wrapper.
use super::{ClickRange, ClickTarget, InlineActions, SpoilerHover, StyledLine};
use crate::{theme::TRANSPARENT, Theme};
use gpui_kit::*;
use std::{collections::BTreeMap, ops::Range};

pub(super) struct SpanFocus {
    handle: FocusHandle,
    key: String,
    focused: bool,
    hovered: bool,
    callback: SpoilerHover,
    _subscriptions: Vec<Subscription>,
}
impl SpanFocus {
    fn new(
        key: String,
        callback: SpoilerHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let handle = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus_in(&handle, window, |this, _, cx| {
                this.focused = true;
                this.publish(cx);
            }),
            cx.on_focus_out(&handle, window, |this, _, _, cx| {
                this.focused = false;
                this.publish(cx);
            }),
        ];
        Self {
            handle,
            key,
            focused: false,
            hovered: false,
            callback,
            _subscriptions: subscriptions,
        }
    }
    fn publish(&self, cx: &mut Context<Self>) {
        (self.callback)(&self.key, self.focused || self.hovered, cx);
    }
    fn hover(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.hovered != value {
            self.hovered = value;
            self.publish(cx);
        }
    }
}
#[derive(Clone)]
pub(super) struct Span {
    pub range: Range<usize>,
    state: Entity<SpanFocus>,
}
pub(super) fn states(
    line: &StyledLine,
    callback: SpoilerHover,
    window: &mut Window,
    cx: &mut App,
) -> Vec<Span> {
    let mut groups: BTreeMap<String, Range<usize>> = BTreeMap::new();
    for item in &line.spoilers {
        if let ClickTarget::Spoiler(key) = &item.target {
            groups
                .entry(key.clone())
                .and_modify(|range| {
                    range.start = range.start.min(item.range.start);
                    range.end = range.end.max(item.range.end);
                })
                .or_insert(item.range.clone());
        }
    }
    groups
        .into_iter()
        .map(|(key, range)| {
            let state = window.use_keyed_state(
                ElementId::Name(format!("spoiler-focus-{key}").into()),
                cx,
                |window, cx| SpanFocus::new(key.clone(), callback.clone(), window, cx),
            );
            state.update(cx, |this, _| this.callback = callback.clone());
            Span { range, state }
        })
        .collect()
}
/// Preserve public semantic strings while preventing concealed content from
/// leaking through the accessibility label of the rendering projection.
pub(super) fn accessible(line: &StyledLine) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    let mut concealed = line.concealed.clone();
    concealed.sort_by_key(|range| range.start);
    for range in concealed {
        if range.end <= cursor {
            continue;
        }
        if range.start > cursor {
            output.push_str(&line.text[cursor..range.start]);
            output.push_str("…");
        } else if cursor == 0 {
            output.push_str("…");
        }
        cursor = range.end;
    }
    output.push_str(&line.text[cursor..]);
    output
}
pub(super) fn rectangles(layout: &TextLayout, range: &Range<usize>) -> Vec<Bounds<Pixels>> {
    let mut rectangles = Vec::new();
    let mut paragraph_start = 0;
    let mut visual_row = 0;
    let line_height = layout.line_height();
    let origin = layout.bounds().origin;
    for line in layout.line_layouts() {
        let unwrapped = &line.unwrapped_layout;
        let mut starts = vec![0];
        starts.extend(
            line.wrap_boundaries
                .iter()
                .map(|boundary| unwrapped.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index),
        );
        for (index, start) in starts.iter().copied().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(unwrapped.len);
            let from = range.start.max(paragraph_start + start);
            let to = range.end.min(paragraph_start + end);
            if from < to {
                let offset = unwrapped.x_for_index(start);
                let x = unwrapped.x_for_index(from - paragraph_start) - offset;
                let right = unwrapped.x_for_index(to - paragraph_start) - offset;
                rectangles.push(Bounds::new(
                    origin + point(x, line_height * visual_row as f32),
                    size((right - x).max(px(1.)), line_height),
                ));
            }
            visual_row += 1;
        }
        paragraph_start += unwrapped.len + 1;
    }
    rectangles
}
pub(super) fn prepaint(
    layout: &TextLayout,
    spans: Vec<Span>,
    targets: Vec<ClickRange>,
    actions: InlineActions,
    theme: Theme,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    for (group, span) in spans.into_iter().enumerate() {
        let handle = span.state.read(cx).handle.clone();
        for (index, rect) in rectangles(layout, &span.range).into_iter().enumerate() {
            let hover = span.state.clone();
            let target_layout = layout.clone();
            let targets = targets
                .iter()
                .map(|item| (item.range.clone(), item.target.clone()))
                .collect::<Vec<_>>();
            let actions = actions.clone();
            let click_handle = handle.clone();
            let focus = theme.focus_ring;
            let mut element = gpui_kit::base::Button::new(ElementId::Name(
                format!("spoiler-{group}-{index}").into(),
            ))
            .role(Role::Group)
            .accessibility_label(crate::i18n::t("messages.spoilerReveal"))
            .track_focus(&handle)
            .focusable(index == 0)
            .p_0()
            .w(rect.size.width)
            .h(rect.size.height)
            .bg(TRANSPARENT)
            .rounded(px(theme.radius_sm))
            .cursor_pointer()
            .focus_visible(move |style| {
                style.shadow(vec![BoxShadow {
                    color: focus.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .on_hover(move |value, _, cx| hover.update(cx, |this, cx| this.hover(*value, cx)))
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                window.focus(&click_handle, cx);
                if let Ok(index) = target_layout.index_for_position(event.position()) {
                    for (range, target) in &targets {
                        if range.contains(&index) {
                            match target {
                                ClickTarget::Url(url) => crate::external_links::request(url, window, cx),
                                ClickTarget::UserMention(id) => {
                                    if let Some(callback) = &actions.user {
                                        callback(id, window, cx);
                                    }
                                }
                                ClickTarget::ChannelMention(id) => {
                                    if let Some(callback) = &actions.channel {
                                        callback(id, window, cx);
                                    }
                                }
                                _ => {}
                            }
                            break;
                        }
                    }
                }
            })
            .into_any_element();
            element.layout_as_root(AvailableSpace::min_size(), window, cx);
            element.prepaint_at(rect.origin, window, cx);
            elements.push(element);
        }
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::accessible;
    use crate::{
        markdown::{styled_line, tokenize_inline, ClickTarget, MentionContext, RenderColors},
        Theme,
    };
    #[test]
    fn nested_styles_share_original_group_and_accessibility_conceals_contents() {
        let mentions = MentionContext {
            users: &[],
            channels: &[],
            roles: &[],
        };
        let colors = RenderColors::from_theme(&Theme::dark());
        let line = styled_line(
            &tokenize_inline("before ||one **two** three|| and ||four||"),
            &colors,
            &mentions,
            "m.0",
            |_| false,
        );
        let keys = line
            .spoilers
            .iter()
            .map(|item| match &item.target {
                ClickTarget::Spoiler(key) => key.as_str(),
                _ => panic!("spoiler target"),
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, ["m.0.s0", "m.0.s0", "m.0.s0", "m.0.s1"]);
        assert_eq!(accessible(&line), "before … and …");
        assert!(!accessible(&line).contains("two"));
        let revealed = styled_line(
            &tokenize_inline("before ||one **two** three|| and ||four||"),
            &colors,
            &mentions,
            "m.0",
            |key| key == "m.0.s0",
        );
        assert_eq!(accessible(&revealed), "before one two three and …");
    }
    #[test]
    fn adjacent_original_wrappers_keep_separate_group_ids() {
        let mentions = MentionContext {
            users: &[],
            channels: &[],
            roles: &[],
        };
        let colors = RenderColors::from_theme(&Theme::dark());
        let line = styled_line(
            &tokenize_inline("||one||||two||"),
            &colors,
            &mentions,
            "m",
            |_| false,
        );
        let keys = line
            .spoilers
            .iter()
            .map(|item| match &item.target {
                ClickTarget::Spoiler(key) => key.as_str(),
                _ => panic!("spoiler target"),
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, ["m.s0", "m.s1"]);
    }
}
