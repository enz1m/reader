//! Flowing chapter text with character-precise highlights, selection and footnote links.

use enzui_iced_theme::ContrastFloor;
use enzui_iced_widgets::rich_text::enz_prose_semantics;
use enzui_iced_widgets::selection_scope::{ScopeSelection, enz_selection_scope};
use enzui_iced_widgets::tokens::annotation_glyph_ink;
use iced::widget::text::{LineHeight, Span};
use iced::widget::{column, container, rich_text, span};
use iced::{Color, Element, Font, Length, Padding};

use crate::model::{ReadingThemeColors, rgb};
use crate::plumbing::epub_support::{InlineSpanKind, ReadingParagraph};

// Persisted highlight colors stay stable across reading themes.
fn highlight_rgb(name: &str) -> Color {
    let (r, g, b) = match name {
        "yellow" => (0xff, 0xe0, 0x66),
        "green" => (0x9b, 0xe8, 0x8a),
        "blue" => (0x8a, 0xc8, 0xff),
        "pink" => (0xff, 0x9b, 0xc8),
        "red" => (0xff, 0x8a, 0x8a),
        "purple" => (0xc8, 0x9b, 0xff),
        _ => (0xff, 0xe0, 0x66),
    };
    Color::from_rgba8(r, g, b, 0.55)
}

/// The recommended readable column width for body text (characters ~ 66-75).
///
/// Reads `SizingTokens::container_default` (720 px), the prose-width tier, so density
/// retunes reach the reading column.
pub fn readable_column_width() -> f32 {
    enzui_iced_theme::SizingTokens::default().container_default
}

// Build the iced `Span`s for one paragraph, slicing its text at the char
// ranges of its typed inline spans.
//
// `Link` is the click payload fired by `Rich::on_link_click`; only footnote refs carry
// one. A range starting inside an already-emitted one is skipped; the plumbing sorts
// and de-overlaps ranges.
fn paragraph_spans<'a, Link: Clone>(
    para: &'a ReadingParagraph,
    colors: &ReadingThemeColors,
    make_link: &impl Fn(&str) -> Link,
) -> Vec<Span<'a, Link>> {
    let fg = rgb(colors.fg);
    let paper = rgb(colors.bg);
    let link_color = rgb(colors.link);

    let text = para.text.as_str();
    let char_to_byte: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .collect();
    let n_chars = char_to_byte.len().saturating_sub(1);

    let mut spans: Vec<Span<'a, Link>> = Vec::new();
    let mut cursor = 0usize;

    for s in &para.spans {
        let start = s.char_start.min(n_chars);
        let end = s.char_end.min(n_chars);
        if end <= start || start < cursor {
            // Out of range or overlapping an already-emitted span: skip.
            continue;
        }
        // Gap before the decorated range → plain span.
        if start > cursor {
            let slice = &text[char_to_byte[cursor]..char_to_byte[start]];
            spans.push(span(slice).color(fg));
        }
        let slice = &text[char_to_byte[start]..char_to_byte[end]];
        match &s.kind {
            InlineSpanKind::Highlight { color } => {
                let tint = highlight_rgb(color);
                let ink = annotation_glyph_ink(fg, tint, paper, ContrastFloor::Text);
                spans.push(
                    span(slice)
                        .color(ink)
                        .background(tint)
                        .padding(Padding::from([0.0, 1.0])),
                );
            }
            InlineSpanKind::FootnoteRef { target_id } => {
                spans.push(
                    span(slice)
                        .color(link_color)
                        .underline(true)
                        .link(make_link(target_id)),
                );
            }
        }
        cursor = end;
    }
    // Trailing plain text after the last decorated range.
    if cursor < n_chars {
        let slice = &text[char_to_byte[cursor]..];
        spans.push(span(slice).color(fg));
    }
    // A paragraph with no decorations at all still needs its text.
    if spans.is_empty() {
        spans.push(span(text).color(fg));
    }
    spans
}

/// Render a single paragraph as a rich-text element.
pub fn paragraph_view<'a, Message, Link>(
    para: &'a ReadingParagraph,
    colors: &ReadingThemeColors,
    font_size: f32,
    line_spacing: f32,
    make_link: impl Fn(&str) -> Link,
    on_link_click: impl Fn(Link) -> Message + 'a,
) -> Element<'a, Message>
where
    Message: 'a,
    Link: Clone + 'static,
{
    let spans = paragraph_spans(para, colors, &make_link);
    // Raw `rich_text` rather than a typography token: `.size(font_size)` is the reader's
    // persisted font-size control.
    let el: Element<'a, Message> = rich_text(spans)
        .size(font_size)
        .line_height(LineHeight::Relative(line_spacing))
        .font(Font::default())
        .on_link_click(on_link_click)
        .width(Length::Fill)
        .into();
    // `enz_prose_semantics` exposes this paragraph's text to the selection scope.
    enz_prose_semantics(el, para.text.clone())
}

