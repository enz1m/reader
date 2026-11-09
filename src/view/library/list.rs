//! Virtualised book shelf.

use super::cards::book_item;
use super::metrics::{LIBRARY_COVER_HEIGHT, library_title_width, library_track_width};
use super::rows::{book_row_pitch, library_height_key};
use super::tracks::LibraryTracks;
use super::*;

// The virtualised shelf. The provider is borrowed from application state and
// keyed by each book's canonical path; no row projection is allocated here.
pub(crate) fn book_list<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    content_height: f32,
    bounded_width: f32,
    tracks: usize,
) -> Element<'a, Msg> {
    let row_gap = theme.spacing.xs();
    let track_gap = theme.spacing.lg();
    let books = model.library_rows().rows();
    let provider = std::rc::Rc::new(LibraryTracks {
        books,
        columns: tracks,
    });

    // Shape each book at its actual responsive track width, then reserve the
    // tallest member's pitch for that horizontal track. The table remains
    // cache-bounded and the virtual list still realises by viewport, in groups
    // of at most two books.
    let track_width = library_track_width(bounded_width, tracks, theme);
    let title_width = library_title_width(bounded_width, tracks, theme);
    let key = library_height_key(model.library_revision, bounded_width, tracks, theme);
    let heights = {
        let theme_ref = theme;
        model.library_height_cache.get(key, move |counter| {
            let geometry = LibraryTracks {
                books,
                columns: tracks,
            };
            (0..geometry.track_count())
                .map(|index| {
                    let (start, len) = geometry.bounds(index).expect("in-range library track");
                    books[start..start + len]
                        .iter()
                        .map(|book| book_row_pitch(book, theme_ref, title_width, counter))
                        .fold(LIBRARY_COVER_HEIGHT, f32::max)
                })
                .collect()
        })
    };
    let row_heights = std::rc::Rc::clone(&heights);
    enz_virtual_list_variable(provider, heights.iter().copied(), move |index, track| {
        // Read the committed track pitch from the same cached table used by
        // virtualization. A shorter neighbour simply leaves quiet space below
        // itself; neither row can collide with the next horizontal track.
        let pitch = row_heights
            .get(index as usize)
            .copied()
            .unwrap_or(LIBRARY_COVER_HEIGHT);
        if tracks == 1 {
            // Preserve the lean one-column tree on tablet/mobile: the track is
            // only an adapter for virtualization and needs no layout wrapper.
            let book = &books[track.start];
            return container(book_item(
                book,
                model.recent_thumbnails.get(&book.path),
                theme,
            ))
            .padding(Padding::new(0.0).bottom(row_gap))
            .width(Length::Fill)
            .height(Length::Fixed(pitch))
            .into();
        }

        let mut track_row = row![].spacing(track_gap).width(Length::Fill);
        for book in &books[track.start..track.start + track.len] {
            // `book_item` itself requests Fill, so the Row gives both siblings
            // equal shares without a redundant per-book Container node.
            track_row = track_row.push(book_item(
                book,
                model.recent_thumbnails.get(&book.path),
                theme,
            ));
        }
        // A singleton keeps one bounded track width without adding an empty accessible cell.
        let width = if track.len < tracks {
            Length::Fixed(track_width)
        } else {
            Length::Fill
        };
        track_row
            .padding(Padding::new(0.0).bottom(row_gap))
            .width(width)
            .height(Length::Fixed(pitch))
            .into()
    })
    // Overscan is expressed in virtual tracks. Two-track mode already realises
    // twice the books per visible band, so it uses no extra track; otherwise a
    // single overscan unit adds two more full pairs and grows the retained tree
    // beyond the one-column bound simply because the layout became wider.
    .overscan(if tracks == 2 { 0 } else { 5 })
    // The list reports its measured viewport and offset through the reducer.
    // `refresh_projection` rebuilds the provider but deliberately leaves
    // these fields untouched, so a rescan cannot reset the shelf to the top.
    .viewport_height(if model.library_viewport_height > 0.0 {
        model.library_viewport_height
    } else {
        content_height.max(240.0)
    })
    .scroll_offset(model.library_scroll_offset)
    .on_scroll(|viewport| Msg::LibraryScrolled {
        offset: viewport.absolute_offset().y,
        viewport_height: viewport.bounds().height,
    })
    .id(iced::widget::Id::new(LIBRARY_LIST_ID))
    .width(Length::Fill)
    .height(Length::Fill)
    .into_element()
}
