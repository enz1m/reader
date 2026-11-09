//! Character-anchored EPUB pagination.
//!
//! Shaper byte offsets are converted to reader character offsets. Reflow preserves
//! the selected passage while typography or viewport dimensions change.

use crate::plumbing::epub_support::{InlineSpan, ReadingParagraph};

/// Fallback chrome allowance in logical pixels before page-box measurement arrives.
pub const READING_CHROME: f32 = 63.0;

/// Lines reserved on a chapter's first page for the heading drawn only there. The
/// `Title2` heading and its margin take one body-line slot.
pub const HEADING_RESERVE_LINES: f32 = 1.0;

/// The window the shell opens at before the first size event arrives
/// (`iced::window::Settings::default()`'s 1024x768).
///
/// The first painted frame of a restored book is already paginated, so assuming the
/// opening size avoids a wrong first cut.
pub const DEFAULT_WINDOW: (f32, f32) = (1024.0, 768.0);

/// The smallest page box paginated into; smaller boxes clamp here rather than producing
/// one-line pages.
pub const MIN_PAGE_HEIGHT: f32 = 120.0;

/// A logical reading position: a chapter plus a character offset within that
/// chapter's plain text.
///
/// Stable across reflow, unlike a page index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadingAnchor {
    pub chapter: usize,
    pub char_offset: usize,
}

impl ReadingAnchor {
    pub fn new(chapter: usize, char_offset: usize) -> Self {
        Self {
            chapter,
            char_offset,
        }
    }
}

/// The geometry a page is cut to: the readable column width, the page box
/// height, and the two reading metrics the user controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageMetrics {
    column_width: f32,
    page_height: f32,
    font_size: f32,
    line_spacing: f32,
    /// The chapter heading's height with its bottom margin, in pixels, or
    /// `None` to reserve [`HEADING_RESERVE_LINES`].
    heading_height: Option<f32>,
}

impl PageMetrics {
    /// Build the metrics, sanitising every input.
    ///
    /// A non-finite or absurd value falls back to a usable default; a `NaN` height would
    /// otherwise produce zero pages.
    pub fn new(column_width: f32, page_height: f32, font_size: f32, line_spacing: f32) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        Self {
            column_width: finite(column_width, 320.0).clamp(120.0, 4096.0),
            page_height: finite(page_height, MIN_PAGE_HEIGHT).clamp(MIN_PAGE_HEIGHT, 8192.0),
            font_size: finite(font_size, 18.0).clamp(6.0, 96.0),
            line_spacing: finite(line_spacing, 1.5).clamp(0.8, 4.0),
            heading_height: None,
        }
    }

    /// The first page's heading costs `height` pixels (its measured text plus
    /// bottom margin) and one paragraph gap, rather than a fixed line.
    pub fn with_heading_height(mut self, height: f32) -> Self {
        self.heading_height = (height.is_finite() && height >= 0.0).then_some(height);
        self
    }

    // Body lines the first page's heading takes, fractional.
    fn heading_lines(&self) -> f32 {
        match self.heading_height {
            Some(height) => height / self.line_height() + self.paragraph_gap_lines(),
            None => HEADING_RESERVE_LINES,
        }
    }

    /// The metrics for a reading surface in a `window_height`-tall window,
    /// after the shell chrome ([`READING_CHROME`]) is taken out.
    pub fn for_window(
        column_width: f32,
        window_height: f32,
        font_size: f32,
        line_spacing: f32,
    ) -> Self {
        let page_height = if window_height.is_finite() {
            window_height - READING_CHROME
        } else {
            MIN_PAGE_HEIGHT
        };
        Self::new(column_width, page_height, font_size, line_spacing)
    }

    pub fn column_width(&self) -> f32 {
        self.column_width
    }

    pub fn page_height(&self) -> f32 {
        self.page_height
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    pub fn line_spacing(&self) -> f32 {
        self.line_spacing
    }

    /// One line of body text, in logical pixels.
    pub fn line_height(&self) -> f32 {
        self.font_size * self.line_spacing
    }

    /// How many whole body lines fit in the page box, at least one.
    pub fn lines_per_page(&self) -> usize {
        ((self.page_height / self.line_height()).floor() as usize).max(1)
    }

    // Typography shared by exact measurement and exact page breaking.
    fn typography(&self) -> enzui_iced_widgets::text_measure::MeasureTypography {
        use enzui_iced_widgets::enz_text::TextSize;
        use iced::advanced::text::Shaping;

        enzui_iced_widgets::text_measure::MeasureTypography::new(TextSize::Body)
            .pin_size_px(self.font_size)
            // Line spacing is an independent user setting, so pin the absolute height.
            .pin_line_height_px(self.line_height())
            .shaping(Shaping::Advanced)
    }

    /// Exact visual-line count from the renderer's paragraph shaper.
    pub fn lines_for_text(&self, text: &str) -> usize {
        enzui_iced_widgets::text_measure::enz_text_measure_with(
            text,
            self.column_width,
            self.typography(),
        )
        .line_count
    }

    // Character offset where the first visual line after `max_lines` starts.
    fn break_char_offset(&self, text: &str, max_lines: usize) -> usize {
        let byte_offset = enzui_iced_widgets::text_measure::enz_text_break_offset_with(
            text,
            self.column_width,
            max_lines,
            self.typography(),
        );
        shaper_byte_offset_to_char_offset(text, byte_offset)
    }

    /// The blank space between two paragraphs, expressed in body lines.
    ///
    /// `crate::reading_pane::chapter_view` spaces paragraphs by `font_size * 0.7`, so
    /// the gap in line units shrinks as the line height grows.
    pub fn paragraph_gap_lines(&self) -> f32 {
        (self.font_size * 0.7) / self.line_height()
    }
}

