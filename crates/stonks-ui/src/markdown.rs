//! Port of `src/features/messages/components/FormattedMessage.tsx`:
//! block parsing (paragraphs, fences, tables, quotes, headings, lists),
//! inline tokenizing (mentions, custom emoji, timestamps, links, code,
//! bold/italic/strike/spoiler) and the tiny code highlighter. Rendering
//! assembles `StyledText`-ready highlight runs; interactivity (link
//! clicks, spoiler hover) is wired by the caller via the returned ranges.
//!
//! Unicode/custom artwork and scaled monospace code use measured inline
//! objects before native wrapping. Original text stays separate from that
//! projection; user/channel actions and grouped spoiler hover/focus behavior
//! are wired by the shared view with caller-owned navigation/image loading.

use std::ops::Range;
use std::sync::LazyLock;

use gpui_kit::{FontStyle, FontWeight, HighlightStyle, Hsla, SharedString, StrikethroughStyle};
use regex::{Regex, RegexBuilder};

use crate::i18n::{self, DateStyle};
use crate::theme::Theme;

mod blocks;
mod code;
mod inline_images;
mod spoiler;
mod view;
pub use inline_images::{
    custom_emoji_requests, inline_text_runs, CustomEmoji, CustomEmojiResolver, InlineEmoji,
    InlineMedia,
};
pub use view::{
    render, render_with_actions, render_with_media, render_with_options, InlineAction,
    InlineActions, RenderOptions, SpoilerHover,
};

const ID: &str = "[0-9A-HJKMNP-TV-Z]{26}";

fn token_re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .expect("markdown regex compiles")
}

static INLINE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    token_re(&format!(
        "(<@{ID}>|<%{ID}>|<#{ID}>|@everyone|@online|:{ID}:|<:[^:>]{{1,64}}:{ID}>|:[A-Za-z0-9_-]{{1,32}}:|<t:\\d{{1,13}}(?::[tTdDfFR])?>|\\[[^\\]\\n]+\\]\\(https?://[^\\s)]+\\)|https?://[^\\s<>]+|`[^`\\n]+`|\\*\\*[^*\\n]+\\*\\*|__[^_\\n]+__|~~[^~\\n]+~~|\\|\\|[^|\\n]+\\|\\||!![^!\\n]+!!|\\*[^*\\n]+\\*|_[^_\\n]+_)"
    ))
});
static USER_MENTION: LazyLock<Regex> = LazyLock::new(|| token_re(&format!("^<@({ID})>$")));
static ROLE_MENTION: LazyLock<Regex> = LazyLock::new(|| token_re(&format!("^<%({ID})>$")));
static CHANNEL_MENTION: LazyLock<Regex> = LazyLock::new(|| token_re(&format!("^<#({ID})>$")));
static CUSTOM_EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    token_re(&format!(
        "^(?::({ID}):|<:([^:>]{{1,64}}):({ID})>|:([A-Za-z0-9_-]{{1,32}}):)$"
    ))
});
static TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"^<t:(\d{1,13})(?::([tTdDfFR]))?>$"));
static MARKDOWN_LINK: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"^\[([^\]\n]+)\]\((https?://[^\s)]+)\)$"));
static TRAILING_PUNCT: LazyLock<Regex> = LazyLock::new(|| token_re(r"[),.!?]+$"));

/// Style flags accumulated while descending into nested inline wrappers
/// (`**a _b_ c_**` renders `b c` bold+italic).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct RunFlags {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub spoiler: bool,
    pub spoiler_group: Option<usize>,
    pub code: bool,
}

/// Interactive payload of an inline run; plain text carries `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    None,
    Link(String),
    UserMention(String),
    RoleMention(String),
    ChannelMention(String),
    /// `@everyone` / `@online`.
    MassMention,
    /// `:id:` / `<:name:id>` / `:name:`; the view resolves metadata/images,
    /// while tokenization keeps the original semantic spelling intact.
    CustomEmoji {
        id: Option<String>,
        name: Option<String>,
        raw: String,
    },
    Timestamp {
        millis: i64,
        style: Option<char>,
    },
}

