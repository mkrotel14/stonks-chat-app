//! Debounced, offline draft checks with the system Hunspell CLI and dictionary.
use super::Composer;
use gpui_kit::{
    canvas, point, px, AnyElement, ContentMask, Context, EntityInputHandler, Hsla, IntoElement,
    SharedString, Styled, Task, UnderlineStyle, Window,
};
use std::{
    io::Write,
    ops::Range,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

const DEBOUNCE: Duration = Duration::from_millis(400);
// Keep stdin below Linux's minimum pipe capacity even when Hunspell writes
// suggestions before consuming its full input.
const MAX_WORDS: usize = 60;
const MAX_WORD_BYTES: usize = 64;

#[derive(Default)]
pub(super) struct Spellcheck {
    enabled: bool,
    generation: u64,
    task: Option<Task<()>>,
    running: bool,
    snapshot: Option<Snapshot>,
    ranges: Arc<[Range<usize>]>,
}

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    text: SharedString,
    mentions: Vec<Range<usize>>,
}

impl Spellcheck {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if !self.running {
            self.task = None;
        }
        self.snapshot = None;
        self.ranges = Arc::default();
    }
}

impl Composer {
    pub fn set_spellcheck_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.spellcheck.enabled != enabled {
            self.spellcheck.enabled = enabled;
            self.spellcheck.invalidate();
            cx.notify();
        }
    }

    pub(super) fn spelling_overlay(
        &mut self,
        color: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.spellcheck.enabled || self.state.active_channel().is_none() {
            return None;
        }
        let text = self.input.read(cx).value();
        let composing = self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        });
        if composing || text.trim().is_empty() {
            if self.spellcheck.snapshot.is_some() {
                self.spellcheck.invalidate();
            }
            return None;
        }
        let snapshot = Snapshot {
            mentions: if self.state.value() == text.as_str() {
                self.state
                    .draft_mentions()
                    .iter()
                    .map(|mention| mention.range.clone())
                    .collect()
            } else {
                Vec::new()
            },
            text,
        };
        if self.spellcheck.snapshot.as_ref() != Some(&snapshot) {
            if self.spellcheck.snapshot.is_some() {
                self.spellcheck.invalidate();
            }
            if !self.spellcheck.running {
                self.schedule_spelling(snapshot, window, cx);
            }
            return None;
        }
        if self.spellcheck.ranges.is_empty() || self.state.sending_here() {
            return None;
        }
        let ranges = self.spellcheck.ranges.clone();
        let input = self.input.clone();
        Some(
            canvas(
                |_, _, _| {},
                move |viewport, _, window, cx| {
                    let input = input.read(cx);
                    if input.value() != snapshot.text {
                        return;
                    }
                    // Paint after textarea prepaint has published this frame's scrolled
                    // geometry. A canvas has no hitbox and never handles editor events.
                    window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                        let style = UnderlineStyle {
                            thickness: px(1.),
                            color: Some(color),
                            wavy: true,
                        };
                        for range in ranges.iter() {
                            let (Some(start), Some(end)) = (
                                input.range_to_bounds(&(range.start..range.start)),
                                input.range_to_bounds(&(range.end..range.end)),
                            ) else {
                                continue;
                            };
                            // The public API doesn't expose per-row fragments. Don't
                            // draw a misleading line when a word spans a soft wrap.
                            if start.origin.y != end.origin.y {
                                continue;
                            }
                            let width = (end.origin.x - start.origin.x).abs();
                            if width > px(0.) {
                                window.paint_underline(
                                    point(
                                        start.origin.x.min(end.origin.x),
                                        start.bottom() - px(3.),
                                    ),
                                    width,
                                    &style,
                                );
                            }
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element(),
        )
    }

    fn schedule_spelling(&mut self, snapshot: Snapshot, window: &Window, cx: &mut Context<Self>) {
        let generation = self.spellcheck.generation;
        self.spellcheck.snapshot = Some(snapshot.clone());
        self.spellcheck.task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let ready = this.update_in(cx, |this, window, cx| {
                if this.spellcheck.generation != generation {
                    return false;
                }
                let ready = this.input.read(cx).value() == snapshot.text
                    && !this.input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    });
                this.spellcheck.running = ready;
                if !ready {
                    this.spellcheck.invalidate();
                    cx.notify();
                }
                ready
            });
            if !matches!(ready, Ok(true)) {
                return;
            }
            let ranges = cx
                .background_executor()
                .spawn(async move { check(&snapshot).unwrap_or_default() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.spellcheck.running = false;
                this.spellcheck.task = None;
                if this.spellcheck.generation == generation {
                    this.spellcheck.ranges = ranges.into();
                }
                cx.notify();
            });
        }));
    }
}

