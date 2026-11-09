//! Reading paragraphs and inline spans.

use super::*;

/// The visual kind of an inline decoration span in reading text.
///
/// The reading pane turns each into an `iced::widget::text::Span` with a background
/// highlight, an underline, or a clickable link payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineSpanKind {
    /// A highlight: coloured background over the range. `color` is a palette
    /// key ("yellow", "green", "blue", "pink", "red").
    Highlight { color: String },
    /// A clickable footnote / endnote reference. `target_id` is the note id
    /// the reading pane fires on click; the range is drawn underlined.
    FootnoteRef { target_id: String },
}

/// One inline decoration over a char range within a paragraph's `text`, relative to the
/// paragraph; the reading pane converts to byte offsets when slicing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSpan {
    pub char_start: usize,
    pub char_end: usize,
    pub kind: InlineSpanKind,
}

/// Full paragraph text with character-relative decorations, preserving native word wrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingParagraph {
    pub chapter: usize,
    pub para_index: usize,
    pub text: String,
    /// Typed inline decorations at char ranges within `text`.
    pub spans: Vec<InlineSpan>,
    pub char_start: usize,
    pub char_end: usize,
}

/// Build reading paragraphs for a single chapter with typed inline spans.
///
/// Each paragraph of the chapter's plain text becomes one [`ReadingParagraph`]
/// carrying typed [`InlineSpan`]s at char ranges within `text`:
/// - [`InlineSpanKind::Highlight`] — background tint at a precise char range,
/// - [`InlineSpanKind::FootnoteRef`] — underline with a clickable target id.
///
pub fn build_chapter_reading_paragraphs(
    chapter: &EpubChapter,
    highlights: &[EpubHighlightRange],
    located_refs: &[LocatedFootnoteRef],
) -> Vec<ReadingParagraph> {
    let plain_text = &chapter.plain_text;
    let total_chars = plain_text.chars().count();
    if total_chars == 0 {
        return Vec::new();
    }

    let ch_highlights: Vec<&EpubHighlightRange> = highlights
        .iter()
        .filter(|h| h.chapter == chapter.index && h.char_start < h.char_end)
        .collect();
    let ch_refs: Vec<&LocatedFootnoteRef> = located_refs
        .iter()
        .filter(|r| r.chapter == chapter.index)
        .collect();

    // Split into paragraphs at newlines.
    let chars_vec: Vec<char> = plain_text.chars().collect();
    let mut paragraphs = Vec::new();
    let mut para_start = 0usize;
    let mut para_counter = 0usize;

    // Collect paragraph boundaries [start, end) in char offsets.
    let mut para_ranges: Vec<(usize, usize, usize)> = Vec::new();
    for (i, &c) in chars_vec.iter().enumerate() {
        if c == '\n' {
            if para_start < i {
                para_ranges.push((para_start, i, para_counter));
            }
            para_start = i + 1;
            para_counter += 1;
        }
    }
    // Final paragraph.
    if para_start < total_chars {
        para_ranges.push((para_start, total_chars, para_counter));
    }

    // Fast path: no markers at all → one paragraph per newline-delimited block.
    if ch_highlights.is_empty() && ch_refs.is_empty() {
        for &(pstart, pend, pidx) in &para_ranges {
            let text: String = chars_vec[pstart..pend].iter().collect();
            paragraphs.push(ReadingParagraph {
                chapter: chapter.index,
                para_index: pidx,
                text,
                spans: Vec::new(),
                char_start: pstart,
                char_end: pend,
            });
        }
        return paragraphs;
    }

    // Build spans from overlapping highlights and footnote refs, with offsets relative to
    // the paragraph.
    for &(pstart, pend, pidx) in &para_ranges {
        let text: String = chars_vec[pstart..pend].iter().collect();
        let para_char_count = pend - pstart;
        let mut spans: Vec<InlineSpan> = Vec::new();

        // Highlight spans.
        for h in &ch_highlights {
            let hs = h.char_start.min(total_chars);
            let he = h.char_end.min(total_chars);
            if hs < pend && he > pstart {
                let span_start = hs.max(pstart) - pstart;
                let span_end = he.min(pend) - pstart;
                if span_start < span_end && span_end <= para_char_count {
                    spans.push(InlineSpan {
                        char_start: span_start,
                        char_end: span_end,
                        kind: InlineSpanKind::Highlight {
                            color: h.color.clone(),
                        },
                    });
                }
            }
        }

        // Footnote/link ref spans.
        for r in &ch_refs {
            let rpos = r.char_pos;
            let rend = r.char_pos + r.char_len;
            if rpos >= pstart && rpos < pend {
                let span_start = rpos - pstart;
                let span_end = rend.min(pend) - pstart;
                if span_start < span_end && span_end <= para_char_count {
                    spans.push(InlineSpan {
                        char_start: span_start,
                        char_end: span_end,
                        kind: InlineSpanKind::FootnoteRef {
                            target_id: r.target_id.clone(),
                        },
                    });
                }
            }
        }

        // Keep spans ordered by start so the reading pane can slice in one pass.
        spans.sort_by_key(|s| (s.char_start, s.char_end));

        paragraphs.push(ReadingParagraph {
            chapter: chapter.index,
            para_index: pidx,
            text,
            spans,
            char_start: pstart,
            char_end: pend,
        });
    }

    paragraphs
}
