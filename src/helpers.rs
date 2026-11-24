// SPDX-License-Identifier: MIT

//! Small pure helpers shared by views and handlers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, now.
pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// How long ago `then` was, seen from `now`, as a feed shows it: `now`, `5m`,
/// `3h`, `2d`, `6w`, `1y`. A time in the future (a clock that runs ahead)
/// reads as `now`.
pub fn relative_time(now: u64, then: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const YEAR: u64 = 365 * DAY;

    let elapsed = now.saturating_sub(then);
    match elapsed {
        e if e < MINUTE => "now".to_string(),
        e if e < HOUR => format!("{}m", e / MINUTE),
        e if e < DAY => format!("{}h", e / HOUR),
        e if e < WEEK => format!("{}d", e / DAY),
        e if e < YEAR => format!("{}w", e / WEEK),
        e => format!("{}y", e / YEAR),
    }
}

/// At most `max` characters of `text`, with an ellipsis when it was longer.
pub fn truncate_chars(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    match text.char_indices().nth(max) {
        Some((cut, _)) => {
            let head = text.get(..cut).unwrap_or(text);
            std::borrow::Cow::Owned(format!("{}\u{2026}", head.trim_end()))
        }
        None => std::borrow::Cow::Borrowed(text),
    }
}

/// `bytes` as a size someone reads: in the unit that leaves one to three
/// digits before the point, with one after it past the first unit.
pub fn readable_size(bytes: usize) -> String {
    let mut size = bytes as f64;
    let mut unit = "B";
    for larger in ["kB", "MB", "GB"] {
        if size < 1000.0 {
            break;
        }
        size /= 1000.0;
        unit = larger;
    }
    if unit == "B" { format!("{bytes} B") } else { format!("{size:.1} {unit}") }
}

/// A count short enough to sit beside an icon: `999`, `1.2k`, `15k`,
/// `3.4M`. Rounded down, so it never says more than there is.
pub fn compact_count(count: u64) -> String {
    let scaled = |unit: u64, suffix: &str| {
        let (whole, tenth) = (count / unit, count % unit * 10 / unit);
        if whole < 10 && tenth > 0 { format!("{whole}.{tenth}{suffix}") } else { format!("{whole}{suffix}") }
    };
    match count {
        0..1_000 => count.to_string(),
        1_000..1_000_000 => scaled(1_000, "k"),
        _ => scaled(1_000_000, "M"),
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_count, readable_size, relative_time, truncate_chars};

    #[test]
    fn a_size_is_said_in_the_unit_that_keeps_it_short() {
        assert_eq!(readable_size(0), "0 B");
        assert_eq!(readable_size(999), "999 B");
        assert_eq!(readable_size(1_000), "1.0 kB");
        assert_eq!(readable_size(21_400_000), "21.4 MB");
        assert_eq!(readable_size(3_000_000_000), "3.0 GB");
        assert_eq!(readable_size(5_000_000_000_000), "5000.0 GB");
    }

    #[test]
    fn a_count_is_cut_to_what_fits_beside_an_icon() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_000), "1k");
        assert_eq!(compact_count(1_250), "1.2k");
        assert_eq!(compact_count(9_999), "9.9k");
        assert_eq!(compact_count(15_800), "15k");
        assert_eq!(compact_count(999_999), "999k");
        assert_eq!(compact_count(3_400_000), "3.4M");
        assert_eq!(compact_count(21_000_000), "21M");
    }

    #[test]
    fn a_time_reads_in_the_largest_unit_that_fits() {
        let now = 1_000_000_000;
        let ago = |seconds: u64| relative_time(now, now - seconds);
        assert_eq!(ago(0), "now");
        assert_eq!(ago(59), "now");
        assert_eq!(ago(60), "1m");
        assert_eq!(ago(59 * 60 + 59), "59m");
        assert_eq!(ago(3 * 3600), "3h");
        assert_eq!(ago(2 * 86_400), "2d");
        assert_eq!(ago(6 * 7 * 86_400), "6w");
        assert_eq!(ago(400 * 86_400), "1y");
    }

    #[test]
    fn a_time_ahead_of_the_clock_reads_as_now() {
        assert_eq!(relative_time(100, 500), "now");
    }

    #[test]
    fn long_text_is_cut_on_a_character_and_marked() {
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("exactly ten", 11), "exactly ten");
        assert_eq!(truncate_chars("olá avestruz", 3), "olá…");
        assert_eq!(truncate_chars("one two three", 4), "one…");
    }
}
