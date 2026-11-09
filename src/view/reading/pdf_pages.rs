//! PDF page geometry, rotation, note marks and text selection.

use super::*;

/// Compose the reader rotation with the first arranged occurrence of a source page.
///
/// Reordering keeps source identity; a source absent from the arrangement uses only
/// the reader rotation. Merged pages have no source bitmap in the open document.
#[must_use]
pub fn effective_page_rotation(
    model: &Model,
    source_index: usize,
) -> crate::viewport::PageRotation {
    let own = model
        .arrangement
        .entries()
        .iter()
        .find(|e| e.source == crate::pages::PageSource::Original(source_index))
        .map(|e| e.rotation)
        .unwrap_or(crate::viewport::PageRotation::NONE);
    model.rotation.compose(own)
}

// The size of a note marker in the page margin.
pub(super) const NOTE_MARKER: f32 = 28.0;

// Only note markers intercept the pointer; the remaining page must keep text drag selection
// available.
pub(super) fn pdf_annotation_overlay<'a>(
    model: &'a Model,
    page_index: usize,
    base: Element<'a, Msg>,
    page_w: f32,
    page_h: f32,
    rotation: crate::viewport::PageRotation,
) -> Element<'a, Msg> {
    if model.format != Format::Pdf {
        return base;
    }
    let Some(marks) = model.pdf_overlay_annotations.get(&page_index) else {
        return base;
    };
    if marks.is_empty() {
        return base;
    }
    let mut layers = vec![base];
    for mark in marks.iter() {
        let [x, y, w, h] = displayed_page_rect(mark.rect, page_w, page_h, rotation);
        if mark.tints_text {
            // The annotation's own colour, from the document, at the
            // strength a highlighter leaves on paper.
            let [r, g, b] = mark
                .color
                .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            layers.push(placed(
                enz_surface(Space::new())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .fill(Color::from_rgba8(r, g, b, 0.35))
                    .border_style(BorderStyle::None)
                    .radius_size(RadiusSize::Sm)
                    .into_element(),
                x,
                y,
            ));
        }
        if model.selected_annotation == Some(mark.id) {
            layers.push(placed(
                enz_surface(Space::new())
                    .width(Length::Fixed(w + 6.0))
                    .height(Length::Fixed(h + 6.0))
                    .fill(Color::TRANSPARENT)
                    .border_style(BorderStyle::Accent)
                    .line_size(LineSize::Thick)
                    .radius_size(RadiusSize::Sm)
                    .into_element(),
                (x - 3.0).max(0.0),
                (y - 3.0).max(0.0),
            ));
        }
        if let Some(note) = &mark.note {
            let [mx, my] = note_marker_origin([x, y, w, h], page_w);
            // An opaque surface preserves contrast over white paper.
            let marker = enzui_iced_widgets::toolbar_icon::enz_toolbar_icon(
                "message-square-text",
                Msg::OpenNoteEditorForEdit(mark.id),
            )
            .id(enzui_iced_core::identity::derive_id(
                "reader.pdf.comment",
                &mark.id.to_string(),
            ))
            .tooltip(note_preview(note))
            .aria_label(format!("Comment: {}", note_preview(note)))
            .into_element();
            layers.push(placed(
                enz_surface(marker)
                    .radius(NOTE_MARKER / 2.0)
                    .border_style(BorderStyle::Thin)
                    .elevation(2)
                    .into_element(),
                mx,
                my,
            ));
        }
    }
    iced::widget::Stack::with_children(layers)
        .width(Length::Fixed(page_w))
        .height(Length::Fixed(page_h))
        .clip(true)
        .into()
}

fn displayed_page_rect(
    [x, y, w, h]: [f32; 4],
    page_w: f32,
    page_h: f32,
    rotation: crate::viewport::PageRotation,
) -> [f32; 4] {
    let [x, y, w, h] = match rotation.quarters() {
        1 => [1.0 - y - h, x, h, w],
        2 => [1.0 - x - w, 1.0 - y - h, w, h],
        3 => [y, 1.0 - x - w, h, w],
        _ => [x, y, w, h],
    };
    [x * page_w, y * page_h, w * page_w, h * page_h]
}

