//! Library geometry and chrome metrics.

use super::*;

// The largest desktop shelf. The cap keeps two tracks comfortably readable
// instead of stretching either row across an ultrawide window.
pub(crate) const LIBRARY_MAX_WIDTH: f32 = 1280.0;

// Use two tracks only when each retains enough width for wrapped document metadata.
pub(crate) const LIBRARY_TWO_TRACK_MIN_WIDTH: f32 = 960.0;

// The cover cell's height in px. A row is never SHORTER than this (the cover
// plus its insets), and taller whenever the title/author/caption/bar column
// needs more room.
pub(crate) const LIBRARY_COVER_HEIGHT: f32 = 72.0;

// The cover cell's width in px. Read by BOTH `book_item` (which lays the tile
// out) and the row-pitch derivation (which subtracts it from the row width to
// learn how wide the title wraps), so the two cannot drift.
pub(crate) const LIBRARY_COVER_WIDTH: f32 = 56.0;

// The progress bar's painted height (mirrors `.bar_height(6.0)` below). Part
// of the derived row pitch so the bar is never the strip that overflows.
pub(crate) const LIBRARY_BAR_HEIGHT: f32 = 6.0;

// Allow for shaping/layout rounding so the final line cannot be clipped.
pub(crate) const LIBRARY_ROW_SAFETY: f32 = 8.0;

// Chrome shared by the whole shelf before its width is divided into tracks:
// the outer content inset and the virtual list's scrollbar band.
pub(crate) fn library_shelf_chrome_width(theme: &EnzTheme) -> f32 {
    let spacing = &theme.spacing;
    let scrollbar = enzui_iced_theme::ScrollbarTokens::DEFAULT;
    2.0 * (spacing.xl() + spacing.xs()) + scrollbar.hit_width + scrollbar.content_gutter
}

// Chrome inside one book track, derived from the same row tokens and control
// geometry that [`book_item`] paints. This is kept separate from shelf chrome
// because desktop divides the remaining allocation into two bounded tracks.
pub(crate) fn library_item_chrome_width(theme: &EnzTheme) -> f32 {
    let spacing = &theme.spacing;
    let remove = ButtonSize::Sm.tokens();
    let row_surface = ListRowSurfaceTokens::from_palette(&theme.palette);
    2.0 * row_surface.outer_gutter
        + 2.0 * spacing.md()
        + LIBRARY_COVER_WIDTH
        + 2.0 * spacing.lg()
        + (2.0 * remove.padding_x + remove.icon_size)
}

pub(crate) fn library_track_count(bounded_width: f32) -> usize {
    if bounded_width >= LIBRARY_TWO_TRACK_MIN_WIDTH {
        2
    } else {
        1
    }
}

// Conservative allocation of one horizontal track after outer shelf chrome.
pub(crate) fn library_track_width(bounded_width: f32, tracks: usize, theme: &EnzTheme) -> f32 {
    let tracks = tracks.max(1) as f32;
    let track_gaps = (tracks - 1.0) * theme.spacing.lg();
    (bounded_width - library_shelf_chrome_width(theme) - track_gaps).max(0.0) / tracks
}

// Conservative width available to one title after the shelf allocation is
// divided into responsive tracks. Under-estimating is safe (a taller row);
// over-estimating could clip wrapped metadata, so shared scrollbar chrome is
// removed before division and every per-item control is removed afterwards.
pub(crate) fn library_title_width(bounded_width: f32, tracks: usize, theme: &EnzTheme) -> f32 {
    (library_track_width(bounded_width, tracks, theme) - library_item_chrome_width(theme)).max(24.0)
}

// The info column's inter-line spacing, mirroring `book_item`'s `column!`.
pub(crate) fn library_info_gap(theme: &EnzTheme) -> f32 {
    (theme.spacing.xs() + theme.spacing.sm()) / 2.0
}