// Convert the framework shaper's UTF-8 byte offset into reader-app's
// chapter character-offset space.
//
// Treating a byte offset as a char offset can land inside a multi-byte character.
// `enz_text_break_offset_with` guarantees a character boundary, and slicing first makes
// a violation loud.
fn shaper_byte_offset_to_char_offset(text: &str, byte_offset: usize) -> usize {
    assert!(
        byte_offset <= text.len() && text.is_char_boundary(byte_offset),
        "shaper break {byte_offset} must be a UTF-8 character boundary"
    );
    text[..byte_offset].chars().count()
}

/// One page: a half-open character range in the chapter's plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSpan {
    pub char_start: usize,
    pub char_end: usize,
}

impl PageSpan {
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.char_start && offset < self.char_end
    }
}

/// Cut a chapter's paragraphs into viewport-sized pages.
///
/// Returns one [`PageSpan`] per page in reading order, tiling the chapter with no gaps or
/// overlaps (`tests::pages_tile_the_chapter_exactly`). An empty chapter yields no pages.
///
/// Paragraphs stay whole when they fit; a paragraph longer than a page splits at a word
/// boundary, so no page begins mid-word.
#[must_use]
pub fn paginate(paragraphs: &[ReadingParagraph], metrics: &PageMetrics) -> Vec<PageSpan> {
    let Some((start, end)) = content_bounds(paragraphs) else {
        return Vec::new();
    };
    paginate_range(paragraphs, metrics, start, end, true)
}

/// [`paginate`], but with a page boundary forced at `anchor`, so after reflow the
/// reader's passage starts a page. Earlier pages are cut forward from the chapter start,
/// leaving the page before the anchor short. An anchor outside the content bounds
/// degenerates to [`paginate`].
#[must_use]
pub fn paginate_anchored(
    paragraphs: &[ReadingParagraph],
    metrics: &PageMetrics,
    anchor: usize,
) -> Vec<PageSpan> {
    let Some((start, end)) = content_bounds(paragraphs) else {
        return Vec::new();
    };
    if anchor <= start || anchor >= end {
        return paginate_range(paragraphs, metrics, start, end, true);
    }
    let mut pages = paginate_range(paragraphs, metrics, start, anchor, true);
    pages.extend(paginate_range(paragraphs, metrics, anchor, end, false));
    pages
}

// Cut the content in `[range_start, range_end)` into pages.
//
// `with_heading` reserves [`HEADING_RESERVE_LINES`] on the first page. Each paragraph's
// visual-line count and overflow offset come from the renderer's shaper; there is no
// width estimate or second wrap implementation.
fn paginate_range(
    paragraphs: &[ReadingParagraph],
    metrics: &PageMetrics,
    range_start: usize,
    range_end: usize,
    with_heading: bool,
) -> Vec<PageSpan> {
    if range_end <= range_start {
        return Vec::new();
    }

    let lines_per_page = metrics.lines_per_page();
    let gap = metrics.paragraph_gap_lines();

    let mut pages: Vec<PageSpan> = Vec::new();
    let mut page_start = range_start;
    // Fractional, because paragraph gaps are not whole lines.
    let mut used: f32 = if with_heading {
        metrics
            .heading_lines()
            .min(lines_per_page.saturating_sub(1) as f32)
    } else {
        0.0
    };
    let mut first_on_page = true;

    let close_page = |end: usize, page_start: &mut usize, pages: &mut Vec<PageSpan>| {
        if end > *page_start {
            pages.push(PageSpan {
                char_start: *page_start,
                char_end: end,
            });
            *page_start = end;
        }
    };

    for para in paragraphs {
        if para.char_end <= range_start || para.char_start >= range_end {
            continue;
        }
        let para_end = para.char_end.min(range_end);
        // Byte ranges for slicing this paragraph only; shaper offsets convert through
        // `shaper_byte_offset_to_char_offset`.
        let char_to_byte: Vec<usize> = para
            .text
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(para.text.len()))
            .collect();
        let mut cursor = para.char_start.max(page_start).max(range_start);
        let mut needs_gap = !first_on_page;
        while cursor < para_end {
            if needs_gap {
                used += gap;
                needs_gap = false;
            }
            let budget_lines = (lines_per_page as f32 - used).floor().max(0.0) as usize;
            if budget_lines == 0 {
                close_page(cursor, &mut page_start, &mut pages);
                used = 0.0;
                first_on_page = true;
                continue;
            }
            let lo = (cursor - para.char_start).min(char_to_byte.len() - 1);
            let hi = (para_end - para.char_start).min(char_to_byte.len() - 1);
            let text = &para.text[char_to_byte[lo]..char_to_byte[hi]];
            let lines = metrics.lines_for_text(text);
            if lines <= budget_lines {
                used += lines as f32;
                cursor = para_end;
                first_on_page = false;
                break;
            }

            // Overflow: the shaper returns where the first line not on this page begins.
            let consumed_chars = metrics.break_char_offset(text, budget_lines);
            let cut = (cursor + consumed_chars).clamp(cursor + 1, para_end);
            close_page(cut, &mut page_start, &mut pages);
            cursor = cut;
            used = 0.0;
            first_on_page = true;
        }
    }
    close_page(range_end, &mut page_start, &mut pages);
    pages
}

