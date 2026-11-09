//! EPUB full-text search and CFI verification.

use super::*;

/// Upper bound on the matches one full-text scan collects.
///
/// A one-character query over a long chapter would otherwise grow hits, `search_hits`
/// and the rendered list with the document. Matches past the cap are dropped and the
/// result is reported as truncated. Mirrors [`crate::search::MAX_PDF_SEARCH_HITS`].
pub const MAX_EPUB_SEARCH_HITS: usize = 2_048;

/// A single match returned by `find_hits`: chapter + `[start, end)` in the
/// chapter's `plain_text`, plus a preview snippet suitable for the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpubHit {
    pub chapter: usize,
    pub char_start: usize,
    pub char_end: usize,
    /// A ~40-char context window centred on the match, for UI previews.
    pub preview: String,
}

/// Case-insensitive substring search across every chapter's plain text.
/// Returns matches in reading order (chapter, then character offset), capped at
/// [`MAX_EPUB_SEARCH_HITS`].
///
/// Offsets count chars in `plain_text`; a `{chapter, start, end}` triple is a stable
/// enough anchor for text that has not changed.
pub fn find_hits(chapters: &[EpubChapter], query: &str) -> Vec<EpubHit> {
    find_hits_in(
        chapters.iter().map(|c| (c.index, c.plain_text.as_str())),
        query,
    )
}

/// The engine behind [`find_hits`], over `(chapter_index, plain_text)` pairs.
///
/// Lets [`crate::search::SearchIndex`] scan an owned `Send` snapshot without cloning the
/// chapters on every keystroke.
pub fn find_hits_in<'a>(
    chapters: impl Iterator<Item = (usize, &'a str)>,
    query: &str,
) -> Vec<EpubHit> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    // Lowercased needle as a char sequence. Case-insensitive matching compares
    // lowercased sequences, but the reported offsets index the ORIGINAL text.
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    for (chapter_index, plain_text) in chapters {
        // Original chars, so `char_start`/`char_end` are offsets into the
        // untouched `plain_text` regardless of any length-changing lowercase
        // (e.g. İ → "i̇") that would drift offsets computed on a lowered copy.
        let hay: Vec<char> = plain_text.chars().collect();
        let mut i = 0usize;
        while i < hay.len() {
            if let Some(consumed) = char_window_matches_lower(&hay[i..], &needle) {
                let char_start = i;
                // `consumed` is the number of ORIGINAL chars the match spans,
                // which can differ from `needle.len()` if a matched char
                // expanded on lowercasing.
                let char_end = i + consumed;
                let preview = preview_slice(plain_text, char_start, char_end, 20);
                out.push(EpubHit {
                    chapter: chapter_index,
                    char_start,
                    char_end,
                    preview,
                });
                if out.len() >= MAX_EPUB_SEARCH_HITS {
                    return out;
                }
            }
            // Advance one original char (overlapping matches are intentional,
            // matching the previous behaviour).
            i += 1;
        }
    }
    out
}

// If the lowercase of `hay`'s leading chars equals `needle_lower`, returns the number of
// original `hay` chars spanned. A char whose lowercase expands (İ → two chars) counts as
// one original char; a needle ending mid-expansion is rejected.
fn char_window_matches_lower(hay: &[char], needle_lower: &[char]) -> Option<usize> {
    let mut ni = 0usize; // index into needle_lower
    let mut consumed = 0usize; // original hay chars consumed
    for &hc in hay {
        if ni >= needle_lower.len() {
            break;
        }
        for lc in hc.to_lowercase() {
            if ni >= needle_lower.len() || lc != needle_lower[ni] {
                return None;
            }
            ni += 1;
        }
        consumed += 1;
    }
    if ni == needle_lower.len() {
        Some(consumed)
    } else {
        None
    }
}

pub(crate) fn preview_slice(text: &str, start: usize, end: usize, ctx_chars: usize) -> String {
    let total = text.chars().count();
    let before_start = start.saturating_sub(ctx_chars);
    let after_end = (end + ctx_chars).min(total);
    let mut out = String::new();
    if before_start > 0 {
        out.push('…');
    }
    out.extend(
        text.chars()
            .skip(before_start)
            .take(after_end - before_start),
    );
    if after_end < total {
        out.push('…');
    }
    out
}