fn words(text: &str, mentions: &[Range<usize>]) -> Vec<Range<usize>> {
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
                    let range = b..b + word.len();
                    if !mentions
                        .iter()
                        .any(|mention| mention.start < range.end && range.start < mention.end)
                    {
                        ranges.push(range);
                        if ranges.len() == MAX_WORDS {
                            return ranges;
                        }
                    }
                }
                begin = i + c.len_utf8();
            }
        }
    }
    ranges
}

fn check(snapshot: &Snapshot) -> Result<Vec<Range<usize>>, ()> {
    let ranges = words(&snapshot.text, &snapshot.mentions);
    if ranges.is_empty() {
        Ok(ranges)
    } else {
        run(&snapshot.text, ranges)
    }
}

fn run(text: &str, ranges: Vec<Range<usize>>) -> Result<Vec<Range<usize>>, ()> {
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

fn parse(output: &str, ranges: Vec<Range<usize>>) -> Result<Vec<Range<usize>>, ()> {
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
            // Some dictionaries split apostrophized words. Don't mark the whole
            // word based on a record for just one fragment.
            match record {
                [line] if line.starts_with(['&', '?', '#']) => Some(range),
                _ => None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_utf8_ranges_without_chat_syntax() {
        let text = "🙂 Olá, maçã! @Mary #channel <@ABC> https://example.org `co de` ``more code`` :wave: naïve spells.";
        let extracted: Vec<_> = words(text, &[])
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(extracted, ["Olá", "maçã", "naïve", "spells"]);
    }

    #[test]
    fn maps_hunspell_records_to_original_ranges() {
        let text = "Olá wrld wrld zzzzz don't";
        let records = "@(#) Hunspell\n*\n\n& wrld 2 1: world, wild\n\n& wrld 1 1: world\n\n# zzzzz 1\n\n*\n# t 5\n\n";
        let ranges = words(text, &[]);
        let result = parse(records, ranges.clone()).unwrap();
        assert_eq!(result, ranges[1..4]);
        assert!(parse("@(#) Hunspell\n*\n\n", ranges).is_err());
    }

    #[test]
    fn excludes_entire_mention_labels_before_applying_word_limit() {
        let text = "@Mary Jane wrld";
        assert_eq!(words(text, &[0..10]), vec![11..15]);
        let long = "hello ".repeat(100);
        assert_eq!(words(&long, &[]).len(), MAX_WORDS);
    }

    #[test]
    fn starts_disabled_and_invalidation_discards_snapshot_and_results() {
        let mut state = Spellcheck::default();
        assert!(!state.enabled);
        let generation = state.generation;
        state.snapshot = Some(Snapshot {
            text: "wrld".into(),
            mentions: vec![],
        });
        state.ranges = Arc::from([0..4]);
        state.invalidate();
        assert!(state.snapshot.is_none());
        assert!(state.ranges.is_empty());
        assert_ne!(state.generation, generation);
    }

    #[test]
    #[ignore = "requires the system Hunspell CLI and an English dictionary"]
    fn system_hunspell_marks_a_typo() {
        let snapshot = Snapshot {
            text: "hello speling".into(),
            mentions: vec![],
        };
        assert_eq!(check(&snapshot).unwrap(), vec![6..13]);
    }
}
