//! Row measurement and library constants.

use super::metrics::{
    LIBRARY_BAR_HEIGHT, LIBRARY_COVER_HEIGHT, LIBRARY_ROW_SAFETY, library_info_gap,
};
use super::text::{LIBRARY_MISSING_FILE_TEXT, book_author_text, book_stats_text, book_title_text};
use super::*;

// Shape `text` at `width`/`size` with the framework's text shaper and return its
// wrapped height in px, bumping `counter` once, so CJK, emoji and combining runs wrap by
// their true advance.
pub(crate) fn measured_child_height(
    text: &str,
    width: f32,
    size: TextSize,
    counter: &std::cell::Cell<u64>,
) -> f32 {
    counter.set(counter.get() + 1);
    // An empty string still occupies one line box in the rendered column.
    enz_text_measure(text, width, size)
        .measured_height
        .max(size.line_height_px())
}

// Share shaped row heights between virtualization and painting; cache them to avoid reshaping
// the entire library per frame.
pub(crate) fn book_row_pitch(
    book: &RecentBook,
    theme: &EnzTheme,
    title_width: f32,
    counter: &std::cell::Cell<u64>,
) -> f32 {
    let spacing = &theme.spacing;
    let info_gap = library_info_gap(theme);

    // Every wrapping child, shaped for real at the info-column width.
    let title_h = measured_child_height(
        &book_title_text(book),
        title_width,
        TextSize::Title3,
        counter,
    );
    let author_h = measured_child_height(
        &book_author_text(book),
        title_width,
        TextSize::Body,
        counter,
    );
    let stats_h = measured_child_height(
        &book_stats_text(book),
        title_width,
        TextSize::Caption,
        counter,
    );

    let mut lines_h = title_h + author_h + stats_h + LIBRARY_BAR_HEIGHT;
    let mut gaps = 3.0_f32;
    if book.file_missing {
        lines_h += measured_child_height(
            LIBRARY_MISSING_FILE_TEXT,
            title_width,
            TextSize::Caption,
            counter,
        );
        gaps += 1.0;
    }
    let info_h = lines_h + gaps * info_gap;

    // The row body is the taller of the cover and the info column, plus the
    // body padding on both sides; the pitch adds the inter-row gap and a
    // sub-pixel safety margin.
    let content_h = info_h.max(LIBRARY_COVER_HEIGHT);
    content_h + 2.0 * spacing.md() + spacing.xs() + LIBRARY_ROW_SAFETY
}

// The cache key for the derived row-height table: the library
// revision (the set/order of books), the bounded content width, and a
// fingerprint of the typography + spacing ramp the pitch is built from. On an
// unchanged key the table is reused with zero shaping.
pub(crate) fn library_height_key(
    revision: u64,
    bounded_width: f32,
    tracks: usize,
    theme: &EnzTheme,
) -> u128 {
    let width_bits = bounded_width.to_bits();
    // Track count affects table length even when the measured width is unchanged.
    let layout_signature =
        library_typography_signature(theme) ^ (tracks as u32).wrapping_mul(0x9e37_79b9);
    ((revision as u128) << 64) | ((width_bits as u128) << 32) | (layout_signature as u128)
}

// Include individual spacing values so compensating geometry changes invalidate cached pitches.
pub(crate) fn library_typography_signature(theme: &EnzTheme) -> u32 {
    let vals = [
        TextSize::Title3.line_height_px(),
        TextSize::Body.line_height_px(),
        TextSize::Caption.line_height_px(),
        theme.spacing.xs(),
        theme.spacing.sm(),
        theme.spacing.md(),
        theme.spacing.lg(),
        theme.spacing.xl(),
        ListRowSurfaceTokens::from_palette(&theme.palette).outer_gutter,
    ];
    let mut hash: u32 = 2_166_136_261;
    for v in vals {
        for byte in v.to_bits().to_le_bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    hash
}

/// Stable id of the virtual list's internal scrollable. Tests and future
/// reveal operations target this id rather than depending on tree position.
pub const LIBRARY_LIST_ID: &str = "reader-library-list";

/// The screen title. Names the pane's CONTENTS ("Library" = the shelf).
pub const LIBRARY_TITLE: &str = "Library";

/// The "Continue reading" section heading: the newest books, a section of the
/// library rather than a separate screen.
pub const CONTINUE_READING_TITLE: &str = "Continue reading";

/// The "All books" section heading, shown only when the "Continue reading"
/// section is also shown, so the two groups read as distinct.
pub const ALL_BOOKS_TITLE: &str = "All books";

/// The filter field's placeholder and accessible name (`enz_search_field` labels from the
/// placeholder), distinct from "Search" so it is never confused with find-in-document.
pub const LIBRARY_FILTER_LABEL: &str = "Filter library";

/// The trailing "remove" control's accessible name (icon-only in the row).
pub const REMOVE_FROM_RECENTS: &str = crate::view::states::REMOVE_FROM_RECENTS;

/// Stable widget [`Id`](iced::advanced::widget::Id) of a book row's
/// `Remove from recents` button, keyed by the book's `path`.
///
/// After a removal the reducer re-derives the successor's id from its path through this
/// one function, so focus lands on that row.
pub fn remove_from_recents_id(path: &str) -> iced::advanced::widget::Id {
    enzui_iced_core::identity::derive_id("reader.library.remove-recent", path)
}
