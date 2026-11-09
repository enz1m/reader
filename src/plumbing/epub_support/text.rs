//! EPUB text blocks, footnotes, and load types.

use super::*;

/// Represents an EPUB table-of-contents entry.
#[derive(Clone, Debug)]
pub struct EpubTocEntry {
    pub title: String,
    pub chapter_index: usize,
    pub level: usize,
}

/// Parsed EPUB metadata.
#[derive(Clone, Debug, Default)]
pub struct EpubMetadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub description: String,
    pub publisher: String,
}

/// Result of loading an EPUB file.
#[derive(Clone, Debug)]
pub struct EpubLoadResult {
    pub metadata: EpubMetadata,
    pub chapters: Vec<EpubChapter>,
    pub toc: Vec<EpubTocEntry>,
    /// All footnotes / endnotes found across the book, deduplicated per
    /// `(chapter, id)` and with labels resolved.  Empty when the book uses
    /// no notes.
    pub footnotes: Vec<Footnote>,
    /// Footnote reference sites per chapter — the in-text anchors that
    /// point at notes.  Used by the UI to render clickable inline markers
    /// in the chapter body.
    pub footnote_refs: Vec<FootnoteRef>,
}

impl EpubLoadResult {
    /// Resolve an inline reference from `source_chapter` to its parsed note body.
    ///
    /// Same-chapter identifiers are scoped to that chapter. Cross-chapter
    /// references additionally match their href, so repeated note ids in
    /// different XHTML resources cannot select the wrong body. Ambiguous,
    /// missing, and empty targets return `None`.
    pub fn resolve_footnote(&self, source_chapter: usize, target_id: &str) -> Option<&Footnote> {
        let mut matching_refs = self.footnote_refs.iter().filter(|reference| {
            reference.chapter == source_chapter && reference.target_id == target_id
        });
        let reference = matching_refs.next()?;
        if matching_refs.any(|candidate| candidate.target_href != reference.target_href) {
            return None;
        }

        let note = match reference.target_href.as_deref() {
            None => self
                .footnotes
                .iter()
                .find(|note| note.chapter == source_chapter && note.id == target_id),
            Some(href) => self
                .footnotes
                .iter()
                .find(|note| note.id == target_id && note.source_href.as_deref() == Some(href)),
        }?;
        (!note.text.trim().is_empty()).then_some(note)
    }
}

/// A highlight range for projection onto chapter text.
#[derive(Clone, Debug)]
pub struct EpubHighlightRange {
    pub chapter: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub color: String,
    pub style: String,
}

/// A footnote-ref marker located in the chapter's plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedFootnoteRef {
    pub chapter: usize,
    /// Char offset in the chapter's `plain_text` where the label was found.
    pub char_pos: usize,
    /// Length in chars of the label in the plain text.
    pub char_len: usize,
    /// Target footnote id (e.g. "fn1").
    pub target_id: String,
    /// Display label ("1", "*", "†").
    pub label: String,
}

/// Locate footnote reference labels in a chapter's plain text.
///
/// For each `FootnoteRef` of `chapter_index`, scan forward through `plain_text` for its
/// label and accept the first unconsumed match.
///
/// Returns the located refs sorted by `char_pos` ascending.
pub fn locate_footnote_refs(
    plain_text: &str,
    chapter_index: usize,
    refs: &[FootnoteRef],
) -> Vec<LocatedFootnoteRef> {
    let chapter_refs: Vec<&FootnoteRef> =
        refs.iter().filter(|r| r.chapter == chapter_index).collect();
    if chapter_refs.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = plain_text.chars().collect();
    let mut used_positions: Vec<bool> = vec![false; chars.len()];
    let mut results = Vec::new();

    for fref in &chapter_refs {
        let label = fref.label.trim();
        if label.is_empty() {
            continue;
        }
        let label_chars: Vec<char> = label.chars().collect();
        let label_len = label_chars.len();
        if label_len > chars.len() {
            continue;
        }
        // Short numeric labels prefer word-attached positions ("reference1") over
        // standalone ones ("Chapter 1"), avoiding heading and prose digits.
        let label_lower = label.to_lowercase();
        let label_lower_chars: Vec<char> = label_lower.chars().collect();
        let is_short_numeric = label_len <= 3 && label.chars().all(|c| c.is_ascii_digit());

        // Collect ALL candidate positions.
        let mut candidates: Vec<usize> = Vec::new();
        let mut search_from = 0usize;
        while search_from + label_len <= chars.len() {
            let candidate: String = chars[search_from..search_from + label_len]
                .iter()
                .collect::<String>()
                .to_lowercase();
            let candidate_chars: Vec<char> = candidate.chars().collect();
            if candidate_chars == label_lower_chars && !used_positions[search_from] {
                // For short numeric labels, skip if the digit is part of
                // a larger number (preceded or followed by another digit).
                if is_short_numeric {
                    let prev_digit = search_from > 0 && chars[search_from - 1].is_ascii_digit();
                    let next_digit = search_from + label_len < chars.len()
                        && chars[search_from + label_len].is_ascii_digit();
                    if prev_digit || next_digit {
                        search_from += 1;
                        continue;
                    }
                }
                candidates.push(search_from);
            }
            search_from += 1;
        }

        // For short numeric labels, prefer word-attached positions
        // (preceded by an alphanumeric letter) over standalone ones.
        let best = if is_short_numeric && candidates.len() > 1 {
            candidates
                .iter()
                .copied()
                .find(|&pos| pos > 0 && chars[pos - 1].is_alphabetic())
                .or_else(|| candidates.first().copied())
        } else {
            candidates.first().copied()
        };

        if let Some(pos) = best {
            used_positions[pos..pos + label_len].fill(true);
            results.push(LocatedFootnoteRef {
                chapter: chapter_index,
                char_pos: pos,
                char_len: label_len,
                target_id: fref.target_id.clone(),
                label: label.to_string(),
            });
        }
    }

    results.sort_by_key(|r| r.char_pos);
    results
}
