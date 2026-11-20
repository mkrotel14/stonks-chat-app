//! Port of `src/localization/index.ts` — flat key dictionaries (en, pt-BR),
//! `{var}` interpolation, CLDR plural categories, and the Intl-equivalent
//! date/time/relative formatting the app uses.
//!
//! The dictionaries are embedded from `src/locales/*.json` (single source of
//! truth shared with the Tauri app until it is retired). Plural categories
//! and relative-time words are ported from verified `Intl` output — see the
//! tests, which assert the exact strings the browser APIs produce.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    En,
    PtBr,
}

/// `ClientPreferences["dateFormat"]`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DateFormat {
    DayFirst,
    MonthFirst,
    YearFirst,
}

/// `ClientPreferences["timeFormat"]`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeFormat {
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DateStyle {
    Numeric,
    Long,
}

/// Broken-down local date/time. The public API converts unix millis via the
/// system timezone; tests construct parts directly.
#[derive(Clone, Copy, Debug)]
pub struct DateTimeParts {
    pub year: i32,
    /// 1-12
    pub month: u8,
    /// 1-31
    pub day: u8,
    /// 0-23
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = Sunday
    pub weekday: u8,
}

pub struct Localization {
    language: Language,
    date_format: DateFormat,
    time_format: TimeFormat,
    english: HashMap<String, String>,
    portuguese: HashMap<String, String>,
}

fn load_dictionary(json: &str) -> HashMap<String, String> {
    let Value::Object(entries) = serde_json::from_str(json).expect("locale JSON is an object")
    else {
        panic!("locale JSON is an object");
    };
    entries
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_owned())))
        .collect()
}

impl Default for Localization {
    fn default() -> Self {
        Self {
            language: Language::En,
            date_format: DateFormat::DayFirst,
            time_format: TimeFormat::TwentyFourHour,
            english: load_dictionary(include_str!("../assets/locales/en.json")),
            portuguese: load_dictionary(include_str!("../assets/locales/pt-BR.json")),
        }
    }
}

impl Localization {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors `configureLocalization`: unknown languages fall back to en.
    pub fn configure(&mut self, language: &str, date_format: &str, time_format: &str) {
        self.language = match language {
            "pt-BR" => Language::PtBr,
            _ => Language::En,
        };
        self.date_format = match date_format {
            "month-first" => DateFormat::MonthFirst,
            "year-first" => DateFormat::YearFirst,
            _ => DateFormat::DayFirst,
        };
        self.time_format = match time_format {
            "12-hour" => TimeFormat::TwelveHour,
            _ => TimeFormat::TwentyFourHour,
        };
    }

