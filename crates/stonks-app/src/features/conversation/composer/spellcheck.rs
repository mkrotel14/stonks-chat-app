//! Explicit, offline draft checks with the system Hunspell CLI and dictionary.
use super::Composer;
use gpui_kit::{div, px, Context, Div, EntityInputHandler, ParentElement, Styled, Task, Window};
use std::{
    collections::VecDeque,
    io::Write,
    ops::Range,
    process::{Command, Stdio},
};
use stonks_ui::{
    components::{Button, ButtonVariant},
    i18n::t,
    Theme,
};

// Keep a single request below Linux's minimum pipe capacity, so writing stdin
// before draining stdout cannot deadlock. Also bound dictionary work per click.
const MAX_WORDS: usize = 60;
const MAX_WORD_BYTES: usize = 64;

#[derive(Default)]
pub(super) struct Spellcheck {
    enabled: bool,
    generation: u64,
    task: Option<Task<()>>,
    review: Option<Review>,
}

struct Review {
    text: String,
    result: Result<VecDeque<Correction>, ()>,
    limited: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Correction {
    range: Range<usize>,
    suggestions: Vec<String>,
}

impl Spellcheck {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.review = None;
    }
}

impl Composer {
    /// Host preference hook. Defaults off; disabling drops any pending results.
    pub fn set_spellcheck_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.spellcheck.enabled != enabled {
            self.spellcheck.enabled = enabled;
            self.spellcheck.invalidate();
            cx.notify();
        }
    }

    fn check_spelling(&mut self, cx: &mut Context<Self>) {
        if !self.spellcheck.enabled
            || self.spellcheck.task.is_some()
            || self.state.active_channel().is_none()
            || self.state.sending_here()
        {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        self.spellcheck.invalidate();
        let generation = self.spellcheck.generation;
        let task = cx.background_executor().spawn(async move { check(text) });
        self.spellcheck.task = Some(cx.spawn(async move |this, cx| {
            let review = task.await;
            let _ = this.update(cx, |this, cx| {
                this.spellcheck.task = None;
                if this.spellcheck.generation == generation
                    && this.input.read(cx).value().as_str() == review.text
                {
                    this.spellcheck.review = Some(review);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn correct_spelling(
        &mut self,
        generation: u64,
        range: Range<usize>,
        replacement: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spellcheck.generation != generation
            || !self.spellcheck.enabled
            || self.state.active_channel().is_none()
            || self.state.sending_here()
        {
            return;
        }
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let Some(review) = &mut self.spellcheck.review else {
            return;
        };
        if self.input.read(cx).value().as_str() != review.text {
            return;
        }
        let Ok(corrections) = &mut review.result else {
            return;
        };
        if !corrections.front().is_some_and(|item| item.range == range) {
            return;
        }
        corrections.pop_front();
        // Remaining issues are in input order. Rebase their byte ranges before
        // the normal editor edit path later invalidates any old UI callbacks.
        for item in corrections.iter_mut() {
            item.range.start = item.range.start - range.len() + replacement.len();
            item.range.end = item.range.end - range.len() + replacement.len();
        }
        review.text.replace_range(range.clone(), &replacement);
        let review = self.spellcheck.review.take();
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace(replacement, window, cx);
            input.focus(window, cx);
        });
        self.changed(cx);
        self.spellcheck.review =
            review.filter(|review| self.input.read(cx).value().as_str() == review.text);
        cx.notify();
    }

    pub(super) fn spelling_controls(&self, theme: Theme, cx: &mut Context<Self>) -> Option<Div> {
        let value = self.input.read(cx).value();
        if !self.spellcheck.enabled
            || self.state.active_channel().is_none()
            || self.state.sending_here()
            || value.trim().is_empty()
        {
            return None;
        }
        let pending = self.spellcheck.task.is_some();
        let generation = self.spellcheck.generation;
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(theme.space1))
            .text_color(theme.text_muted)
            .text_size(px(theme.font_size_sm))
            .child(
                Button::new("composer-spelling", theme)
                    .variant(ButtonVariant::Quiet)
                    .label(if pending {
                        copy("Checking…", "Verificando…")
                    } else {
                        copy("Check spelling", "Verificar ortografia")
                    })
                    .disabled(pending)
                    .on_click(cx.listener(|this, _, _, cx| this.check_spelling(cx))),
            );
        if let Some(review) = self
            .spellcheck
            .review
            .as_ref()
            .filter(|r| r.text == value.as_str())
        {
            match &review.result {
                Err(()) => {
                    row = row.child(copy(
                        "Install Hunspell and a dictionary for your system language.",
                        "Instale o Hunspell e um dicionário para o idioma do sistema.",
                    ))
                }
                Ok(items) => {
                    if let Some(item) = items.front() {
                        let word = &review.text[item.range.clone()];
                        row = row.child(format!("{}: {word}", t("settings.desktop.spellcheck")));
                        for (index, suggestion) in item.suggestions.iter().enumerate() {
                            let range = item.range.clone();
                            let suggestion = suggestion.clone();
                            row = row.child(
                                Button::new(("composer-suggestion", index), theme)
                                    .variant(ButtonVariant::Quiet)
                                    .label(suggestion.clone())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.correct_spelling(
                                            generation,
                                            range.clone(),
                                            suggestion.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            );
                        }
                        let range = item.range.clone();
                        row = row.child(
                            Button::new("composer-skip-spelling", theme)
                                .variant(ButtonVariant::Quiet)
                                .label(copy("Skip", "Ignorar"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.spellcheck.generation == generation {
                                        if let Some(Review {
                                            result: Ok(items), ..
                                        }) = &mut this.spellcheck.review
                                        {
                                            if items.front().is_some_and(|item| item.range == range)
                                            {
                                                items.pop_front();
                                                cx.notify();
                                            }
                                        }
                                    }
                                })),
                        );
                    } else {
                        row = row.child(if review.limited {
                            copy(
                                "Review complete for the first 60 words.",
                                "Revisão concluída para as primeiras 60 palavras.",
                            )
                        } else {
                            copy(
                                "Spelling review complete.",
                                "Revisão ortográfica concluída.",
                            )
                        });
                    }
                }
            }
        }
        Some(row)
    }
}

// This native-only fallback UI is intentionally kept out of shared preferences
// and locale files. Follow the existing locale through its spellcheck label.
fn copy(en: &'static str, pt: &'static str) -> &'static str {
    if t("settings.desktop.spellcheck") == "Spellcheck" {
        en
    } else {
        pt
    }
}

fn words(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut ticks = 0;
    let mut offset = 0;
    for part in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += part.len();
        let was_code = ticks != 0;
        for run in part.split_inclusive(|c| c != '`') {
            let count = run.bytes().take_while(|&byte| byte == b'`').count();
            if count > 0 {
                if ticks == 0 {
                    ticks = count;
                } else if ticks == count {
                    ticks = 0;
                }
            }
        }
        let candidate = part.trim_start_matches(['(', '[', '{', '"', '*', '~', '>']);
        if was_code
            || part.contains('`')
            || candidate.starts_with(['@', '#', ':', '/', '<'])
            || part.contains('@')
            || part.contains("://")
            || candidate
                .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                .contains('.')
            || part.chars().any(|c| c.is_numeric() || c == '_')
        {
            continue;
        }
        let mut begin = 0;
        for (i, c) in part.char_indices().chain([(part.len(), '\0')]) {
            if !c.is_alphabetic() && !matches!(c, '\'' | '’' | '\u{0300}'..='\u{036f}') {
                let word = part[begin..i].trim_matches(['\'', '’']);
                let front =
                    part[begin..i].len() - part[begin..i].trim_start_matches(['\'', '’']).len();
                if word.len() <= MAX_WORD_BYTES
                    && word.chars().filter(|c| c.is_alphabetic()).count() >= 2
                {
                    let b = start + begin + front;
                    ranges.push(b..b + word.len());
                    if ranges.len() > MAX_WORDS {
                        return ranges;
                    }
                }
                begin = i + c.len_utf8();
            }
        }
    }
    ranges
}

fn check(text: String) -> Review {
    let mut ranges = words(&text);
    let limited = ranges.len() > MAX_WORDS;
    ranges.truncate(MAX_WORDS);
    let result = if ranges.is_empty() {
        Ok(VecDeque::new())
    } else {
        run(&text, ranges)
    };
    Review {
        text,
        result,
        limited,
    }
}

fn run(text: &str, ranges: Vec<Range<usize>>) -> Result<VecDeque<Correction>, ()> {
    let mut child = Command::new("hunspell")
        .args(["-a", "-i", "UTF-8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    let written = child.stdin.take().ok_or(()).and_then(|mut stdin| {
        for range in &ranges {
            // Hunspell's ^ prefix forces text mode rather than pipe commands.
            writeln!(stdin, "^{}", &text[range.clone()]).map_err(|_| ())?;
        }
        Ok(())
    });
    let output = child.wait_with_output().map_err(|_| ())?;
    written?;
    if !output.status.success() {
        return Err(());
    }
    parse(std::str::from_utf8(&output.stdout).map_err(|_| ())?, ranges)
}

fn parse(output: &str, ranges: Vec<Range<usize>>) -> Result<VecDeque<Correction>, ()> {
    let mut lines = output.lines();
    if !lines.next().is_some_and(|line| line.starts_with("@(#)")) {
        return Err(());
    }
    let lines: Vec<_> = lines.collect();
    let records: Vec<_> = lines
        .split(|line| line.is_empty())
        .filter(|record| !record.is_empty())
        .collect();
    if records.len() != ranges.len() {
        return Err(());
    }
    Ok(ranges
        .into_iter()
        .zip(records)
        .filter_map(|(range, record)| {
            // Some dictionaries split an apostrophized word into multiple tokens;
            // never apply a result for only a fragment to the entire word.
            let [line] = record else {
                return None;
            };
            let suggestions = if line.starts_with(['&', '?']) {
                line.split_once(": ")?
                    .1
                    .split(", ")
                    .filter(|word| !word.is_empty() && word.len() <= MAX_WORD_BYTES)
                    .take(4)
                    .map(str::to_owned)
                    .collect()
            } else if line.starts_with('#') {
                Vec::new()
            } else {
                return None;
            };
            Some(Correction { range, suggestions })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_utf8_ranges_without_chat_syntax() {
        let text = "🙂 Olá, maçã! @Mary #channel <@ABC> https://example.org `co de` ``more code`` :wave: naïve spells.";
        let extracted: Vec<_> = words(text).into_iter().map(|range| &text[range]).collect();
        assert_eq!(extracted, ["Olá", "maçã", "naïve", "spells"]);
    }

    #[test]
    fn maps_hunspell_records_to_original_ranges() {
        let text = "Olá wrld wrld zzzzz don't";
        let records = "@(#) Hunspell\n*\n\n& wrld 2 1: world, wild\n\n& wrld 1 1: world\n\n# zzzzz 1\n\n*\n# t 5\n\n";
        let ranges = words(text);
        let result = parse(records, ranges.clone()).unwrap();
        assert_eq!(result[0].range, ranges[1]);
        assert_eq!(result[1].range, ranges[2]);
        assert_eq!(result[0].suggestions, ["world", "wild"]);
        assert!(result[2].suggestions.is_empty());
        assert_eq!(result.len(), 3);
        assert!(parse("@(#) Hunspell\n*\n\n", ranges).is_err());
    }

    #[test]
    fn starts_disabled_and_invalidation_discards_results() {
        let mut state = Spellcheck::default();
        assert!(!state.enabled);
        let generation = state.generation;
        state.review = Some(check(String::new()));
        state.invalidate();
        assert!(state.review.is_none());
        assert_ne!(state.generation, generation);
    }

    #[test]
    #[ignore = "requires the system Hunspell CLI and an English dictionary"]
    fn system_hunspell_corrects_a_typo() {
        let review = check("hello speling".to_owned());
        let corrections = review.result.unwrap();
        assert_eq!(corrections.len(), 1);
        assert_eq!(&review.text[corrections[0].range.clone()], "speling");
        assert!(corrections[0].suggestions.iter().any(|s| s == "spelling"));
    }
}