#[derive(Clone, Debug)]
pub struct InlineRun {
    /// Raw matched token; the styled line resolves display text (mention
    /// names, link labels) from it.
    pub text: String,
    pub flags: RunFlags,
    pub payload: Payload,
}

fn text_run(text: &str, flags: RunFlags) -> InlineRun {
    InlineRun {
        text: text.to_owned(),
        flags,
        payload: Payload::None,
    }
}

pub fn tokenize_inline(text: &str) -> Vec<InlineRun> {
    tokenize_with(text, RunFlags::default(), &mut 0)
}

fn tokenize_with(text: &str, flags: RunFlags, next_group: &mut usize) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    let mut last = 0;
    for found in INLINE_TOKEN.find_iter(text) {
        if found.start() > last {
            runs.push(text_run(&text[last..found.start()], flags));
        }
        classify(found.as_str(), flags, &mut runs, next_group);
        last = found.end();
    }
    if last < text.len() {
        runs.push(text_run(&text[last..], flags));
    }
    runs
}

fn classify(token: &str, flags: RunFlags, runs: &mut Vec<InlineRun>, next_group: &mut usize) {
    let mut payload = |payload: Payload| {
        runs.push(InlineRun {
            text: token.to_owned(),
            flags,
            payload,
        });
    };
    if let Some(captures) = USER_MENTION.captures(token) {
        return payload(Payload::UserMention(captures[1].to_owned()));
    }
    if let Some(captures) = ROLE_MENTION.captures(token) {
        return payload(Payload::RoleMention(captures[1].to_owned()));
    }
    if token.eq_ignore_ascii_case("@everyone") || token.eq_ignore_ascii_case("@online") {
        return payload(Payload::MassMention);
    }
    if let Some(captures) = CHANNEL_MENTION.captures(token) {
        return payload(Payload::ChannelMention(captures[1].to_owned()));
    }
    if let Some(captures) = CUSTOM_EMOJI.captures(token) {
        return payload(Payload::CustomEmoji {
            id: captures
                .get(1)
                .or(captures.get(3))
                .map(|id| id.as_str().to_owned()),
            name: captures
                .get(2)
                .or(captures.get(4))
                .map(|name| name.as_str().to_owned()),
            raw: token.to_owned(),
        });
    }
    if let Some(captures) = TIMESTAMP.captures(token) {
        if let Ok(value) = captures[1].parse::<i64>() {
            let millis = if value < 10_000_000_000 {
                value.saturating_mul(1_000)
            } else {
                value
            };
            return payload(Payload::Timestamp {
                millis,
                style: captures
                    .get(2)
                    .and_then(|style| style.as_str().chars().next()),
            });
        }
        // Fall through: an out-of-range timestamp renders as plain text.
    }
    if let Some(captures) = MARKDOWN_LINK.captures(token) {
        if safe_link(&captures[2]).is_some() {
            return runs.push(InlineRun {
                text: captures[1].to_owned(),
                flags,
                payload: Payload::Link(captures[2].to_owned()),
            });
        }
    }
    if token
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || token
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        let trailing = TRAILING_PUNCT
            .find(token)
            .map(|found| found.as_str())
            .unwrap_or_default();
        let candidate = &token[..token.len() - trailing.len()];
        if safe_link(candidate).is_some() {
            runs.push(InlineRun {
                text: candidate.to_owned(),
                flags,
                payload: Payload::Link(candidate.to_owned()),
            });
        } else {
            runs.push(text_run(candidate, flags));
        }
        if !trailing.is_empty() {
            runs.push(text_run(trailing, flags));
        }
        return;
    }
    if token.starts_with('`') && token.ends_with('`') && token.len() >= 2 {
        return runs.push(InlineRun {
            text: token[1..token.len() - 1].to_owned(),
            flags: RunFlags {
                code: true,
                ..flags
            },
            payload: Payload::None,
        });
    }
    for (marker, apply) in [
        ("**", |f: RunFlags| RunFlags { bold: true, ..f }),
        ("__", |f: RunFlags| RunFlags { bold: true, ..f }),
        ("~~", |f: RunFlags| RunFlags { strike: true, ..f }),
        ("||", |f: RunFlags| RunFlags { spoiler: true, ..f }),
        ("!!", |f: RunFlags| RunFlags { spoiler: true, ..f }),
    ] as [(&str, fn(RunFlags) -> RunFlags); 5]
    {
        if token.starts_with(marker) && token.ends_with(marker) && token.len() >= 2 * marker.len() {
            let inner = &token[marker.len()..token.len() - marker.len()];
            let mut nested = apply(flags);
            if nested.spoiler && !flags.spoiler {
                nested.spoiler_group = Some(*next_group);
                *next_group += 1;
            }
            return runs.extend(tokenize_with(inner, nested, next_group));
        }
    }
    for marker in ["*", "_"] {
        if token.starts_with(marker) && token.ends_with(marker) && token.len() >= 2 * marker.len() {
            let inner = &token[marker.len()..token.len() - marker.len()];
            return runs.extend(tokenize_with(
                inner,
                RunFlags {
                    italic: true,
                    ..flags
                },
                next_group,
            ));
        }
    }
    runs.push(text_run(token, flags));
}