// Where a note's marker sits: in the margin left of the passage, level with
// its first line, or just right of it when the passage starts at the page
// edge.
pub(crate) fn note_marker_origin([x, y, w, h]: [f32; 4], page_w: f32) -> [f32; 2] {
    let gap = 6.0;
    let left = x - NOTE_MARKER - gap;
    let mx = if left >= 0.0 {
        left
    } else {
        (x + w + gap).min(page_w - NOTE_MARKER).max(0.0)
    };
    let line = h.min(NOTE_MARKER);
    [mx, (y + line / 2.0 - NOTE_MARKER / 2.0).max(0.0)]
}

pub(super) fn note_preview(note: &str) -> String {
    const MAX: usize = 140;
    let note = note.trim();
    match note.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}\u{2026}", note[..cut].trim_end()),
        None => note.to_string(),
    }
}

// `content` with its top-left corner at `(x, y)` in a layer of the page.
pub(super) fn placed<'a>(content: Element<'a, Msg>, x: f32, y: f32) -> Element<'a, Msg> {
    container(content)
        .padding(iced::Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

// Select using source glyph boxes; textless pages remain plain rasters.
pub(super) fn selectable_page<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    page_index: usize,
    raster: Element<'a, Msg>,
    page_w: f32,
    page_h: f32,
    rotation: crate::viewport::PageRotation,
) -> Element<'a, Msg> {
    use enzui_iced_widgets::glyph_selection::enz_glyph_selection;

    let Some(text) = model.pdf_text.get(page_index) else {
        return raster;
    };
    if text.glyphs.is_empty() || text.width_pts <= 0.0 || text.height_pts <= 0.0 {
        return raster;
    }
    let upright_width = if rotation.swaps_axes() {
        page_h
    } else {
        page_w
    };
    let scale = upright_width / text.width_pts;
    use enzui_iced_core::image_transform::QuarterTurn;
    let turn = match rotation.quarters() {
        1 => QuarterTurn::Clockwise90,
        2 => QuarterTurn::Clockwise180,
        3 => QuarterTurn::Clockwise270,
        _ => QuarterTurn::None,
    };
    let selection = pdf_selected_range(model, page_index, text);
    let area: Element<'a, Msg> = enz_glyph_selection(raster, &text.glyphs)
        .scale(scale)
        .rotation(turn)
        .selection(selection)
        .on_select(move |range| Msg::PdfTextSelected {
            page: page_index,
            range,
            finished: false,
        })
        .on_finish(move |range| Msg::PdfTextSelected {
            page: page_index,
            range,
            finished: true,
        })
        .on_clear(Msg::ClearPdfSelection)
        .into();
    let area: Element<'a, Msg> = container(area)
        .id(crate::view::widgets::page_selection_id(page_index))
        .width(Length::Fixed(page_w))
        .height(Length::Fixed(page_h))
        .into();
    match model
        .pdf_selection
        .as_ref()
        .filter(|selection| selection.page == page_index && model.pdf_selection_actions)
    {
        Some(selection) => {
            with_selection_actions(theme, area, text, selection, (page_w, page_h), rotation)
        }
        None => area,
    }
}