/// The readable column width for a given viewport width: the ideal reading
/// measure, but never wider than the viewport (minus gutters) so it never
/// overflows at tablet/mobile widths.
pub fn column_width_for(viewport_width: f32) -> f32 {
    let usable = (viewport_width - 48.0).max(240.0);
    usable.min(readable_column_width())
}

/// Render a whole chapter as a centred, readable-width column of paragraphs.
///
/// `column_width` is clamped to the viewport. `make_link` converts a footnote target id
/// into the payload that `on_link_click` turns into a `Message`. `heading` is the first
/// child of the same column, so it shares the body's left edge.
#[allow(clippy::too_many_arguments)]
pub fn chapter_view<'a, Message, Link>(
    heading: Option<Element<'a, Message>>,
    paragraphs: &'a [ReadingParagraph],
    colors: &ReadingThemeColors,
    font_size: f32,
    line_spacing: f32,
    column_width: f32,
    make_link: impl Fn(&str) -> Link + Copy + 'a,
    on_link_click: impl Fn(Link) -> Message + Copy + 'a,
    on_copy: impl Fn(String) -> Message + 'a,
    on_selection_change: impl Fn(Option<ScopeSelection>) -> Message + 'a,
    // Required so the pane cannot be built without forwarding
    // `Model::selection_clear_generation`.
    selection_clear_generation: u64,
    // Only these paragraphs are built, with spacers for the rest; `None`
    // builds the whole chapter.
    window: Option<&ParagraphWindow>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
    Link: Clone + 'static,
{
    // Paragraph spacing scales with font size for a consistent reading rhythm.
    let para_gap = font_size * PARAGRAPH_GAP;
    let realised = match window {
        Some(window) => {
            &paragraphs
                [window.range.start.min(paragraphs.len())..window.range.end.min(paragraphs.len())]
        }
        None => paragraphs,
    };
    let mut prose = column![].spacing(para_gap).width(Length::Fill);
    for para in realised {
        prose = prose.push(paragraph_view(
            para,
            colors,
            font_size,
            line_spacing,
            make_link,
            on_link_click,
        ));
    }

    // The heading stays outside the selection scope, or it would become paragraph 0 and
    // shift every persisted CFI.
    let selected_prose = enz_selection_scope(prose)
        .on_copy(on_copy)
        .on_selection_change(on_selection_change)
        .clear_generation(selection_clear_generation)
        .into_element();
    let mut col = column![].spacing(para_gap).width(Length::Fill);
    if let Some(heading) = heading {
        col = col.push(heading);
    }
    // The spacers sit outside the selection document, so its paragraph
    // indices count from the window's first paragraph.
    if let Some(window) = window.filter(|window| window.above > 0.0) {
        col = col.push(iced::widget::Space::new().height(window.above));
    }
    col = col.push(selected_prose);
    if let Some(window) = window.filter(|window| window.below > 0.0) {
        col = col.push(iced::widget::Space::new().height(window.below));
    }

    // Centre the readable column in the available width.
    container(container(col).width(Length::Fixed(column_width)))
        .center_x(Length::Fill)
        .padding(Padding::from([COLUMN_PADDING, COLUMN_PADDING]))
        .into()
}

/// The readable column's own padding on every side.
pub const COLUMN_PADDING: f32 = 24.0;

/// A selection made in a windowed chapter, with its paragraph indices
/// counted from the chapter's start rather than the window's.
pub fn chapter_selection(
    mut selection: enzui_iced_widgets::selection_scope::ScopeSelection,
    window_start: usize,
) -> enzui_iced_widgets::selection_scope::ScopeSelection {
    selection.start.paragraph += window_start;
    selection.end.paragraph += window_start;
    selection
}

/// A chapter with at least this many paragraphs is realised as a window
/// around the viewport in scroll mode; a shorter one is built whole.
pub const WINDOW_MIN_PARAGRAPHS: usize = 300;

/// Cached chapter geometry for viewport realization; omitted paragraphs retain their space
/// without per-scroll shaping.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterGeometry {
    key: GeometryKey,
    /// Each paragraph's height.
    heights: Vec<f32>,
    /// Where each paragraph starts, counting the gaps before it.
    tops: Vec<f32>,
    gap: f32,
}

/// What a [`ChapterGeometry`] was measured for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryKey {
    /// The open document's load attempt, so another book's chapter with the
    /// same index and length never reuses this one's geometry.
    pub document: u64,
    pub chapter: usize,
    pub paragraphs: usize,
    pub font_size_bits: u32,
    pub line_spacing_bits: u32,
    pub column_width_bits: u32,
}

impl GeometryKey {
    pub fn new(
        document: u64,
        chapter: usize,
        paragraphs: usize,
        font_size: f32,
        line_spacing: f32,
        column_width: f32,
    ) -> Self {
        Self {
            document,
            chapter,
            paragraphs,
            font_size_bits: font_size.to_bits(),
            line_spacing_bits: line_spacing.to_bits(),
            column_width_bits: column_width.to_bits(),
        }
    }
}