/// http/https only, matching `safeLink` in the Solid renderer.
fn safe_link(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

// --- Blocks ---

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<String>),
    Code {
        language: String,
        code: String,
    },
    Quote(Vec<String>),
    Heading {
        level: usize,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

static CODE_FENCE: LazyLock<Regex> = LazyLock::new(|| token_re(r"^```([^`]*)$"));
static CODE_FENCE_END: LazyLock<Regex> = LazyLock::new(|| token_re(r"^```\s*$"));
static TABLE_DELIMITER: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$"));
static TABLE_ROW: LazyLock<Regex> = LazyLock::new(|| token_re(r"^\s*\|"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| token_re(r"^>\s?"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| token_re(r"^(#{1,3})\s+(.+)$"));
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| token_re(r"^\s*(?:([-+*])|(\d+)\.)\s+(.+)$"));

pub fn parse_blocks(content: &str) -> Vec<Block> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if let Some(fence) = CODE_FENCE.captures(line) {
            let mut code = Vec::new();
            index += 1;
            while index < lines.len() && !CODE_FENCE_END.is_match(lines[index]) {
                code.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            blocks.push(Block::Code {
                language: fence[1].trim().to_lowercase(),
                code: code.join("\n"),
            });
            continue;
        }
        if index + 1 < lines.len() && TABLE_DELIMITER.is_match(lines[index + 1]) {
            let cells = |value: &str| {
                value
                    .trim()
                    .trim_start_matches('|')
                    .trim_end_matches('|')
                    .split('|')
                    .map(|cell| cell.trim().to_owned())
                    .collect::<Vec<_>>()
            };
            let headers = cells(line);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len() && TABLE_ROW.is_match(lines[index]) {
                rows.push(cells(lines[index]));
                index += 1;
            }
            blocks.push(Block::Table { headers, rows });
            continue;
        }
        if QUOTE.is_match(line) {
            let mut quote = Vec::new();
            while index < lines.len() && QUOTE.is_match(lines[index]) {
                quote.push(QUOTE.replace(lines[index], "").into_owned());
                index += 1;
            }
            blocks.push(Block::Quote(quote));
            continue;
        }
        if let Some(heading) = HEADING.captures(line) {
            blocks.push(Block::Heading {
                level: heading[1].len(),
                text: heading[2].to_owned(),
            });
            index += 1;
            continue;
        }
        if let Some(item) = LIST_ITEM.captures(line) {
            let ordered = item.get(2).is_some();
            let mut items = vec![item[3].to_owned()];
            index += 1;
            while index < lines.len() {
                let Some(item) = LIST_ITEM.captures(lines[index]) else {
                    break;
                };
                if item.get(2).is_some() != ordered {
                    break;
                }
                items.push(item[3].to_owned());
                index += 1;
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }
        let mut paragraph = Vec::new();
        while index < lines.len()
            && !lines[index].trim().is_empty()
            && !CODE_FENCE.is_match(lines[index])
            && !QUOTE.is_match(lines[index])
            && !HEADING.is_match(lines[index])
            && !LIST_ITEM.is_match(lines[index])
        {
            paragraph.push(lines[index].to_owned());
            index += 1;
        }
        blocks.push(Block::Paragraph(paragraph));
    }
    blocks
}

// --- Code highlighting ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeToken {
    Plain,
    Comment,
    String,
    Keyword,
    Number,
}

static HIGHLIGHT_LANGUAGES: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"^(?:js|javascript|ts|typescript|json|rust|rs|css|html|sh|bash)$"));
static HIGHLIGHT_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    token_re(
        r#"(//[^\n]*|#[^\n]*|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|\b(?:true|false|null|let|const|function|fn|pub|struct|impl|async|await|return|if|else|match)\b|\b\d+(?:\.\d+)?\b)"#,
    )
});
static KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    token_re(
        r"^(?:true|false|null|let|const|function|fn|pub|struct|impl|async|await|return|if|else|match)$",
    )
});

