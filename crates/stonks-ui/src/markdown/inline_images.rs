//! Inline-object rendering projection. Semantic strings/tokenization stay intact.
//! Blank-font glyph advances enter native shaping before wrapping; artwork is
//! prepainted at the resulting glyph positions rather than placed over text.
use super::*;
use crate::components::emoji::{emoji_spans, EmojiGlyph};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{collections::HashSet, rc::Rc};

pub const FONT_UNITS: f32 = 2048.;
pub const OBJECT_BASE: u32 = 0xDFFF;
/// Installed GPUI applies family overrides only to complete existing runs;
/// it does not split a run at override boundaries. Split every style/font
/// boundary explicitly so each inline object reaches the native shaper.
pub fn inline_text_runs(
    base: &TextStyle,
    len: usize,
    highlights: &[(Range<usize>, HighlightStyle)],
    overrides: &[(Range<usize>, SharedString)],
) -> Vec<TextRun> {
    let mut boundaries = std::collections::BTreeSet::from([0, len]);
    for (range, _) in highlights {
        boundaries.insert(range.start);
        boundaries.insert(range.end);
    }
    for (range, _) in overrides {
        boundaries.insert(range.start);
        boundaries.insert(range.end);
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    boundaries
        .windows(2)
        .filter_map(|pair| {
            let start = pair[0];
            let end = pair[1];
            if start >= end || end > len {
                return None;
            }
            let mut style = base.clone();
            for (range, highlight) in highlights {
                if start >= range.start && end <= range.end {
                    style = style.highlight(*highlight);
                }
            }
            for (range, family) in overrides {
                if start >= range.start && end <= range.end {
                    style.font_family = family.clone();
                }
            }
            Some(style.to_run(end - start))
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmoji {
    pub id: Option<String>,
    pub name: Option<String>,
    pub raw: String,
}
#[derive(Clone)]
pub struct InlineEmoji {
    pub label: String,
    pub image: Option<ImageSource>,
}
pub type CustomEmojiResolver = Rc<dyn Fn(&CustomEmoji, bool, &mut App) -> Option<InlineEmoji>>;
#[derive(Clone, Default)]
pub struct InlineMedia {
    pub custom: Option<CustomEmojiResolver>,
}
#[derive(Clone, Debug)]
pub(super) enum Kind {
    Unicode { value: String, size: f32 },
    Custom(CustomEmoji),
    Code { value: String, flags: RunFlags },
}
#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub range: Range<usize>,
    pub kind: Kind,
    pub hidden: bool,
}

pub(super) fn collect(line: &mut StyledLine, run: &InlineRun, range: Range<usize>, hidden: bool) {
    if run.flags.code {
        line.images.push(Candidate {
            range: range.clone(),
            kind: Kind::Code {
                value: line.text[range].to_owned(),
                flags: run.flags,
            },
            hidden,
        });
        return;
    }
    if let Payload::CustomEmoji { id, name, raw } = &run.payload {
        line.images.push(Candidate {
            range,
            kind: Kind::Custom(CustomEmoji {
                id: id.clone(),
                name: name.clone(),
                raw: raw.clone(),
            }),
            hidden,
        });
        return;
    }
    if run.payload != Payload::None {
        return;
    }
    let text = &line.text[range.clone()];
    let spans = emoji_spans(text);
    let mut remainder = String::new();
    let mut cursor = 0;
    for (span, _) in &spans {
        remainder.push_str(&text[cursor..span.start]);
        cursor = span.end;
    }
    remainder.push_str(&text[cursor..]);
    let size = if !spans.is_empty() && spans.len() <= 3 && remainder.trim().is_empty() {
        32.
    } else {
        20.
    };
    for (span, value) in spans {
        line.images.push(Candidate {
            range: range.start + span.start..range.start + span.end,
            kind: Kind::Unicode { value, size },
            hidden,
        });
    }
}
/// Matching the render's block/run keys keeps concealed custom images out of
/// image-source retention and out of the remote resolver altogether.
pub fn custom_emoji_requests(
    content: &str,
    id: &str,
    revealed: &HashSet<String>,
) -> Vec<CustomEmoji> {
    let mut lines = Vec::new();
    for (index, block) in parse_blocks(content).into_iter().enumerate() {
        let key = format!("{id}.{index}");
        match block {
            Block::Paragraph(lines_) | Block::Quote(lines_) => lines.push((key, lines_.join("\n"))),
            Block::Heading { text, .. } => lines.push((key, text)),
            Block::List { items, .. } => {
                for (index, text) in items.into_iter().enumerate() {
                    lines.push((format!("{key}.{index}"), text));
                }
            }
            Block::Table { headers, rows } => {
                for (row, cells) in std::iter::once(headers).chain(rows).enumerate() {
                    for (column, text) in cells.into_iter().enumerate() {
                        lines.push((format!("{key}.{row}.{column}"), text));
                    }
                }
            }
            Block::Code { .. } => {}
        }
    }
    lines
        .into_iter()
        .flat_map(|(key, text)| {
            tokenize_inline(&text)
                .into_iter()
                .enumerate()
                .filter_map(move |(index, run)| {
                    if run.flags.code
                        || (run.flags.spoiler
                            && !revealed.contains(&spoiler_key(&key, &run.flags, index)))
                    {
                        return None;
                    }
                    match run.payload {
                        Payload::CustomEmoji { id, name, raw } => {
                            Some(CustomEmoji { id, name, raw })
                        }
                        _ => None,
                    }
                })
        })
        .collect()
}

#[derive(Clone)]
struct CodePaint {
    text: String,
    font: Font,
    font_size: f32,
    padding_left: f32,
    baseline: f32,
    flags: RunFlags,
}
#[derive(Clone)]
struct Object {
    original: Range<usize>,
    projected: Range<usize>,
    label: String,
    unicode: Option<String>,
    image: Option<ImageSource>,
    size: f32,
    margin: f32,
    reserved: f32,
    glyph: char,
    custom: bool,
    code: Option<CodePaint>,
    hidden: bool,
}
struct Projection {
    text: String,
    objects: Vec<Object>,
}
impl Projection {
    fn new(
        line: &StyledLine,
        font_size: f32,
        media: &InlineMedia,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let mut objects = Vec::new();
        for candidate in &line.images {
            if let Kind::Code { value, flags } = &candidate.kind {
                let code_size = font_size * 0.88;
                let padding = code_size * 0.32;
                let mut run = window.text_style().to_run(value.len());
                run.font.family = "monospace".into();
                if flags.bold {
                    run.font.weight = FontWeight(800.);
                }
                if flags.italic {
                    run.font.style = FontStyle::Italic;
                }
                let shaped = window.text_system().shape_line(
                    value.clone().into(),
                    px(code_size),
                    &[run.clone()],
                    None,
                );
                let mut boundaries = shaped
                    .runs
                    .iter()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index))
                    .collect::<Vec<_>>();
                boundaries.push(0);
                boundaries.push(value.len());
                boundaries.sort_unstable();
                boundaries.dedup();
                let baseline =
                    (px(code_size * 1.5) - shaped.ascent - shaped.descent) / 2. + shaped.ascent;
                for (pair_index, pair) in boundaries.windows(2).enumerate() {
                    let start = pair[0];
                    let end = pair[1];
                    if start == end
                        || !value.is_char_boundary(start)
                        || !value.is_char_boundary(end)
                    {
                        continue;
                    }
                    let left = if pair_index == 0 { padding } else { 0. };
                    let right = if end == value.len() { padding } else { 0. };
                    let width = f32::from(shaped.x_for_index(end) - shaped.x_for_index(start))
                        .abs()
                        + left
                        + right;
                    let units = (width / font_size * FONT_UNITS).round().clamp(1., 6400.) as u32;
                    let Some(glyph) = char::from_u32(OBJECT_BASE + units) else {
                        continue;
                    };
                    let code = CodePaint {
                        text: value[start..end].to_owned(),
                        font: run.font.clone(),
                        font_size: code_size,
                        padding_left: left,
                        baseline: f32::from(baseline),
                        flags: *flags,
                    };
                    objects.push(Object {
                        original: candidate.range.start + start..candidate.range.start + end,
                        projected: 0..0,
                        label: code.text.clone(),
                        unicode: None,
                        image: None,
                        size: code_size * 1.5,
                        margin: 0.,
                        reserved: units as f32 / FONT_UNITS * font_size,
                        glyph,
                        custom: false,
                        code: Some(code),
                        hidden: candidate.hidden,
                    });
                }
                continue;
            }
            let (label, unicode, image, size, margin, custom) = match &candidate.kind {
                Kind::Unicode { value, size } => {
                    (value.clone(), Some(value.clone()), None, *size, 0., false)
                }
                Kind::Custom(request) => {
                    let Some(emoji) = media
                        .custom
                        .as_ref()
                        .and_then(|resolve| resolve(request, !candidate.hidden, cx))
                    else {
                        continue;
                    };
                    (
                        emoji.label,
                        None,
                        emoji.image,
                        font_size * 1.45,
                        font_size * 0.06,
                        true,
                    )
                }
                Kind::Code { .. } => continue,
            };
            let units = ((size + margin * 2.) / font_size * FONT_UNITS)
                .round()
                .clamp(1., 6400.) as u32;
            let Some(glyph) = char::from_u32(OBJECT_BASE + units) else {
                continue;
            };
            objects.push(Object {
                original: candidate.range.clone(),
                projected: 0..0,
                label,
                unicode,
                image,
                size,
                margin,
                reserved: units as f32 / FONT_UNITS * font_size,
                glyph,
                custom,
                code: None,
                hidden: candidate.hidden,
            });
        }
        Self::replace(&line.text, objects)
    }
    fn replace(semantic: &str, mut objects: Vec<Object>) -> Self {
        let mut text = String::new();
        let mut cursor = 0;
        for object in &mut objects {
            text.push_str(&semantic[cursor..object.original.start]);
            let start = text.len();
            text.push(object.glyph);
            object.projected = start..text.len();
            cursor = object.original.end;
        }
        text.push_str(&semantic[cursor..]);
        Self { text, objects }
    }
    fn offset(&self, offset: usize, end: bool) -> usize {
        let mut delta = 0isize;
        for object in &self.objects {
            if offset <= object.original.start {
                break;
            }
            if offset < object.original.end {
                return if end {
                    object.projected.end
                } else {
                    object.projected.start
                };
            }
            delta += object.projected.len() as isize - object.original.len() as isize;
        }
        offset.saturating_add_signed(delta)
    }
    fn range(&self, range: &Range<usize>) -> Range<usize> {
        self.offset(range.start, false)..self.offset(range.end, true)
    }
}
#[derive(Clone, IntoElement)]
pub(super) struct InlineText {
    pub theme: crate::Theme,
    pub line: StyledLine,
    pub key: String,
    pub hover: SpoilerHover,
    pub actions: InlineActions,
    pub media: InlineMedia,
}
impl RenderOnce for InlineText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let font_size =
            f32::from(window.text_style().font_size.to_pixels(window.rem_size())).max(1.);
        let accessible = super::spoiler::accessible(&self.line);
        let mut spans = super::spoiler::states(&self.line, self.hover.clone(), window, cx);
        let projection = Projection::new(&self.line, font_size, &self.media, window, cx);
        for span in &mut spans {
            span.range = projection.range(&span.range);
        }
        let overlay_targets = self
            .line
            .clickable
            .iter()
            .map(|item| ClickRange {
                range: projection.range(&item.range),
                target: item.target.clone(),
            })
            .collect::<Vec<_>>();
        let overlay_actions = self.actions.clone();
        let theme = self.theme;
        let ranges = self
            .line
            .clickable
            .iter()
            .map(|item| projection.range(&item.range))
            .collect::<Vec<_>>();
        let targets = self
            .line
            .clickable
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>();
        let highlights = self
            .line
            .highlights
            .iter()
            .map(|(range, style)| {
                let mut style = *style;
                if projection.objects.iter().any(|object| {
                    object.code.is_some()
                        && !object.hidden
                        && range.start <= object.original.start
                        && range.end >= object.original.end
                }) {
                    style.background_color = None;
                }
                (projection.range(range), style)
            })
            .collect::<Vec<_>>();
        let code_ranges = self
            .line
            .font_overrides
            .iter()
            .filter(|(_, family)| family.as_ref() == "monospace")
            .filter(|(range, _)| {
                projection.objects.iter().any(|object| {
                    object.code.is_some()
                        && !object.hidden
                        && object.original.start >= range.start
                        && object.original.end <= range.end
                })
            })
            .map(|(range, _)| projection.range(range))
            .collect::<Vec<_>>();
        let mut overrides = self
            .line
            .font_overrides
            .iter()
            .filter(|(range, _)| {
                !projection.objects.iter().any(|object| {
                    object.code.is_some()
                        && object.original.start >= range.start
                        && object.original.end <= range.end
                })
            })
            .map(|(range, family)| (projection.range(range), family.clone()))
            .collect::<Vec<_>>();
        overrides.extend(projection.objects.iter().map(|object| {
            (
                object.projected.clone(),
                SharedString::from(crate::fonts::INLINE_OBJECTS),
            )
        }));
        overrides.sort_by_key(|(range, _)| range.start);
        let max_size = projection
            .objects
            .iter()
            .filter(|object| object.code.is_none())
            .map(|object| object.size)
            .fold(0., f32::max);
        let runs = inline_text_runs(
            &window.text_style(),
            projection.text.len(),
            &highlights,
            &overrides,
        );
        let text = StyledText::new(projection.text).with_runs(runs);
        let layout = text.layout().clone();
        let actions = self.actions;
        let interactive = InteractiveText::new(ElementId::Name(self.key.clone().into()), text)
            .on_click(ranges, move |index, window, cx| match targets.get(index) {
                Some(ClickTarget::Url(url)) => crate::external_links::request(url, window, cx),
                Some(ClickTarget::UserMention(id)) => {
                    if let Some(callback) = &actions.user {
                        callback(id, window, cx);
                    }
                }
                Some(ClickTarget::ChannelMention(id)) => {
                    if let Some(callback) = &actions.channel {
                        callback(id, window, cx);
                    }
                }
                _ => {}
            });
        let mut root = div()
            .id(ElementId::Name(format!("{}-semantic", self.key).into()))
            .role(Role::Label)
            .aria_label(accessible)
            .relative()
            .child(interactive);
        let normal_line_height = f32::from(
            window
                .text_style()
                .line_height
                .to_pixels(px(font_size).into(), window.rem_size()),
        );
        if max_size > normal_line_height {
            root = root.line_height(px(max_size));
        }
        if !projection.objects.is_empty() || !spans.is_empty() {
            let key = self.key;
            root = root.child(
                canvas(
                    move |_, window, cx| {
                        let mut elements = Vec::new();
                        let line_height = layout.line_height();
                        // Code backgrounds follow the same wrapped fragments as the native
                        // text, rather than creating an unbreakable inline-code box.
                        for range in code_ranges {
                            for rect in super::spoiler::rectangles(&layout, &range) {
                                let code_size = font_size * 0.88;
                                let code_font =
                                    window.text_system().resolve_font(&font("monospace"));
                                let ascent = window.text_system().ascent(code_font, px(code_size));
                                let descent =
                                    window.text_system().descent(code_font, px(code_size));
                                let code_baseline =
                                    (px(code_size * 1.5) - ascent - descent) / 2. + ascent;
                                let Some(line) = layout.line_layout_for_index(range.start + 1)
                                else {
                                    continue;
                                };
                                let baseline = (line_height
                                    - line.unwrapped_layout.ascent
                                    - line.unwrapped_layout.descent)
                                    / 2.
                                    + line.unwrapped_layout.ascent;
                                let padding = px(code_size * 0.12);
                                let mut background = div()
                                    .w(rect.size.width)
                                    .h(px(code_size * 1.5) + padding * 2.)
                                    .rounded(px(theme.radius_sm))
                                    .bg(theme.surface_interactive)
                                    .into_any_element();
                                background.layout_as_root(AvailableSpace::min_size(), window, cx);
                                background.prepaint_at(
                                    rect.origin + point(px(0.), baseline - code_baseline - padding),
                                    window,
                                    cx,
                                );
                                elements.push(background);
                            }
                        }
                        for (index, object) in projection.objects.into_iter().enumerate() {
                            if object.hidden {
                                continue;
                            }
                            // A caret exactly at a wrap boundary belongs to the previous line.
                            // An interior byte selects this object's line; its caret is the
                            // far edge of the single spacer glyph, so subtract its advance.
                            let Some(mut at) =
                                layout.position_for_index(object.projected.start + 1)
                            else {
                                continue;
                            };
                            at.x -= px(object.reserved);
                            let Some(line) =
                                layout.line_layout_for_index(object.projected.start + 1)
                            else {
                                continue;
                            };
                            let baseline = (line_height
                                - line.unwrapped_layout.ascent
                                - line.unwrapped_layout.descent)
                                / 2.
                                + line.unwrapped_layout.ascent;
                            let vertical = if object.size > normal_line_height {
                                (line_height - px(object.size)) / 2.
                            } else {
                                baseline - px(object.size)
                                    + px(font_size * if object.custom { 0.35 } else { 0.2 })
                            };
                            let origin = at + point(px(object.margin), vertical);
                            let id = ElementId::Name(format!("{key}-inline-image-{index}").into());
                            let (mut element, origin) = if let Some(code) = object.code {
                                let mut style = HighlightStyle::default();
                                if code.flags.strike {
                                    style.strikethrough = Some(StrikethroughStyle {
                                        thickness: px(1.),
                                        color: None,
                                    });
                                }
                                let text = StyledText::new(code.text.clone())
                                    .with_highlights(vec![(0..code.text.len(), style)]);
                                let element = div()
                                    .id(id)
                                    .role(Role::Label)
                                    .aria_label(code.text)
                                    .font_family(code.font.family)
                                    .font_weight(code.font.weight)
                                    .text_size(px(code.font_size))
                                    .line_height(relative(1.5))
                                    .text_color(if code.flags.strike {
                                        theme.text_muted
                                    } else {
                                        theme.text
                                    })
                                    .when(code.font.style == FontStyle::Italic, |style| {
                                        style.italic()
                                    })
                                    .child(text)
                                    .into_any_element();
                                (
                                    element,
                                    at + point(px(code.padding_left), baseline - px(code.baseline)),
                                )
                            } else if let Some(unicode) = object.unicode {
                                (
                                    EmojiGlyph::new(id, unicode)
                                        .size(object.size)
                                        .into_any_element(),
                                    origin,
                                )
                            } else {
                                let mut image = div()
                                    .id(id)
                                    .role(Role::Image)
                                    .aria_label(object.label)
                                    .size(px(object.size));
                                if let Some(frame) = object.image {
                                    image = image.child(
                                        img(frame).size_full().object_fit(ObjectFit::Contain),
                                    );
                                }
                                (image.into_any_element(), origin)
                            };
                            element.layout_as_root(AvailableSpace::min_size(), window, cx);
                            element.prepaint_at(origin, window, cx);
                            elements.push(element);
                        }
                        elements.extend(super::spoiler::prepaint(
                            &layout,
                            spans,
                            overlay_targets,
                            overlay_actions,
                            theme,
                            window,
                            cx,
                        ));
                        elements
                    },
                    |_, mut elements, window, cx| {
                        for element in &mut elements {
                            element.paint(window, cx);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::{custom_emoji_requests, Kind, Object, Projection, OBJECT_BASE};
    use crate::{
        markdown::{styled_line, tokenize_inline, ClickTarget, MentionContext, RenderColors},
        Theme,
    };
    use std::collections::HashSet;
    #[test]
    fn native_runs_split_font_boundaries_even_inside_one_highlight() {
        let mut base = gpui_kit::TextStyle::default();
        base.font_family = "Inter".into();
        let runs = super::inline_text_runs(
            &base,
            9,
            &[(
                0..9,
                gpui_kit::HighlightStyle {
                    font_weight: Some(gpui_kit::FontWeight(800.)),
                    ..Default::default()
                },
            )],
            &[
                (2..5, crate::fonts::INLINE_OBJECTS.into()),
                (6..9, crate::fonts::INLINE_OBJECTS.into()),
            ],
        );
        assert_eq!(
            runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            [2, 3, 1, 3]
        );
        assert_eq!(runs[0].font.family.as_ref(), "Inter");
        assert_eq!(runs[1].font.family.as_ref(), crate::fonts::INLINE_OBJECTS);
        assert_eq!(runs[2].font.family.as_ref(), "Inter");
        assert_eq!(runs[3].font.family.as_ref(), crate::fonts::INLINE_OBJECTS);
        assert!(runs
            .iter()
            .all(|run| run.font.weight == gpui_kit::FontWeight(800.)));
    }
    fn object(range: std::ops::Range<usize>) -> Object {
        Object {
            original: range,
            projected: 0..0,
            label: "emoji".into(),
            unicode: Some("😺".into()),
            image: None,
            size: 20.,
            margin: 0.,
            reserved: 20.,
            glyph: char::from_u32(OBJECT_BASE + 2643).unwrap(),
            custom: false,
            code: None,
            hidden: false,
        }
    }
    #[test]
    fn projection_remaps_multibyte_objects_without_modifying_semantic_text() {
        let semantic = "a😺 :named: link";
        let projected = Projection::replace(semantic, vec![object(1..5), object(6..13)]);
        assert_eq!(semantic, "a😺 :named: link");
        assert_eq!(projected.objects[0].projected, 1..4);
        assert_eq!(projected.objects[1].projected, 5..8);
        assert_eq!(projected.range(&(14..18)), 9..13);
        assert_eq!(projected.range(&(0..18)), 0..13);
        assert_eq!(projected.range(&(6..13)), 5..8);
    }
    #[test]
    fn source_requests_skip_fenced_inline_code_and_concealed_spoilers() {
        let content=":01M0P54M56DA883E6ZYDK00BKC: `:01M0P54M56DA883E6ZYDK00BKD:` ||:01M0P54M56DA883E6ZYDK00BKE:||\n\n```\n:01M0P54M56DA883E6ZYDK00BKF:\n```";
        let hidden = custom_emoji_requests(content, "m", &HashSet::new());
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].id.as_deref(), Some("01M0P54M56DA883E6ZYDK00BKC"));
        let revealed = HashSet::from(["m.0.s0".into()]);
        assert_eq!(custom_emoji_requests(content, "m", &revealed).len(), 2);
    }
    #[test]
    fn unicode_sizes_and_longest_sequences_match_source_and_code_stays_text() {
        let mentions = MentionContext {
            users: &[],
            channels: &[],
            roles: &[],
        };
        let colors = RenderColors::from_theme(&Theme::dark());
        let line = styled_line(
            &tokenize_inline("👨‍👩‍👧‍👦 👍🏽"),
            &colors,
            &mentions,
            "x",
            |_| false,
        );
        assert_eq!(line.images.len(), 2);
        assert!(line
            .images
            .iter()
            .all(|candidate| matches!(&candidate.kind,Kind::Unicode{size,..} if *size==32.)));
        let line = styled_line(
            &tokenize_inline("Hi 😺 `😺` ||😺||"),
            &colors,
            &mentions,
            "x",
            |_| false,
        );
        assert_eq!(
            line.images
                .iter()
                .filter(
                    |candidate| !candidate.hidden && matches!(candidate.kind, Kind::Unicode { .. })
                )
                .count(),
            1
        );
        assert!(matches!(line.images[0].kind,Kind::Unicode{size,..}if size==20.));
    }
    #[test]
    fn resolved_mentions_are_actions_and_unresolved_mentions_remain_inert() {
        let users = vec![("01M0P54M56DA883E6ZYDK00BKC".into(), "Name".into())];
        let channels = vec![("01M0P54M56DA883E6ZYDK00BKD".into(), "General".into())];
        let mentions = MentionContext {
            users: &users,
            channels: &channels,
            roles: &[],
        };
        let colors = RenderColors::from_theme(&Theme::dark());
        let line=styled_line(&tokenize_inline("<@01M0P54M56DA883E6ZYDK00BKC> <#01M0P54M56DA883E6ZYDK00BKD> <@01M0P54M56DA883E6ZYDK00BKE>"),&colors,&mentions,"x",|_|false);
        assert_eq!(line.clickable.len(), 2);
        assert!(matches!(
            line.clickable[0].target,
            ClickTarget::UserMention(_)
        ));
        assert!(matches!(
            line.clickable[1].target,
            ClickTarget::ChannelMention(_)
        ));
    }
}
