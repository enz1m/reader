//! Bounded EPUB loading, character-based search, reading paragraphs and footnotes.

use super::footnotes::{Footnote, FootnoteRef, extract_from_html, resolve_all};

/// The 1-based reading position of `spine_index` among the content chapters,
/// paired with the total content-chapter count. Returns `None` when there are
/// no content chapters (no open/empty book).
///
/// On front matter, the position is the number of content chapters at or before it,
/// clamped to at least 1, so it is never "Chapter 0" and never exceeds the total.
pub fn reading_position(chapters: &[EpubChapter], spine_index: usize) -> Option<(usize, usize)> {
    let content = content_chapter_indices(chapters);
    let total = content.len();
    if total == 0 {
        return None;
    }
    let position = match content.iter().position(|&i| i == spine_index) {
        Some(rank) => rank + 1,
        None => {
            // Front-matter (or a stray index): count content chapters up to and
            // including this spine position, floored at 1 and capped at total.
            content
                .iter()
                .filter(|&&i| i <= spine_index)
                .count()
                .max(1)
                .min(total)
        }
    };
    Some((position, total))
}

/// The spine index of the content chapter before `spine_index`, or `None` from the first
/// content chapter.
///
/// Shares `content_chapter_indices`' definition, so the titlebar counter and prev/next
/// agree. Front matter is reachable only from the ToC; from front matter this returns the
/// last content chapter before it.
pub fn prev_content_chapter(chapters: &[EpubChapter], spine_index: usize) -> Option<usize> {
    let content = content_chapter_indices(chapters);
    match content.iter().position(|&i| i == spine_index) {
        // On a content chapter: step to the previous one, stopping at the first.
        Some(rank) => rank.checked_sub(1).map(|r| content[r]),
        // On front-matter (or a stray index): the last content chapter strictly
        // before this position, if any.
        None => content.iter().rev().find(|&&i| i < spine_index).copied(),
    }
}

/// The spine index of the content chapter after `spine_index`, or `None` from the last
/// one. Mirror of [`prev_content_chapter`].
pub fn next_content_chapter(chapters: &[EpubChapter], spine_index: usize) -> Option<usize> {
    let content = content_chapter_indices(chapters);
    match content.iter().position(|&i| i == spine_index) {
        // On a content chapter: step to the next one, stopping at the last.
        Some(rank) => content.get(rank + 1).copied(),
        // On front-matter: the first content chapter strictly after this.
        None => content.iter().find(|&&i| i > spine_index).copied(),
    }
}

mod archive;
mod chapters;
mod cover;
mod html;
mod loading;
mod paragraphs;
mod search;
mod text;
mod toc;

pub use chapters::{EpubChapter, content_chapter_indices};
pub use cover::{
    CoverThumbnail, cover_thumbnail, decode_cover_bytes, strip_redundant_leading_heading,
};
pub use html::{html_to_plain_text, strip_footnote_bodies};
pub use loading::load_epub;
pub use paragraphs::{
    InlineSpan, InlineSpanKind, ReadingParagraph, build_chapter_reading_paragraphs,
};
pub use search::{EpubHit, MAX_EPUB_SEARCH_HITS, find_hits, find_hits_in};
pub use text::{
    EpubHighlightRange, EpubLoadResult, EpubMetadata, EpubTocEntry, LocatedFootnoteRef,
    locate_footnote_refs,
};

#[cfg(test)]
mod tests;