// Anchor actions to the selection's final line so they stay near the drag endpoint and scroll
// with the page.
pub(super) fn with_selection_actions<'a>(
    theme: &'a EnzTheme,
    page: Element<'a, Msg>,
    text: &'a crate::pdf::PdfPageText,
    selection: &'a crate::model::PdfSelection,
    (page_w, page_h): (f32, f32),
    rotation: crate::viewport::PageRotation,
) -> Element<'a, Msg> {
    // One rect per line; the lowest is the last line.
    let rects = text.glyphs.range_rects(selection.range.clone());
    let Some(&[x, y, w, h]) = rects.iter().max_by(|a, b| a[1].total_cmp(&b[1])) else {
        return page;
    };
    let [x, y, w, h] = displayed_page_rect(
        [
            x / text.width_pts,
            y / text.height_pts,
            w / text.width_pts,
            h / text.height_pts,
        ],
        page_w,
        page_h,
        rotation,
    );
    let anchor = iced::widget::Space::new().width(w).height(h);
    // Escape dismisses the strip; a press elsewhere on the page needs no
    // backdrop, because the selection layer under it already turns a click
    // into a cleared selection and a drag into a new one.
    let anchored = enzui_iced_widgets::anchored_overlay::dismiss_on_escape(
        enzui_iced_widgets::anchored_overlay::anchored_overlay(
            anchor,
            Some(overlays::pdf_selection_actions(theme)),
        )
        .placement(Placement::new(Side::Bottom).gap(theme.spacing.sm())),
        true,
        || Msg::DismissPdfSelectionActions,
    );
    iced::widget::stack![page, placed(anchored, x, y)].into()
}

// The glyph range to paint as selected on `page_index`: the live selection,
// or else the active search hit's line.
pub(crate) fn pdf_selected_range(
    model: &Model,
    page_index: usize,
    text: &crate::pdf::PdfPageText,
) -> Option<std::ops::Range<usize>> {
    match model.pdf_selection.as_ref() {
        Some(selection) if selection.page == page_index => Some(selection.range.clone()),
        _ => model
            .active_pdf_search_hit()
            .filter(|hit| hit.page == page_index)
            .and_then(|hit| glyphs_in(text, hit.rect)),
    }
}

// The glyphs inside a PDF user-space rectangle (a search hit's line), as one
// range.
pub(super) fn glyphs_in(
    text: &crate::pdf::PdfPageText,
    rect: [f32; 4],
) -> Option<std::ops::Range<usize>> {
    let [x0, y0, x1, y1] = rect;
    let (top, bottom) = (text.height_pts - y1, text.height_pts - y0);
    let inside = |g: &enzui_iced_core::glyph_selection::Glyph| {
        let cx = g.rect[0] + g.rect[2] / 2.0;
        let cy = g.rect[1] + g.rect[3] / 2.0;
        cx >= x0 - 0.5 && cx <= x1 + 0.5 && cy >= top - 0.5 && cy <= bottom + 0.5
    };
    let glyphs = text.glyphs.glyphs();
    let start = glyphs.iter().position(inside)?;
    let end = glyphs.iter().rposition(inside)? + 1;
    Some(start..end)
}

// Share page heights between painting, raster admission and scroll targets, including before
// rasters arrive.
pub(crate) fn page_display_heights(model: &Model, theme: &EnzTheme, window_width: f32) -> Vec<f32> {
    let fit_w = page_fit_width(model, theme, window_width);
    let height = |extent: &crate::raster::PageExtent, rotation| {
        page_display_box(model, theme, (extent.width, extent.height), fit_w, rotation).1
    };
    let mut heights: Vec<f32> = model
        .page_extents
        .iter()
        .map(|extent| height(extent, model.rotation))
        .collect();
    // Reverse traversal preserves the first source occurrence, as individual page lookup does.
    for entry in model.arrangement.entries().iter().rev() {
        if let crate::pages::PageSource::Original(index) = entry.source
            && let Some(extent) = model.page_extents.get(index)
        {
            heights[index] = height(extent, model.rotation.compose(entry.rotation));
        }
    }
    heights
}