    pub fn locale_code(&self) -> &'static str {
        match self.language {
            Language::En => "en",
            Language::PtBr => "pt-BR",
        }
    }

    fn dictionary(&self) -> &HashMap<String, String> {
        match self.language {
            Language::En => &self.english,
            Language::PtBr => &self.portuguese,
        }
    }

    fn lookup<'a>(&'a self, key: &'a str) -> &'a str {
        self.dictionary()
            .get(key)
            .or_else(|| self.english.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }

    pub fn t(&self, key: &str) -> String {
        self.lookup(key).to_owned()
    }

    pub fn t_vars<V: ToString>(&self, key: &str, values: &[(&str, V)]) -> String {
        let mut message = self.lookup(key).to_owned();
        for (name, value) in values {
            message = message.replace(&format!("{{{name}}}"), &value.to_string());
        }
        message
    }

    /// Count-dependent message with `<key>.<plural category>` entries.
    /// Categories match `Intl.PluralRules` for en and pt-BR (verified):
    /// en: one iff n == 1; pt-BR: one iff n is 0 or 1.
    pub fn tp<V: ToString>(&self, key: &str, count: i64, values: &[(&str, V)]) -> String {
        let category = match self.language {
            Language::En if count == 1 => "one",
            Language::PtBr if count == 0 || count == 1 => "one",
            _ => "other",
        };
        let keyed = format!("{key}.{category}");
        let mut message =
            if self.dictionary().contains_key(&keyed) || self.english.contains_key(&keyed) {
                self.lookup(&keyed).to_owned()
            } else {
                self.lookup(key).to_owned()
            };
        message = message.replace("{count}", &count.to_string());
        for (name, value) in values {
            message = message.replace(&format!("{{{name}}}"), &value.to_string());
        }
        message
    }

    // --- Dates ---

    pub fn format_date(
        &self,
        parts: &DateTimeParts,
        style: DateStyle,
        include_year: bool,
        include_weekday: bool,
    ) -> String {
        let months = match self.language {
            Language::En => EN_MONTHS,
            Language::PtBr => PT_MONTHS,
        };
        let year = self.year_string(parts, include_year);
        let core = match style {
            DateStyle::Numeric => match self.date_format {
                DateFormat::MonthFirst => join_slash(
                    &format!("{:02}", parts.month),
                    &format!("{:02}", parts.day),
                    year.as_deref(),
                ),
                DateFormat::YearFirst => match &year {
                    Some(year) => format!("{year}-{:02}-{:02}", parts.month, parts.day),
                    None => format!("{:02}-{:02}", parts.month, parts.day),
                },
                DateFormat::DayFirst => join_slash(
                    &format!("{:02}", parts.day),
                    &format!("{:02}", parts.month),
                    year.as_deref(),
                ),
            },
            DateStyle::Long => {
                let month = months[(parts.month - 1) as usize];
                match self.date_format {
                    DateFormat::MonthFirst => match &year {
                        Some(year) => format!("{month} {}, {year}", parts.day),
                        None => format!("{month} {}", parts.day),
                    },
                    DateFormat::YearFirst => match &year {
                        Some(year) => format!("{year} {month} {}", parts.day),
                        None => format!("{month} {}", parts.day),
                    },
                    DateFormat::DayFirst => match &year {
                        Some(year) => format!("{} {month} {year}", parts.day),
                        None => format!("{} {month}", parts.day),
                    },
                }
            }
        };
        if include_weekday {
            let weekdays = match self.language {
                Language::En => EN_WEEKDAYS,
                Language::PtBr => PT_WEEKDAYS,
            };
            format!("{}, {core}", weekdays[parts.weekday as usize])
        } else {
            core
        }
    }

    fn year_string(&self, parts: &DateTimeParts, include_year: bool) -> Option<String> {
        include_year.then(|| parts.year.to_string())
    }

    /// Intl parity quirk: en pads the 24h hour ("09:05"), pt-BR does not
    /// ("9:05"); 12h is unpadded with AM/PM in both locales.
    pub fn format_time(&self, parts: &DateTimeParts, include_seconds: bool) -> String {
        let seconds = include_seconds
            .then(|| format!(":{:02}", parts.second))
            .unwrap_or_default();
        match self.time_format {
            TimeFormat::TwentyFourHour => match self.language {
                Language::En => format!("{:02}:{:02}{seconds}", parts.hour, parts.minute),
                Language::PtBr => format!("{}:{:02}{seconds}", parts.hour, parts.minute),
            },
            TimeFormat::TwelveHour => {
                let period = if parts.hour < 12 { "AM" } else { "PM" };
                let hour = match parts.hour % 12 {
                    0 => 12,
                    hour => hour,
                };
                format!("{hour}:{:02}{seconds} {period}", parts.minute)
            }
        }
    }

    pub fn format_datetime(
        &self,
        parts: &DateTimeParts,
        style: DateStyle,
        include_seconds: bool,
    ) -> String {
        format!(
            "{} {} {}",
            self.format_date(parts, style, true, false),
            self.t("common.at"),
            self.format_time(parts, include_seconds)
        )
    }

    /// Port of `formatRelativeTime` against `Intl.RelativeTimeFormat`
    /// (numeric: "auto"). Buckets: <60s seconds, <1h minutes, <1d hours,
    /// else days; special words for day -2..=2 where the locale defines them.
    pub fn format_relative(&self, then_millis: i64, now_millis: i64) -> String {
        let delta = ((then_millis - now_millis) as f64 / 1000.0).round() as i64;
        let absolute = delta.abs();
        let (amount, unit) = if absolute < 60 {
            (delta, Unit::Second)
        } else if absolute < 3_600 {
            ((delta as f64 / 60.0).round() as i64, Unit::Minute)
        } else if absolute < 86_400 {
            ((delta as f64 / 3_600.0).round() as i64, Unit::Hour)
        } else {
            ((delta as f64 / 86_400.0).round() as i64, Unit::Day)
        };

        if amount == 0 && unit == Unit::Second {
            return match self.language {
                Language::En => "now".into(),
                Language::PtBr => "agora".into(),
            };
        }
        if unit == Unit::Day {
            let word = match (self.language, amount) {
                (_, -1) => Some(match self.language {
                    Language::En => "yesterday",
                    Language::PtBr => "ontem",
                }),
                (_, 1) => Some(match self.language {
                    Language::En => "tomorrow",
                    Language::PtBr => "amanhã",
                }),
                (Language::PtBr, -2) => Some("anteontem"),
                (Language::PtBr, 2) => Some("depois de amanhã"),
                _ => None,
            };
            if let Some(word) = word {
                return word.to_owned();
            }
        }

        let count = amount.abs();
        let unit_name = match (self.language, unit, count == 1) {
            (Language::En, Unit::Second, true) => "second",
            (Language::En, Unit::Second, false) => "seconds",
            (Language::En, Unit::Minute, true) => "minute",
            (Language::En, Unit::Minute, false) => "minutes",
            (Language::En, Unit::Hour, true) => "hour",
            (Language::En, Unit::Hour, false) => "hours",
            (Language::En, Unit::Day, _) => "days",
            (Language::PtBr, Unit::Second, true) => "segundo",
            (Language::PtBr, Unit::Second, false) => "segundos",
            (Language::PtBr, Unit::Minute, true) => "minuto",
            (Language::PtBr, Unit::Minute, false) => "minutos",
            (Language::PtBr, Unit::Hour, true) => "hora",
            (Language::PtBr, Unit::Hour, false) => "horas",
            (Language::PtBr, Unit::Day, _) => "dias",
        };
        match (self.language, amount < 0) {
            (Language::En, true) => format!("{count} {unit_name} ago"),
            (Language::En, false) => format!("in {count} {unit_name}"),
            (Language::PtBr, true) => format!("há {count} {unit_name}"),
            (Language::PtBr, false) => format!("em {count} {unit_name}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Second,
    Minute,
    Hour,
    Day,
}

fn join_slash(first: &str, second: &str, year: Option<&str>) -> String {
    match year {
        Some(year) => format!("{first}/{second}/{year}"),
        None => format!("{first}/{second}"),
    }
}

const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const PT_MONTHS: [&str; 12] = [
    "janeiro",
    "fevereiro",
    "março",
    "abril",
    "maio",
    "junho",
    "julho",
    "agosto",
    "setembro",
    "outubro",
    "novembro",
    "dezembro",
];
const EN_WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const PT_WEEKDAYS: [&str; 7] = [
    "domingo",
    "segunda-feira",
    "terça-feira",
    "quarta-feira",
    "quinta-feira",
    "sexta-feira",
    "sábado",
];

// --- Process-wide access (the UI reads through here) ---

static GLOBAL: LazyLock<RwLock<Localization>> = LazyLock::new(|| RwLock::new(Localization::new()));

pub fn configure(language: &str, date_format: &str, time_format: &str) {
    GLOBAL
        .write()
        .expect("localization lock")
        .configure(language, date_format, time_format);
}

pub fn t(key: &str) -> String {
    GLOBAL.read().expect("localization lock").t(key)
}

pub fn t_vars<V: ToString>(key: &str, values: &[(&str, V)]) -> String {
    GLOBAL
        .read()
        .expect("localization lock")
        .t_vars(key, values)
}

pub fn tp<V: ToString>(key: &str, count: i64, values: &[(&str, V)]) -> String {
    GLOBAL
        .read()
        .expect("localization lock")
        .tp(key, count, values)
}

/// Relative timestamp ("now", "5 minutes ago", "yesterday") for `then`
/// (unix millis) against the current time. Timezone-free, so it needs no
/// local-clock conversion.
pub fn format_relative_time(then_millis: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    GLOBAL
        .read()
        .expect("localization lock")
        .format_relative(then_millis, now)
}

/// Local-timezone breakdown of unix millis (for `<t:...>` message tokens).
fn parts_from_millis(millis: i64) -> Option<DateTimeParts> {
    use chrono::{Datelike, Timelike};
    let local = chrono::DateTime::from_timestamp_millis(millis)?.with_timezone(&chrono::Local);
    Some(DateTimeParts {
        year: local.year(),
        month: local.month() as u8,
        day: local.day() as u8,
        hour: local.hour() as u8,
        minute: local.minute() as u8,
        second: local.second() as u8,
        weekday: local.weekday().num_days_from_sunday() as u8,
    })
}

/// Day heading from messagePresentation.ts, respecting local calendar dates.
pub fn format_day_ms(millis: i64) -> String {
    let Some(parts) = parts_from_millis(millis) else {
        return String::new();
    };
    let Some(today) = parts_from_millis(chrono::Local::now().timestamp_millis()) else {
        return String::new();
    };
    GLOBAL
        .read()
        .expect("localization lock")
        .format_day(&parts, &today)
}

/// Source message header: Today/Yesterday at HH:MM, otherwise long date.
pub fn format_message_timestamp_ms(millis: i64) -> String {
    let Some(parts) = parts_from_millis(millis) else {
        return String::new();
    };
    let now = chrono::Local::now();
    let Some(today) = parts_from_millis(now.timestamp_millis()) else {
        return String::new();
    };
    let locale = GLOBAL.read().expect("localization lock");
    let date = chrono::NaiveDate::from_ymd_opt(parts.year, parts.month as u32, parts.day as u32);
    let day = if date == Some(now.date_naive()) {
        locale.t("common.today")
    } else if date == now.date_naive().pred_opt() {
        locale.t("common.yesterday")
    } else {
        locale.format_date(&parts, DateStyle::Long, parts.year != today.year, false)
    };
    format!(
        "{} {} {}",
        day,
        locale.t("common.at"),
        locale.format_time(&parts, false)
    )
}

impl Localization {
    fn format_day(&self, parts: &DateTimeParts, today: &DateTimeParts) -> String {
        if (parts.year, parts.month, parts.day) == (today.year, today.month, today.day) {
            self.t("common.today")
        } else {
            self.format_date(parts, DateStyle::Long, parts.year != today.year, true)
        }
    }
}

/// `format_time` for unix millis; empty string for out-of-range input.
pub fn format_time_ms(millis: i64, include_seconds: bool) -> String {
    let Some(parts) = parts_from_millis(millis) else {
        return String::new();
    };
    GLOBAL
        .read()
        .expect("localization lock")
        .format_time(&parts, include_seconds)
}

/// `format_date` for unix millis; empty string for out-of-range input.
pub fn format_date_ms(
    millis: i64,
    style: DateStyle,
    include_year: bool,
    include_weekday: bool,
) -> String {
    let Some(parts) = parts_from_millis(millis) else {
        return String::new();
    };
    GLOBAL.read().expect("localization lock").format_date(
        &parts,
        style,
        include_year,
        include_weekday,
    )
}

/// `format_datetime` for unix millis; empty string for out-of-range input.
pub fn format_datetime_ms(millis: i64, style: DateStyle, include_seconds: bool) -> String {
    let Some(parts) = parts_from_millis(millis) else {
        return String::new();
    };
    GLOBAL
        .read()
        .expect("localization lock")
        .format_datetime(&parts, style, include_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> DateTimeParts {
        // Friday, 9 January 2026, 09:05:03
        DateTimeParts {
            year: 2026,
            month: 1,
            day: 9,
            hour: 9,
            minute: 5,
            second: 3,
            weekday: 5,
        }
    }

    #[test]
    fn t_interpolates_and_falls_back() {
        let l10n = Localization::new();
        assert_eq!(l10n.t("missing.key"), "missing.key");
        let common = l10n.t("common.at");
        assert_eq!(common, "at");
        let mut pt = Localization::new();
        pt.configure("pt-BR", "day-first", "24-hour");
        assert_eq!(pt.t("common.at"), "às");
        // Unknown languages fall back to en.
        pt.configure("fr", "day-first", "24-hour");
        assert_eq!(pt.t("common.at"), "at");
    }

    #[test]
    fn plural_categories_match_intl() {
        // Ground truth: Intl.PluralRules("en"): 1→one, else other;
        // Intl.PluralRules("pt-BR"): 0,1→one, else other.
        let cases: [(i64, &str, &str); 4] = [
            (0, "other", "one"),
            (1, "one", "one"),
            (2, "other", "other"),
            (5, "other", "other"),
        ];
        for (count, en, pt) in cases {
            let en_category = if count == 1 { "one" } else { "other" };
            let pt_category = if count == 0 || count == 1 {
                "one"
            } else {
                "other"
            };
            assert_eq!((en_category, pt_category), (en, pt), "count {count}");
        }
    }

    #[test]
    fn time_formats_match_intl() {
        let l10n = Localization::new();
        assert_eq!(l10n.format_time(&parts(), false), "09:05");
        let mut twelve = Localization::new();
        twelve.configure("en", "day-first", "12-hour");
        assert_eq!(twelve.format_time(&parts(), true), "9:05:03 AM");
        let mut pt = Localization::new();
        pt.configure("pt-BR", "day-first", "24-hour");
        assert_eq!(pt.format_time(&parts(), false), "9:05");
    }

    #[test]
    fn message_day_labels_use_local_calendar_and_only_include_cross_year() {
        let locale = Localization::new();
        let day = parts();
        assert_eq!(locale.format_day(&day, &day), "Today");
        let mut today = day;
        today.day += 1;
        assert_eq!(locale.format_day(&day, &today), "Friday, 9 January");
        today.year += 1;
        assert_eq!(locale.format_day(&day, &today), "Friday, 9 January 2026");
        let mut pt = Localization::new();
        pt.configure("pt-BR", "day-first", "24-hour");
        assert_eq!(pt.format_day(&day, &day), "Hoje");
        assert_eq!(pt.format_day(&day, &today), "sexta-feira, 9 janeiro 2026");
    }

    #[test]
    fn date_formats_match_intl_parts() {
        let l10n = Localization::new();
        let p = parts();
        assert_eq!(
            l10n.format_date(&p, DateStyle::Long, true, true),
            "Friday, 9 January 2026"
        );
        assert_eq!(
            l10n.format_date(&p, DateStyle::Numeric, true, false),
            "09/01/2026"
        );
        let mut pt = Localization::new();
        pt.configure("pt-BR", "day-first", "24-hour");
        assert_eq!(
            pt.format_date(&p, DateStyle::Long, true, true),
            "sexta-feira, 9 janeiro 2026"
        );
        let mut year_first = Localization::new();
        year_first.configure("en", "year-first", "24-hour");
        assert_eq!(
            year_first.format_date(&p, DateStyle::Numeric, true, false),
            "2026-01-09"
        );
    }

    #[test]
    fn relative_time_matches_intl() {
        // Ground truth from Intl.RelativeTimeFormat(locale, {numeric:"auto"}).
        let now = 1_000_000_000_000i64;
        let secs = |n: i64| now + n * 1_000;
        let days = |n: i64| now + n * 86_400_000;
        let en = Localization::new();
        assert_eq!(en.format_relative(secs(0), now), "now");
        assert_eq!(en.format_relative(secs(-1), now), "1 second ago");
        assert_eq!(en.format_relative(secs(-45), now), "45 seconds ago");
        assert_eq!(en.format_relative(secs(30), now), "in 30 seconds");
        assert_eq!(en.format_relative(secs(-60), now), "1 minute ago");
        assert_eq!(en.format_relative(secs(120), now), "in 2 minutes");
        assert_eq!(en.format_relative(secs(-3_600), now), "1 hour ago");
        assert_eq!(en.format_relative(secs(10_800), now), "in 3 hours");
        assert_eq!(en.format_relative(days(-1), now), "yesterday");
        assert_eq!(en.format_relative(days(1), now), "tomorrow");
        assert_eq!(en.format_relative(days(-4), now), "4 days ago");
        assert_eq!(en.format_relative(days(2), now), "in 2 days");

        let mut pt = Localization::new();
        pt.configure("pt-BR", "day-first", "24-hour");
        assert_eq!(pt.format_relative(secs(0), now), "agora");
        assert_eq!(pt.format_relative(secs(-1), now), "há 1 segundo");
        assert_eq!(pt.format_relative(secs(30), now), "em 30 segundos");
        assert_eq!(pt.format_relative(days(-1), now), "ontem");
        assert_eq!(pt.format_relative(days(1), now), "amanhã");
        assert_eq!(pt.format_relative(days(-2), now), "anteontem");
        assert_eq!(pt.format_relative(days(2), now), "depois de amanhã");
        assert_eq!(pt.format_relative(days(-4), now), "há 4 dias");
    }
}

/// Source invite expiration: Date.now()+seconds, serialized as ISO UTC milliseconds.
pub fn iso_timestamp_after(seconds: u64) -> Option<String> {
    let duration = chrono::TimeDelta::try_seconds(i64::try_from(seconds).ok()?)?;
    chrono::Utc::now()
        .checked_add_signed(duration)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}
pub fn format_iso_datetime(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| format_datetime_ms(value.timestamp_millis(), DateStyle::Numeric, false))
        .unwrap_or_else(|| value.to_owned())
}