// The chapter's overall character span, as the paragraphs report it.
fn content_bounds(paragraphs: &[ReadingParagraph]) -> Option<(usize, usize)> {
    let start = paragraphs.iter().map(|p| p.char_start).min()?;
    let end = paragraphs.iter().map(|p| p.char_end).max()?;
    if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// The index of the page containing `offset`.
///
/// Offsets clamp to the first or last page, so a stale anchor lands on a real page.
/// Returns 0 for an empty pagination.
#[must_use]
pub fn page_for_offset(pages: &[PageSpan], offset: usize) -> usize {
    if pages.is_empty() {
        return 0;
    }
    for (i, p) in pages.iter().enumerate() {
        if p.contains(offset) {
            return i;
        }
    }
    if offset < pages[0].char_start {
        0
    } else {
        pages.len() - 1
    }
}

/// The offset one page forward of `offset`, clamped at the end of the chapter.
///
/// The result is the start of the next page, so following it with
/// [`previous_page_offset`] returns the original page start.
#[must_use]
pub fn next_page_offset(pages: &[PageSpan], offset: usize) -> usize {
    if pages.is_empty() {
        return offset;
    }
    let i = page_for_offset(pages, offset);
    match pages.get(i + 1) {
        Some(next) => next.char_start,
        None => offset,
    }
}

/// The offset one page back of `offset`, clamped at the start of the chapter.
#[must_use]
pub fn previous_page_offset(pages: &[PageSpan], offset: usize) -> usize {
    if pages.is_empty() {
        return offset;
    }
    let i = page_for_offset(pages, offset);
    if i == 0 {
        return pages[0].char_start.min(offset);
    }
    pages[i - 1].char_start
}

/// The paragraphs (or paragraph fragments) that lie inside `span`.
///
/// The paginated view paints only this, so content beyond the page is absent from the
/// widget tree. Inline spans are re-based and clipped to the fragment, so a highlight
/// straddling a break paints on both pages.
#[must_use]
pub fn slice_paragraphs(paragraphs: &[ReadingParagraph], span: &PageSpan) -> Vec<ReadingParagraph> {
    let mut out = Vec::new();
    for para in paragraphs {
        let lo = para.char_start.max(span.char_start);
        let hi = para.char_end.min(span.char_end);
        if hi <= lo {
            continue;
        }
        let mut local_lo = lo - para.char_start;
        let local_hi = hi - para.char_start;
        let raw: String = para
            .text
            .chars()
            .skip(local_lo)
            .take(local_hi - local_lo)
            .collect();
        if raw.trim().is_empty() {
            continue;
        }
        // A continuing fragment starts with the separating space, which would indent the
        // first line; drop it and advance `char_start` with it.
        let leading = raw.chars().take_while(|c| c.is_whitespace()).count();
        let text: String = raw.chars().skip(leading).collect();
        local_lo += leading;
        let lo = lo + leading;
        let spans: Vec<InlineSpan> = para
            .spans
            .iter()
            .filter_map(|s| {
                let start = s.char_start.max(local_lo);
                let end = s.char_end.min(local_hi).max(start);
                if end <= start {
                    return None;
                }
                Some(InlineSpan {
                    char_start: start.saturating_sub(local_lo),
                    char_end: end.saturating_sub(local_lo),
                    kind: s.kind.clone(),
                })
            })
            .collect();
        out.push(ReadingParagraph {
            chapter: para.chapter,
            para_index: para.para_index,
            text,
            spans,
            char_start: lo,
            char_end: hi,
        });
    }
    out
}

#[cfg(test)]
mod tests;