/// The paragraphs a windowed chapter builds, and the space the rest take.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphWindow {
    pub range: std::ops::Range<usize>,
    /// Space above the first realised paragraph.
    pub above: f32,
    /// Space below the last realised paragraph.
    pub below: f32,
}

impl ChapterGeometry {
    /// Measure every paragraph with the same shaper and metrics the reading
    /// pane paints with. Shaping thousands of paragraphs takes long enough to
    /// drop frames, so the app runs this off the UI thread and windows over
    /// [`Self::estimate`] until it lands.
    pub fn measure<'t>(
        key: GeometryKey,
        texts: impl IntoIterator<Item = &'t str>,
        font_size: f32,
        line_spacing: f32,
        column_width: f32,
    ) -> Self {
        let metrics =
            crate::pagination::PageMetrics::new(column_width, 4096.0, font_size, line_spacing);
        let line = metrics.line_height();
        let heights = texts
            .into_iter()
            .map(|text| metrics.lines_for_text(text).max(1) as f32 * line)
            .collect();
        Self::from_heights(key, heights, font_size * PARAGRAPH_GAP)
    }

    /// A quick stand-in for [`Self::measure`]: the first few paragraphs are
    /// shaped exactly, and the rest are sized by their length at the
    /// characters per line those few came out at.
    pub fn estimate<'t>(
        key: GeometryKey,
        texts: impl IntoIterator<Item = &'t str>,
        font_size: f32,
        line_spacing: f32,
        column_width: f32,
    ) -> Self {
        const SAMPLE: usize = 24;
        let metrics =
            crate::pagination::PageMetrics::new(column_width, 4096.0, font_size, line_spacing);
        let line = metrics.line_height();
        let texts: Vec<&str> = texts.into_iter().collect();
        // A paragraph's last line is on average half full, so it counts as
        // half a line when working out how many characters a full one holds.
        let (mut chars, mut full_lines) = (0usize, 0.0f32);
        let mut heights: Vec<f32> = texts
            .iter()
            .take(SAMPLE)
            .map(|text| {
                let count = metrics.lines_for_text(text).max(1);
                chars += text.chars().count();
                full_lines += count as f32 - 0.5;
                count as f32 * line
            })
            .collect();
        // A prose line holds roughly 1.9 characters per em of column.
        let per_line = if full_lines > 0.0 && chars as f32 > full_lines {
            chars as f32 / full_lines
        } else {
            (column_width / font_size.max(1.0) * 1.9).max(1.0)
        };
        heights.extend(
            texts
                .iter()
                .skip(SAMPLE)
                .map(|text| (text.chars().count() as f32 / per_line).ceil().max(1.0) * line),
        );
        Self::from_heights(key, heights, font_size * PARAGRAPH_GAP)
    }

    fn from_heights(key: GeometryKey, heights: Vec<f32>, gap: f32) -> Self {
        let mut tops = Vec::with_capacity(heights.len());
        let mut y = 0.0;
        for height in &heights {
            tops.push(y);
            y += height + gap;
        }
        Self {
            key,
            heights,
            tops,
            gap,
        }
    }

    pub fn key(&self) -> GeometryKey {
        self.key
    }

    /// The total height of the prose: every paragraph and the gaps between.
    pub fn total(&self) -> f32 {
        match (self.tops.last(), self.heights.last()) {
            (Some(top), Some(height)) => top + height,
            _ => 0.0,
        }
    }

    /// The paragraphs within `overscan` of the band `offset..offset + viewport`
    /// of the prose, and the space above and below them.
    pub fn window(&self, offset: f32, viewport: f32, overscan: f32) -> ParagraphWindow {
        let count = self.heights.len();
        let from = (offset - overscan).max(0.0);
        let to = offset + viewport.max(0.0) + overscan;
        let start = self
            .tops
            .iter()
            .zip(&self.heights)
            .position(|(top, height)| top + height >= from)
            // Scrolled past the end (a stale offset after a relayout):
            // realise the last paragraph rather than none.
            .unwrap_or(count.saturating_sub(1));
        let end = self.tops[start..]
            .iter()
            .position(|top| *top > to)
            .map_or(count, |offset| start + offset)
            .max((start + 1).min(count));
        // The column puts one gap between a spacer and the paragraphs, so
        // each spacer is the hidden run minus that gap: above, the run up to
        // the first realised paragraph's top; below, from the end of the last
        // realised one to the end of the prose.
        let above = if start == 0 {
            0.0
        } else {
            (self.tops[start] - self.gap).max(0.0)
        };
        let below = if end >= count {
            0.0
        } else {
            (self.total() - self.tops[end]).max(0.0)
        };
        ParagraphWindow {
            range: start..end,
            above,
            below,
        }
    }
}

// The gap between paragraphs, as a multiple of the font size.
const PARAGRAPH_GAP: f32 = 0.7;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod selection_tests;
