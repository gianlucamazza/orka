/// Format an uptime value (in seconds) as a human-readable string.
/// e.g. `0s`, `45s`, `3m 22s`, `2h 03m 04s`.
pub fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Format a duration (in milliseconds) as a human-readable string.
/// Values < 1000 ms render as `142ms`; values ≥ 1000 ms render as `1.2s`.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Truncate a string to `max` display characters, appending `…` if needed.
/// The result is at most `max` Unicode characters wide.
pub fn truncate_id(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let head: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_seconds_only() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(59), "59s");
    }

    #[test]
    fn format_uptime_minutes_and_seconds() {
        assert_eq!(format_uptime(60), "1m 00s");
        assert_eq!(format_uptime(90), "1m 30s");
        assert_eq!(format_uptime(3599), "59m 59s");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(3600), "1h 00m 00s");
        assert_eq!(format_uptime(3661), "1h 01m 01s");
        assert_eq!(format_uptime(7384), "2h 03m 04s");
    }

    #[test]
    fn format_duration_ms_sub_second() {
        assert_eq!(format_duration_ms(0), "0ms");
        assert_eq!(format_duration_ms(142), "142ms");
        assert_eq!(format_duration_ms(999), "999ms");
    }

    #[test]
    fn format_duration_ms_seconds() {
        assert_eq!(format_duration_ms(1000), "1.0s");
        assert_eq!(format_duration_ms(1200), "1.2s");
        assert_eq!(format_duration_ms(10000), "10.0s");
    }

    #[test]
    fn truncate_id_short_unchanged() {
        assert_eq!(truncate_id("abc123", 10), "abc123");
    }

    #[test]
    fn truncate_id_long_is_truncated() {
        let result = truncate_id("abcdefghijklmnop", 8);
        assert!(result.ends_with('…'));
        assert_eq!(result, "abcdefg…");
    }
}