/// The deliberately small highlighter from the Solid app: comments,
/// strings, keywords, numbers; unknown languages stay plain.
pub fn highlight_code(value: &str, language: &str) -> Vec<(String, CodeToken)> {
    if !HIGHLIGHT_LANGUAGES.is_match(language) {
        return vec![(value.to_owned(), CodeToken::Plain)];
    }
    let mut parts = Vec::new();
    let mut last = 0;
    for found in HIGHLIGHT_TOKEN.find_iter(value) {
        if found.start() > last {
            parts.push((value[last..found.start()].to_owned(), CodeToken::Plain));
        }
        let part = found.as_str();
        let kind = if part.starts_with("//") || part.starts_with('#') {
            CodeToken::Comment
        } else if part.starts_with('"') || part.starts_with('\'') {
            CodeToken::String
        } else if KEYWORD.is_match(part) {
            CodeToken::Keyword
        } else if part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            CodeToken::Number
        } else {
            CodeToken::Plain
        };
        parts.push((part.to_owned(), kind));
        last = found.end();
    }
    if last < value.len() {
        parts.push((value[last..].to_owned(), CodeToken::Plain));
    }
    parts
}

// --- Styled lines ---

/// Colors a styled line needs, snapped from the theme once per render.
pub struct RenderColors {
    pub text: Hsla,
    pub muted: Hsla,
    pub link: Hsla,
    pub danger: Hsla,
    pub selected_bg: Hsla,
    pub code_bg: Hsla,
}

impl RenderColors {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            text: theme.text.into(),
            muted: theme.text_muted.into(),
            link: theme.link.into(),
            danger: theme.danger.into(),
            selected_bg: theme.surface_selected.into(),
            code_bg: theme.surface_interactive.into(),
        }
    }
}

/// Mention resolution inputs, as (id, label) pairs the caller snapshots
/// from the session. Roles carry an optional `#rrggbb` colour.
#[derive(Default)]
pub struct MentionContext<'a> {
    pub users: &'a [(String, String)],
    pub channels: &'a [(String, String)],
    pub roles: &'a [(String, String, Option<String>)],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickTarget {
    Url(String),
    UserMention(String),
    ChannelMention(String),
    /// Key is `{prefix}.{run index}`; the caller maps it to hover or
    /// toggle state.
    Spoiler(String),
}

#[derive(Clone)]
pub struct ClickRange {
    pub range: Range<usize>,
    pub target: ClickTarget,
}

/// One wrapped text run: full text plus everything `StyledText` /
/// `InteractiveText` need (highlights, mono overrides, click ranges).
#[derive(Clone)]
pub struct StyledLine {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    pub font_overrides: Vec<(Range<usize>, SharedString)>,
    pub clickable: Vec<ClickRange>,
    spoilers: Vec<ClickRange>,
    concealed: Vec<Range<usize>>,
    images: Vec<inline_images::Candidate>,
}