pub(crate) fn page_display_box(
    model: &Model,
    theme: &EnzTheme,
    native: (u32, u32),
    fit_width: f32,
    rotation: crate::viewport::PageRotation,
) -> (f32, f32) {
    let (width, height) = crate::viewport::page_box(native, fit_width, model.zoom, rotation);
    if model.zoom.mode() == crate::viewport::ZoomMode::FitWidth {
        return (fit_width, height * fit_width / width);
    }
    if model.zoom.mode() != crate::viewport::ZoomMode::FitPage {
        return (width, height);
    }
    // Reserve the floating pager and gutters so it cannot cover the page's last line.
    let available_height = (model.pages_viewport_height
        - super::page_top_inset(&theme.spacing)
        - theme.sizing.control_height
        - 2.0 * theme.spacing.lg())
    .max(1.0);
    let scale = (fit_width / width).min(available_height / height).min(1.0);
    (width * scale, height * scale)
}

// Whether the column shows facing pages: the reader chose them and the
// pane is wide enough for two ([`crate::spread::min_spread_width`]).
pub(crate) fn spread_engaged(model: &Model, window_width: f32) -> bool {
    model.page_view_mode == crate::viewport::PageViewMode::Continuous
        && model.spread_mode == crate::spread::SpreadMode::TwoPage
        && window_width >= crate::spread::min_spread_width()
}

// The width a page fits before zoom: the pane, or half of it beside its
// facing page in a spread.
pub(crate) fn page_fit_width(model: &Model, theme: &EnzTheme, window_width: f32) -> f32 {
    let pane = window_width - 2.0 * theme.spacing.xl();
    if spread_engaged(model, window_width) {
        ((pane - theme.spacing.lg()) / 2.0).clamp(100.0, MAX_PAGE_WIDTH)
    } else {
        pane.clamp(200.0, MAX_PAGE_WIDTH)
    }
}

// One row of the page column: a page, or a spread's facing pair. In a spread
// a missing side (the cover's left, a last page's right) stays an empty slot
// so the spine keeps its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageRow {
    pub(crate) left: Option<usize>,
    pub(crate) right: Option<usize>,
    pub(crate) facing: bool,
}

impl PageRow {
    pub(crate) fn first(self) -> usize {
        self.left.or(self.right).unwrap_or(0)
    }

    pub(crate) fn last(self) -> usize {
        self.right.or(self.left).unwrap_or(0)
    }
}

// The rows of the column, top to bottom, for a document of `pages` pages.
pub(crate) fn page_rows(model: &Model, window_width: f32, pages: usize) -> Vec<PageRow> {
    if model.page_view_mode == crate::viewport::PageViewMode::SinglePage {
        return (pages > 0)
            .then(|| PageRow {
                left: Some(model.current_page.min(pages - 1)),
                right: None,
                facing: false,
            })
            .into_iter()
            .collect();
    }
    if spread_engaged(model, window_width) {
        crate::spread::compute_spreads(pages)
            .into_iter()
            .map(|spread| PageRow {
                left: spread.left,
                right: spread.right,
                facing: true,
            })
            .collect()
    } else {
        (0..pages)
            .map(|page| PageRow {
                left: Some(page),
                right: None,
                facing: false,
            })
            .collect()
    }
}

pub(crate) fn page_row_of(model: &Model, window_width: f32, page: usize, pages: usize) -> usize {
    if model.page_view_mode == crate::viewport::PageViewMode::SinglePage {
        return 0;
    }
    if spread_engaged(model, window_width) {
        crate::spread::spread_for_page(page, pages).0
    } else {
        page
    }
}

// The first page of the row after (`forward`) or before the one holding
// `page`, or `None` at either end of the document.
pub(crate) fn adjacent_row_page(
    model: &Model,
    window_width: f32,
    page: usize,
    forward: bool,
) -> Option<usize> {
    let pages = model.page_count;
    if model.page_view_mode == crate::viewport::PageViewMode::SinglePage {
        return if forward {
            page.checked_add(1).filter(|next| *next < pages)
        } else {
            page.checked_sub(1)
        };
    }
    let rows = page_rows(model, window_width, pages);
    let row = page_row_of(model, window_width, page, pages);
    let target = if forward {
        row.checked_add(1)?
    } else {
        row.checked_sub(1)?
    };
    rows.get(target).map(|row| row.first())
}

