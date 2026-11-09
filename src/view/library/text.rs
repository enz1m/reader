//! Book title, author, and stats text.

use super::*;

pub(crate) fn book_title_text(book: &RecentBook) -> String {
    book.title.clone()
}

// Measurement and rendering share the same author fallback.
pub(crate) fn book_author_text(book: &RecentBook) -> String {
    if book.author.is_empty() {
        "Unknown author".to_string()
    } else {
        book.author.clone()
    }
}

// The stats caption `book_item` paints under the author, shared with the row-height
// derivation so it shapes exactly the rendered text.
pub(crate) fn book_stats_text(book: &RecentBook) -> String {
    let stats = crate::plumbing::reading_stats::compute_stats(
        book.page,
        book.total,
        book.reading_time_secs,
    );
    let mut line = crate::plumbing::reading_stats::compact_stats_line(&stats, &book.format);
    let relative = crate::plumbing::reading_stats::format_last_opened(&book.last_opened);
    if !relative.is_empty() {
        line.push_str(&format!(" \u{b7} {relative}"));
    }
    line
}

// The missing-file caption `book_item` paints when `file_missing`.
pub(crate) const LIBRARY_MISSING_FILE_TEXT: &str =
    "File not found \u{2014} it may have been moved or deleted";

// Missing rows announce their unavailable state instead of an Open action.
pub(crate) fn book_row_label(title: &str, file_missing: bool) -> String {
    if file_missing {
        format!("{title} (file not found)")
    } else {
        format!("Open {title}")
    }
}