/// `<t:unix:style>` label, port of `timestampLabel`.
fn timestamp_label(millis: i64, style: Option<char>) -> String {
    match style {
        Some('R') => i18n::format_relative_time(millis),
        Some('t') => i18n::format_time_ms(millis, false),
        Some('T') => i18n::format_time_ms(millis, true),
        Some('d') => i18n::format_date_ms(millis, DateStyle::Numeric, true, false),
        Some('D') => i18n::format_date_ms(millis, DateStyle::Long, true, false),
        Some('F') => {
            let date = i18n::format_date_ms(millis, DateStyle::Long, true, true);
            let time = i18n::format_time_ms(millis, false);
            if date.is_empty() {
                return date;
            }
            format!("{date} {time}")
        }
        // `f` (default) matches the Solid app's formatDateTime(long).
        _ => i18n::format_datetime_ms(millis, DateStyle::Long, false),
    }
}

fn parse_hex(value: &str) -> Option<Hsla> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |offset: usize| u8::from_str_radix(&hex[offset..offset + 2], 16).ok();
    Some(
        gpui_kit::rgba(
            ((channel(0)? as u32) << 24)
                | ((channel(2)? as u32) << 16)
                | ((channel(4)? as u32) << 8)
                | 0xff,
        )
        .into(),
    )
}

fn spoiler_key(prefix: &str, flags: &RunFlags, index: usize) -> String {
    flags
        .spoiler_group
        .map(|group| format!("{prefix}.s{group}"))
        .unwrap_or_else(|| format!("{prefix}.{index}"))
}