// Spread row height is the taller page; scroll mapping and raster bands must use these same
// rows.
pub(crate) fn page_row_heights(model: &Model, theme: &EnzTheme, window_width: f32) -> Vec<f32> {
    let heights = page_display_heights(model, theme, window_width);
    if model.page_view_mode == crate::viewport::PageViewMode::SinglePage {
        return heights
            .get(model.current_page)
            .copied()
            .into_iter()
            .collect();
    }
    if !spread_engaged(model, window_width) {
        return heights;
    }
    page_rows(model, window_width, heights.len())
        .into_iter()
        .map(|row| {
            [row.left, row.right]
                .into_iter()
                .flatten()
                .map(|page| heights[page])
                .fold(0.0, f32::max)
        })
        .collect()
}

// Use the rendered row heights, top padding and inter-row gaps for both page jumps and scroll
// tracking.
pub(crate) fn page_scroll_offset(
    model: &Model,
    theme: &EnzTheme,
    window_width: f32,
    target_index: usize,
) -> f32 {
    let rows = page_row_heights(model, theme, window_width);
    let target = page_row_of(model, window_width, target_index, model.page_extents.len())
        .min(rows.len().saturating_sub(1));
    super::page_top_inset(&theme.spacing)
        + rows
            .iter()
            .take(target)
            .map(|h| h + theme.spacing.lg())
            .sum::<f32>()
}

// Resolve the page whose painted box contains the viewport's vertical centre.
// This keeps the pager selection following ordinary wheel/touch scrolling;
// programmatic jumps set the same index directly before the scroll operation.
pub(crate) fn page_at_scroll_offset(
    model: &Model,
    theme: &EnzTheme,
    window_width: f32,
    offset: f32,
    viewport_height: f32,
) -> usize {
    let rows = page_row_heights(model, theme, window_width);
    if rows.is_empty() {
        return 0;
    }
    let centre = offset.max(0.0) + viewport_height.max(0.0) / 2.0;
    let mut best = 0;
    let mut y = super::page_top_inset(&theme.spacing);
    for (index, height) in rows.iter().enumerate() {
        if y <= centre {
            best = index;
        } else {
            break;
        }
        y += height + theme.spacing.lg();
    }
    page_rows(model, window_width, model.page_extents.len())
        .get(best)
        .map_or(0, |row| row.first())
}

// The pages the viewport can currently reach, as [`crate::raster::band`] defines it.
//
// Before the first scroll event the window height stands in for the viewport; zero would
// leave a fresh document's second page blank.
pub(crate) fn visible_page_band(
    model: &Model,
    theme: &EnzTheme,
    window_width: f32,
    window_height: f32,
) -> Option<std::ops::RangeInclusive<usize>> {
    let rows = page_row_heights(model, theme, window_width);
    let band = crate::raster::band(
        &rows,
        super::page_top_inset(&theme.spacing),
        theme.spacing.lg(),
        model.pages_scroll_offset,
        viewport_height_for(model, window_height),
    )?;
    let page_rows = page_rows(model, window_width, model.page_extents.len());
    Some(page_rows[*band.start()].first()..=page_rows[*band.end()].last())
}

// Use one viewport height for reducer raster admission and view realization, falling back to
// window height before measurement.
pub(super) fn viewport_height_for(model: &Model, window_height: f32) -> f32 {
    if model.pages_viewport_height > 1.0 {
        model.pages_viewport_height
    } else {
        window_height.max(1.0)
    }
}

// A spacer replacing k pages carries k−1 internal gaps so later pages keep their original
// scroll offsets.
pub(super) fn run_spacer_height(heights: &[f32], gap: f32) -> f32 {
    if heights.is_empty() {
        return 0.0;
    }
    heights.iter().sum::<f32>() + (heights.len() - 1) as f32 * gap
}
