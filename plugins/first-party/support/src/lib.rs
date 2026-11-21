//! What DeckPilot's first-party plugins for a server the user enters
//! (`deckpilot.gitlab`, `deckpilot.jira`, and `deckpilot.github` for a
//! GitHub Enterprise Server) share: the URLs they build and
//! check ([`server`]), the action's time limit counted with the server's
//! `Date` headers ([`Deadline`]; a guest has no clock), and how text from a
//! server is shown.
//!
//! Each function is a port of what the former built-in `gitlab` and
//! `jira` modules did (removed in WP-C5), so a plugin gives the built-in's
//! results on the same answers.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;

pub mod server;

pub use server::{Server, WebPage};

/// Longest text one output can carry (ABI 1 `MAX_STRING_LEN`, in bytes).
pub const MAX_OUTPUT_TEXT: usize = 16 * 1024;
/// Largest answer the agent hands to a plugin, in KiB.
pub const MAX_ANSWER_KIB: usize = 512;

/// Text from a server as shown in one line: control characters and
/// bidirectional overrides become spaces, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                c
            }
        })
        .take(max)
        .collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out.trim().to_owned()
}

/// `lines` joined by newlines, as many as fit into one output; and how
/// many were kept.
pub fn bounded_lines(lines: &[String]) -> (String, usize) {
    let mut text = String::new();
    let mut kept = 0;
    for line in lines {
        let needed = line.len() + usize::from(kept > 0);
        if text.len() + needed > MAX_OUTPUT_TEXT {
            break;
        }
        if kept > 0 {
            text.push('\n');
        }
        text.push_str(line);
        kept += 1;
    }
    (text, kept)
}

/// [`bounded_lines`], with a last line "… and N more" when not all fit.
pub fn bounded_lines_with_rest(lines: &[String]) -> String {
    let (mut text, kept) = bounded_lines(lines);
    if kept < lines.len() {
        let more = format!("… and {} more", lines.len() - kept);
        while text.len() + more.len() + 1 > MAX_OUTPUT_TEXT {
            match text.rfind('\n') {
                Some(at) => text.truncate(at),
                None => text.clear(),
            }
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&more);
    }
    text
}

/// A count as an output value.
pub fn saturating_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// An IMF-fixdate (RFC 9110 §5.6.7, `Sun, 06 Nov 1994 08:49:37 GMT`) as
/// Unix seconds.
pub fn http_date(value: &str) -> Option<u64> {
    let (_, rest) = value.trim().split_once(", ")?;
    let parts: Vec<&str> = rest.split(' ').collect();
    let [day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    let day: u64 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m == month)?;
    let year: u64 = year.parse().ok().filter(|y| *y >= 1970 && *y < 10_000)?;
    let clock: Vec<u64> = time
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    if *hour > 23 || *minute > 59 || *second > 60 {
        return None;
    }
    let leap = |y: u64| (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400);
    let days_before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut days: u64 = (1970..year).map(|y| if leap(y) { 366 } else { 365 }).sum();
    days += days_before_month[month];
    if month > 1 && leap(year) {
        days += 1;
    }
    days += day - 1;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The seconds a `Retry-After` header asks to wait: a number of seconds,
/// or an HTTP date counted from `now` (the answer's `Date`). `None` for a
/// form not understood, and for a date without a `now` to count from.
pub fn retry_after(value: &str, now: Option<u64>) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    Some(http_date(value)?.saturating_sub(now?))
}

/// The time limit of one action, counted from the server's first answer
/// with the `Date` headers of its answers (one-second steps): a guest has
/// no clock. Each request also has the agent's own limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    seconds: u64,
    first: Option<u64>,
    latest: Option<u64>,
}

impl Deadline {
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds,
            first: None,
            latest: None,
        }
    }

    /// Notes the `Date` of `response` and returns it.
    pub fn observe(&mut self, response: &Response) -> Option<u64> {
        let date = response.header("date").and_then(http_date);
        if date.is_some() {
            self.first = self.first.or(date);
            self.latest = date;
        }
        date
    }

    /// Whether the limit has passed since the first answer.
    pub fn passed(&self) -> bool {
        matches!((self.first, self.latest), (Some(first), Some(latest))
            if latest.saturating_sub(first) >= self.seconds)
    }
}

/// Whether a request failed because the answer is larger than the agent
/// hands to a plugin ([`MAX_ANSWER_KIB`]).
pub fn too_large(error: &CapabilityError) -> bool {
    error.kind() == CapabilityErrorKind::Failed && error.detail().contains("larger than")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_one_clean_line() {
        assert_eq!(clean("a\nb\u{202e}c", 10), "a b c");
        assert_eq!(clean("abcdef", 3), "abc…");
    }

    #[test]
    fn http_dates_are_unix_seconds() {
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            http_date("Thu, 12 Nov 2026 07:00:45 GMT"),
            Some(1_794_466_845)
        );
        assert_eq!(
            http_date("Thu, 29 Feb 2024 12:00:00 GMT"),
            Some(1_709_208_000)
        );
        for bad in [
            "",
            "later",
            "12 Nov 2026 07:00:45 GMT",
            "Thu, 12 Nov 2026 07:00:45 CET",
            "Thu, 12 Foo 2026 07:00:45 GMT",
            "Thu, 12 Nov 2026 25:00:45 GMT",
        ] {
            assert_eq!(http_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn retry_after_is_seconds_or_a_date() {
        assert_eq!(retry_after(" 30 ", None), Some(30));
        assert_eq!(
            retry_after("Thu, 12 Nov 2026 07:00:45 GMT", Some(1_794_466_800)),
            Some(45)
        );
        // A date in the past: no wait.
        assert_eq!(
            retry_after("Thu, 12 Nov 2026 07:00:45 GMT", Some(1_794_466_900)),
            Some(0)
        );
        assert_eq!(retry_after("Thu, 12 Nov 2026 07:00:45 GMT", None), None);
        assert_eq!(retry_after("later", Some(0)), None);
    }

    #[test]
    fn lines_are_bounded_to_one_output() {
        let lines: Vec<String> = (0..3).map(|i| format!("line {i}")).collect();
        assert_eq!(bounded_lines(&lines), ("line 0\nline 1\nline 2".into(), 3));
        assert_eq!(bounded_lines_with_rest(&lines), "line 0\nline 1\nline 2");
        let long: Vec<String> = (0..100).map(|_| "x".repeat(1_000)).collect();
        let (text, kept) = bounded_lines(&long);
        assert_eq!(kept, 16);
        assert!(text.len() <= MAX_OUTPUT_TEXT);
        let with_rest = bounded_lines_with_rest(&long);
        assert!(with_rest.len() <= MAX_OUTPUT_TEXT);
        assert!(with_rest.ends_with("\n… and 84 more"));
    }
}