/// Assemble display text + highlights + interaction ranges for one line
/// of inline runs. `spoiler_prefix` namespaces spoiler keys per message
/// block; `revealed` reports whether a spoiler key is currently visible.
pub fn styled_line(
    runs: &[InlineRun],
    colors: &RenderColors,
    mentions: &MentionContext,
    spoiler_prefix: &str,
    revealed: impl Fn(&str) -> bool,
) -> StyledLine {
    let mut line = StyledLine {
        text: String::new(),
        highlights: Vec::new(),
        font_overrides: Vec::new(),
        clickable: Vec::new(),
        spoilers: Vec::new(),
        concealed: Vec::new(),
        images: Vec::new(),
    };
    for (index, run) in runs.iter().enumerate() {
        let mut style = HighlightStyle::default();
        let mut display: Option<String> = None;
        let mut target = None;

        match &run.payload {
            Payload::Link(url) => {
                style.color = Some(colors.link);
                target = Some(ClickTarget::Url(url.clone()));
            }
            Payload::UserMention(id) => {
                let resolved = mentions.users.iter().find(|(candidate, _)| candidate == id);
                display = Some(match resolved {
                    Some((_, name)) => format!("@{name}"),
                    None => "@unknown-user".to_owned(),
                });
                style.color = Some(colors.link);
                style.background_color = Some(colors.selected_bg);
                style.font_weight = Some(FontWeight::BOLD);
                if resolved.is_none() {
                    style.fade_out = Some(0.7);
                } else {
                    target = Some(ClickTarget::UserMention(id.clone()));
                }
            }
            Payload::RoleMention(id) => {
                let role = mentions
                    .roles
                    .iter()
                    .find(|(candidate, _, _)| candidate == id);
                display = Some(match role {
                    Some((_, name, _)) => format!("@{name}"),
                    None => "@unknown-role".to_owned(),
                });
                style.color = Some(
                    role.and_then(|(_, _, colour)| colour.as_deref())
                        .and_then(parse_hex)
                        .unwrap_or(colors.link),
                );
                style.background_color = Some(colors.selected_bg);
                style.font_weight = Some(FontWeight::BOLD);
                if role.is_none() {
                    style.fade_out = Some(0.7);
                }
            }
            Payload::MassMention => {
                style.color = Some(colors.danger);
                style.background_color = Some(colors.selected_bg);
                style.font_weight = Some(FontWeight::BOLD);
            }
            Payload::ChannelMention(id) => {
                let resolved = mentions
                    .channels
                    .iter()
                    .find(|(candidate, _)| candidate == id);
                display = Some(match resolved {
                    Some((_, name)) => format!("#{name}"),
                    None => "#unknown-channel".to_owned(),
                });
                style.color = Some(colors.link);
                style.background_color = Some(colors.selected_bg);
                style.font_weight = Some(FontWeight::BOLD);
                if resolved.is_none() {
                    style.fade_out = Some(0.7);
                } else {
                    target = Some(ClickTarget::ChannelMention(id.clone()));
                }
            }
            Payload::CustomEmoji { name, raw, .. } => {
                display = Some(match name {
                    Some(name) => format!(":{name}:"),
                    None => raw.clone(),
                });
            }
            Payload::Timestamp {
                millis,
                style: ts_style,
            } => {
                let label = timestamp_label(*millis, *ts_style);
                if !label.is_empty() {
                    display = Some(label);
                    style.color = Some(colors.text);
                    style.background_color = Some(colors.selected_bg);
                }
            }
            Payload::None => {}
        }

        if run.flags.bold {
            style.font_weight = Some(FontWeight(800.));
            if run.payload == Payload::None {
                style.color = Some(colors.text);
            }
        }
        if run.flags.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if run.flags.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: gpui_kit::px(1.),
                color: None,
            });
            if run.payload == Payload::None {
                style.color = Some(colors.muted);
            }
        }
        if run.flags.code {
            style.background_color = Some(colors.code_bg);
        }
        let mut hidden = false;
        if run.flags.spoiler {
            let key = spoiler_key(spoiler_prefix, &run.flags, index);
            style.background_color = Some(colors.selected_bg);
            if !revealed(&key) {
                hidden = true;
                // Hidden: text painted in the background color.
                style.color = Some(colors.selected_bg);
            }
            if hidden {
                target = Some(ClickTarget::Spoiler(key));
            }
        }

        let start = line.text.len();
        match display {
            Some(display) => line.text.push_str(&display),
            None => line.text.push_str(&run.text),
        }
        let range = start..line.text.len();
        if style != HighlightStyle::default() {
            line.highlights.push((range.clone(), style));
        }
        if run.flags.code {
            line.font_overrides
                .push((range.clone(), SharedString::from("monospace")));
        }
        if run.flags.spoiler {
            line.spoilers.push(ClickRange {
                range: range.clone(),
                target: ClickTarget::Spoiler(spoiler_key(spoiler_prefix, &run.flags, index)),
            });
        }
        if hidden {
            line.concealed.push(range.clone());
        }
        inline_images::collect(&mut line, run, range.clone(), hidden);
        if let Some(target) = target {
            line.clickable.push(ClickRange { range, target });
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, Payload)> {
        tokenize_inline(text)
            .into_iter()
            .map(|run| (run.text, run.payload))
            .collect()
    }

    #[test]
    fn splits_plain_text_and_tokens() {
        let runs = kinds("hey <@01M0P54M56DA883E6ZYDK00BKC> bye");
        assert_eq!(
            runs,
            vec![
                ("hey ".to_owned(), Payload::None),
                (
                    "<@01M0P54M56DA883E6ZYDK00BKC>".to_owned(),
                    Payload::UserMention("01M0P54M56DA883E6ZYDK00BKC".to_owned()),
                ),
                (" bye".to_owned(), Payload::None),
            ]
        );
    }

    #[test]
    fn mention_and_mass_variants() {
        let id = "01M0P54M56DA883E6ZYDK00BKC";
        assert_eq!(
            kinds(&format!("<%{id}>"))[0].1,
            Payload::RoleMention(id.to_owned())
        );
        assert_eq!(
            kinds(&format!("<#{id}>"))[0].1,
            Payload::ChannelMention(id.to_owned())
        );
        assert_eq!(kinds("@everyone")[0].1, Payload::MassMention);
        assert_eq!(kinds("@online")[0].1, Payload::MassMention);
    }

    #[test]
    fn custom_emoji_forms() {
        let id = "01M0P54M56DA883E6ZYDK00BKC";
        assert_eq!(
            kinds(&format!(":{id}:"))[0].1,
            Payload::CustomEmoji {
                id: Some(id.to_owned()),
                name: None,
                raw: format!(":{id}:"),
            }
        );
        assert_eq!(
            kinds(&format!("<:stonks:{id}>"))[0].1,
            Payload::CustomEmoji {
                id: Some(id.to_owned()),
                name: Some("stonks".to_owned()),
                raw: format!("<:stonks:{id}>"),
            }
        );
        assert_eq!(
            kinds(":word-1:")[0].1,
            Payload::CustomEmoji {
                id: None,
                name: Some("word-1".to_owned()),
                raw: ":word-1:".to_owned(),
            }
        );
    }

    #[test]
    fn timestamps_seconds_and_millis() {
        assert_eq!(
            kinds("<t:1700000000:R>")[0].1,
            Payload::Timestamp {
                millis: 1_700_000_000_000,
                style: Some('R'),
            }
        );
        assert_eq!(
            kinds("<t:1700000000000>")[0].1,
            Payload::Timestamp {
                millis: 1_700_000_000_000,
                style: None,
            }
        );
    }

    #[test]
    fn links_markdown_and_bare_with_trailing_punctuation() {
        let runs = kinds("[label](https://example.com/x)");
        assert_eq!(
            runs,
            vec![(
                "label".to_owned(),
                Payload::Link("https://example.com/x".to_owned()),
            )]
        );
        let runs = kinds("see https://example.com/a,b!");
        // Trailing punctuation is split off only up to the last
        // non-punctuation char, mirroring the JS `/[),.!?]+$/` behavior.
        assert_eq!(
            runs[1],
            (
                "https://example.com/a,b".to_owned(),
                Payload::Link("https://example.com/a,b".to_owned()),
            )
        );
        assert_eq!(runs[2], ("!".to_owned(), Payload::None));
        let runs = kinds("see https://example.com/a,!");
        assert_eq!(
            runs[1],
            (
                "https://example.com/a".to_owned(),
                Payload::Link("https://example.com/a".to_owned()),
            )
        );
        assert_eq!(runs[2], (",!".to_owned(), Payload::None));
        // Non-http schemes are not links.
        assert!(kinds("javascript:alert(1)")
            .iter()
            .all(|(_, payload)| *payload == Payload::None));
    }

    #[test]
    fn style_wrappers_and_nesting() {
        let runs = tokenize_inline("**bold _both_** and `code` ~~gone~~");
        assert!(runs[0].flags.bold && !runs[0].flags.italic);
        let both = runs
            .iter()
            .find(|run| run.text == "both")
            .expect("nested run");
        assert!(both.flags.bold && both.flags.italic);
        let code = runs
            .iter()
            .find(|run| run.text == "code")
            .expect("code run");
        assert!(code.flags.code);
        let gone = runs
            .iter()
            .find(|run| run.text == "gone")
            .expect("strike run");
        assert!(gone.flags.strike);
    }

    #[test]
    fn spoiler_flags() {
        let runs = tokenize_inline("||secret|| and !!also!!");
        let secret = runs
            .iter()
            .find(|run| run.text == "secret")
            .expect("spoiler run");
        assert!(secret.flags.spoiler);
        let also = runs
            .iter()
            .find(|run| run.text == "also")
            .expect("bang spoiler run");
        assert!(also.flags.spoiler);
    }

    #[test]
    fn blocks_fence_quote_heading_list_paragraph() {
        let blocks = parse_blocks(
            "# Title\n\npara line one\nline two\n\n> quoted\n> more\n\n- a\n- b\n\n1. x\n2. y\n\n```rust\nlet a = 1;\n```\n",
        );
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                text: "Title".to_owned(),
            }
        );
        assert_eq!(
            blocks[1],
            Block::Paragraph(vec!["para line one".to_owned(), "line two".to_owned()])
        );
        assert_eq!(
            blocks[2],
            Block::Quote(vec!["quoted".to_owned(), "more".to_owned()])
        );
        assert_eq!(
            blocks[3],
            Block::List {
                ordered: false,
                items: vec!["a".to_owned(), "b".to_owned()],
            }
        );
        assert_eq!(
            blocks[4],
            Block::List {
                ordered: true,
                items: vec!["x".to_owned(), "y".to_owned()],
            }
        );
        assert_eq!(
            blocks[5],
            Block::Code {
                language: "rust".to_owned(),
                code: "let a = 1;".to_owned(),
            }
        );
    }

    #[test]
    fn blocks_table() {
        let blocks = parse_blocks("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert_eq!(
            blocks,
            vec![Block::Table {
                headers: vec!["a".to_owned(), "b".to_owned()],
                rows: vec![vec!["1".to_owned(), "2".to_owned()]],
            }]
        );
    }

    #[test]
    fn malformed_fence_makes_progress_and_preserves_text() {
        assert_eq!(
            parse_blocks("```bad`fence\ntext"),
            vec![Block::Paragraph(vec!["```bad`fence".into(), "text".into()]),]
        );
        assert_eq!(
            parse_blocks("```rust\nunfinished"),
            vec![Block::Code {
                language: "rust".into(),
                code: "unfinished".into()
            },]
        );
    }

    #[test]
    fn links_validate_urls_and_accept_uppercase_schemes() {
        assert!(safe_link("https://").is_none());
        assert!(safe_link("https://[invalid").is_none());
        assert!(safe_link("javascript:alert(1)").is_none());
        assert!(matches!(
            &kinds("HTTPS://example.com")[0].1,
            Payload::Link(_)
        ));
    }

    #[test]
    fn styled_line_resolves_mentions_and_keeps_utf8_ranges() {
        let id = "01M0P54M56DA883E6ZYDK00BKC".to_owned();
        let users = vec![(id.clone(), "João 🦀".into())];
        let channels = vec![(id.clone(), "geral".into())];
        let roles = vec![(id.clone(), "Equipe".into(), Some("#12abef".into()))];
        let mentions = MentionContext {
            users: &users,
            channels: &channels,
            roles: &roles,
        };
        for theme in [Theme::dark(), Theme::light()] {
            let colors = RenderColors::from_theme(&theme);
            let line = styled_line(
                &tokenize_inline(&format!(
                    "Olá <@{id}> <#{id}> <%{id}> [link](https://example.com) ||segredo|| `código`"
                )),
                &colors,
                &mentions,
                "message.0",
                |_| false,
            );
            assert_eq!(line.text, "Olá @João 🦀 #geral @Equipe link segredo código");
            for (range, _) in &line.highlights {
                assert!(line.text.is_char_boundary(range.start));
                assert!(line.text.is_char_boundary(range.end));
            }
            assert_eq!(
                &line.text[line
                    .clickable
                    .iter()
                    .find(|item| matches!(item.target, ClickTarget::Url(_)))
                    .unwrap()
                    .range
                    .clone()],
                "link"
            );
            let spoiler = line
                .clickable
                .iter()
                .find(|item| matches!(item.target, ClickTarget::Spoiler(_)))
                .unwrap();
            assert_eq!(&line.text[spoiler.range.clone()], "segredo");
            let hidden = line
                .highlights
                .iter()
                .find(|(range, _)| range == &spoiler.range)
                .unwrap();
            assert_eq!(hidden.1.color, hidden.1.background_color);
            assert_eq!(&line.text[line.font_overrides[0].0.clone()], "código");
            let revealed = styled_line(
                &tokenize_inline("||segredo||"),
                &colors,
                &mentions,
                "s",
                |_| true,
            );
            assert_ne!(revealed.highlights[0].1.color, Some(colors.selected_bg));
        }
    }

    #[test]
    fn highlighter_buckets() {
        let parts = highlight_code("let x = \"s\"; // hi\nlet n = 42", "rust");
        assert!(parts.contains(&("let".to_owned(), CodeToken::Keyword)));
        assert!(parts.contains(&("\"s\"".to_owned(), CodeToken::String)));
        assert!(parts.contains(&("// hi".to_owned(), CodeToken::Comment)));
        assert!(parts.contains(&("42".to_owned(), CodeToken::Number)));
        // Unknown languages stay plain.
        assert_eq!(
            highlight_code("let x = 1", "brainfuck"),
            vec![("let x = 1".to_owned(), CodeToken::Plain)]
        );
    }
}
