//! Reading progress and elapsed-time statistics derived from persisted records.

/// Computed reading statistics for a single book.
#[derive(Clone, Debug, PartialEq)]
pub struct BookStats {
    /// Current page/chapter (1-indexed for display).
    pub current_pos: usize,
    /// Total pages/chapters.
    pub total: usize,
    /// Percentage read (0..100, integer).
    pub percent_read: u32,
    /// Accumulated reading time in seconds.
    pub reading_time_secs: u64,
    /// Human-readable reading time string (e.g. "2h 15m", "45m", "< 1m").
    pub reading_time_display: String,
}

/// Compute reading statistics from raw progress data.
///
/// - `page` is 0-indexed (as stored in `ProgressRecord`).
/// - `total` is the total number of pages/chapters.
/// - `reading_time_secs` is the accumulated reading time.
pub fn compute_stats(page: usize, total: usize, reading_time_secs: u64) -> BookStats {
    let current_pos = page + 1; // 1-indexed for display

    let percent_read = if total == 0 {
        0
    } else {
        ((current_pos as f64 / total as f64) * 100.0)
            .round()
            .min(100.0) as u32
    };

    let reading_time_display = format_reading_time(reading_time_secs);

    BookStats {
        current_pos,
        total,
        percent_read,
        reading_time_secs,
        reading_time_display,
    }
}

/// Format a reading time in seconds to a human-readable string.
///
/// - 0 → "No reading time"
/// - < 60s → "< 1m"
/// - 60s..3600s → "Nm"
/// - 3600s+ → "Nh Mm"
/// - 86400s+ → "Nd Nh"
pub fn format_reading_time(secs: u64) -> String {
    if secs == 0 {
        return "No reading time".to_string();
    }
    if secs < 60 {
        return "< 1m".to_string();
    }
    let minutes = secs / 60;
    let hours = minutes / 60;
    let days = hours / 24;

    if days > 0 {
        let rem_hours = hours % 24;
        if rem_hours > 0 {
            format!("{}d {}h", days, rem_hours)
        } else {
            format!("{}d", days)
        }
    } else if hours > 0 {
        let rem_mins = minutes % 60;
        if rem_mins > 0 {
            format!("{}h {}m", hours, rem_mins)
        } else {
            format!("{}h", hours)
        }
    } else {
        format!("{}m", minutes)
    }
}

/// Compact one-line stats string for the "Continue reading" card.
///
/// Example: "30% · 3 of 10 · EPUB · 1h 23m read".
pub fn compact_stats_line(stats: &BookStats, format: &str) -> String {
    let mut line = format!(
        "{}% · {} of {} · {}",
        stats.percent_read,
        stats.current_pos,
        stats.total.max(1),
        format.to_uppercase()
    );
    if stats.reading_time_secs > 0 {
        line.push_str(&format!(" · {} read", stats.reading_time_display));
    }
    line
}

/// A short relative-time label for a recents card ("just now", "5m ago",
/// "3h ago", "2d ago"), or the bare date for anything older than ~30 days.
/// Empty input → empty string. Uses the current wall clock.
pub fn format_last_opened(iso: &str) -> String {
    format_relative_time(iso, chrono::Utc::now())
}

/// Pure core of [`format_last_opened`], with `now` supplied so it is
/// deterministically testable. An unparseable timestamp degrades to its date
/// portion (before the `T`), never a panic.
pub fn format_relative_time(iso: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    if iso.is_empty() {
        return String::new();
    }
    let created = match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return iso.split('T').next().unwrap_or(iso).to_string(),
    };
    let secs = now.signed_duration_since(created).num_seconds();
    // A future timestamp (clock skew, secs < 0) collapses into the same
    // "just now" bucket rather than a nonsensical negative age.
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 30 * 86400 {
        format!("{}d ago", secs / 86400)
    } else {
        iso.split('T').next().unwrap_or(iso).to_string()
    }
}
