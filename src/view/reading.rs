//! PDF/CBZ page views and character-anchored EPUB reading surfaces.

use enzui_iced_core::placement::{Placement, Side};
use enzui_iced_core::semantics::{HeadingLevel, Role, Semantics};
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::anchored_overlay::{
    dismissible_anchored_menu, dismissible_anchored_menu_at,
};
use enzui_iced_widgets::enz_text::{TextSize, enz_text};
use enzui_iced_widgets::scroll_view::enz_scroll_view;
use enzui_iced_widgets::semantics_container::SemanticsContainer;
use enzui_iced_widgets::status_page::enz_status_page;
use enzui_iced_widgets::surface::{BorderStyle, LineSize, RadiusSize, enz_surface};
use iced::widget::{Space, column, container};
use iced::{Border, Color, Element, Length};

use crate::message::Msg;
use crate::model::{Format, Model, rgb};
use crate::reading_pane::{chapter_view, column_width_for};

/// Stable id for the EPUB reading scroll view.
///
/// Exposed as a function so the reducer can build a `snap_to` task that
/// targets the same widget the view creates.
pub fn epub_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("reader-epub-scroll")
}

/// Stable id for the PDF/CBZ all-pages scroll view.
///
/// The numbered pager and the existing page-key commands target this exact
/// scroll owner through `iced::widget::operation::scroll_to`.
pub fn pages_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("reader-pages-scroll")
}

pub fn view<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Msg> {
    match model.format {
        Format::Epub => epub_view(model, theme, window_width),
        // The rendered-page column realises only the reachable band, so it needs the height.
        Format::Pdf | Format::Cbz => {
            // ShellGeometry describes the destination allocation. During a sidebar
            // transition, fit-width pages and their anchor need the same live width.
            let pages = enzui_iced_widgets::responsive::enz_responsive(move |size| {
                pages_view(model, theme, size.width, window_height)
            });
            iced::widget::keyed_column([(model.document_load_attempt, pages.into())])
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}

// A comfortable maximum on-screen page width, so an A4 page is not blown up on a
// desktop while still fitting a 375px phone.
//
// A literal rather than a `container_*` token: it caps a page raster tuned to A4/Letter
// aspect, not a prose line length.
const MAX_PAGE_WIDTH: f32 = 820.0;

fn epub_view<'a>(model: &'a Model, theme: &'a EnzTheme, window_width: f32) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    let Some(epub) = model.epub.as_ref() else {
        return container(enz_status_page("No document").icon("book"))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };
    let colors = super::reading_colors(model.reading_theme, theme);
    let surface_bg = rgb(colors.surface_bg);
    let card_bg = rgb(colors.card_bg);
    let sep = rgb(colors.separator);

    let chapter = model.epub_current_chapter;
    // The reader-visible heading, not the raw `<title>` metadata.
    let title = epub
        .load
        .chapters
        .get(chapter)
        .map(|c| c.display_heading().to_string())
        .unwrap_or_default();

    // `load_epub` strips the leading in-body <h1>, so the heading is drawn once here. It
    // uses the reading theme's `fg`, not the chrome text tone, which is invisible on a light
    // reading card under dark chrome, and rides inside the body column to share its edge.
    let heading: Element<'a, Msg> = container(
        enz_text(title)
            .size(TextSize::Title2)
            .color(rgb(colors.fg))
            .heading(HeadingLevel::Page),
    )
    .padding(iced::Padding {
        top: 0.0,
        right: 0.0,
        bottom: spacing.md(),
        left: 0.0,
    })
    .into();

    // Paginated mode paints one page from `model.epub_pages[page]`, as computed by
    // `App::repaginate`; content beyond it is absent from the tree. The heading belongs to
    // the first page only.
    let paginated = model.epub_is_paginated();
    let heading: Option<Element<'a, Msg>> = if paginated && model.epub_page_index() > 0 {
        None
    } else {
        Some(heading)
    };

    // The chapter body as a centred readable column, with the heading as its
    // first row so the two share one measure.
    let paragraphs = if paginated {
        Some(&model.epub_page_content)
    } else {
        epub.chapter_paragraphs.get(&chapter)
    };
    let window = model.epub_paragraph_window(column_width_for(window_width));
    // The selection document holds only the realised paragraphs, so its
    // indices are shifted to the chapter's before they reach the model.
    let first = window.as_ref().map_or(0, |window| window.range.start);
    let body: Element<'a, Msg> = match paragraphs {
        Some(paras) if !paras.is_empty() => chapter_view(
            heading,
            paras,
            &colors,
            model.epub_font_size as f32,
            model.epub_line_spacing,
            column_width_for(window_width),
            // make_link: footnote target id → String payload.
            |id| id.to_string(),
            Msg::FootnoteClicked,
            Msg::CopiedSelection,
            move |selection: Option<enzui_iced_widgets::selection_scope::ScopeSelection>| {
                Msg::SelectionChanged(
                    selection
                        .map(|selection| crate::reading_pane::chapter_selection(selection, first)),
                )
            },
            model.selection_clear_generation,
            window.as_ref(),
        ),
        // Empty chapter: still show the heading (in the same centred column) so
        // the title is not lost, followed by the placeholder line.
        _ => chapter_view(
            heading,
            &[],
            &colors,
            model.epub_font_size as f32,
            model.epub_line_spacing,
            column_width_for(window_width),
            |id| id.to_string(),
            Msg::FootnoteClicked,
            Msg::CopiedSelection,
            Msg::SelectionChanged,
            model.selection_clear_generation,
            None,
        ),
    };

    // Keep the overlay host mounted; reparenting the selection scope discards its internal
    // selection.
    let selection = model
        .active_selection
        .as_ref()
        .filter(|selection| !selection.text.trim().is_empty());
    let body = dismissible_anchored_menu_at(
        body,
        selection.map(|_| selection_actions(theme)),
        selection.map_or(iced::Rectangle::default(), |selection| selection.bounds),
        Placement::new(Side::Bottom).gap(spacing.sm()),
        || Msg::DismissSelectionActions,
    );

    // Rich text reports the clicked target but not its glyph rectangle. Anchor
    // the note to the readable prose column; opening it never changes chapter,
    // scroll fraction, or paginated character offset.
    let body = dismissible_anchored_menu(
        body,
        model
            .active_footnote
            .as_ref()
            .map(|note| footnote_popover(note, theme)),
        Placement::new(Side::Right).gap(spacing.sm()),
        || Msg::DismissFootnote,
    );

    // A container rather than `enz_card`: the card paints per-book `ReadingTheme` colours,
    // and `enz_card` takes its surface from chrome tokens with no way to cascade the
    // reading-theme ink to the body text.
    let card = container(body)
        .width(Length::Fill)
        // A page fills its box so the bottom margin is painted in the page colour; in
        // scrolled mode the card shrinks to its content.
        .height(if paginated {
            Length::Fill
        } else {
            Length::Shrink
        })
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(card_bg.into()),
            border: Border {
                width: 1.0,
                color: sep,
                radius: 0.0.into(),
            },
            text_color: Some(rgb(colors.fg)),
            ..Default::default()
        });

    // Paginated content is clipped to the measured page box without an inner scroller.
    if paginated {
        // The sensor reports the page box's laid-out height, which pages are
        // cut to, so a page never runs past the bottom whatever the chrome
        // above it or the font size.
        let page = container(
            container(card)
                // No bottom gutter: exact shaper breaks already stop at a whole line.
                .padding(iced::Padding {
                    top: page_top_inset(spacing),
                    right: spacing.xl(),
                    bottom: 0.0,
                    left: spacing.xl(),
                })
                .center_x(Length::Fill)
                .width(Length::Fill)
                // Fill the viewport so the page box IS the viewport; the card
                // (height `Shrink`) sits at its top, and `clip` cuts anything
                // that would run past the bottom edge.
                .height(Length::Fill)
                .clip(true),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(surface_bg.into()),
            ..Default::default()
        });
        // A page does not scroll; the wheel turns it instead.
        let page = iced::widget::mouse_area(page).on_scroll(|delta| {
            Msg::EpubPageWheel(match delta {
                iced::mouse::ScrollDelta::Lines { y, .. } => y,
                iced::mouse::ScrollDelta::Pixels { y, .. } => y / WHEEL_PIXELS_PER_LINE,
            })
        });
        return iced::widget::sensor(page)
            .on_resize(|size: iced::Size| Msg::EpubPageBoxMeasured(size.height))
            .into();
    }

    let generation = model.position_restore_generation;
    container(
        enz_scroll_view(
            container(card)
                .padding(iced::Padding {
                    top: page_top_inset(spacing),
                    right: spacing.xl(),
                    bottom: spacing.xl(),
                    left: spacing.xl(),
                })
                .center_x(Length::Fill)
                .width(Length::Fill),
        )
        // Naming the painted reading background makes the overlay thumb ink contrast with
        // the page rather than with the chrome palette.
        .backdrop(rgb(colors.bg))
        // Track the scroll position so a resize can restore it.
        .id(epub_scroll_id())
        .on_scroll(move |viewport| Msg::EpubScrolled {
            generation,
            fraction: viewport.relative_offset().y,
            offset: viewport.absolute_offset().y,
            viewport_height: viewport.bounds().height,
        })
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_t: &iced::Theme| container::Style {
        background: Some(surface_bg.into()),
        ..Default::default()
    })
    .into()
}

#[path = "reading/overlays.rs"]
mod overlays;

use overlays::{footnote_popover, selection_actions};

#[path = "reading/pdf_pages.rs"]
mod pdf_pages;

pub use pdf_pages::effective_page_rotation;
use pdf_pages::*;
pub(crate) use pdf_pages::{
    adjacent_row_page, page_at_scroll_offset, page_display_box, page_fit_width, page_scroll_offset,
    visible_page_band,
};

// Reserve the final page geometry while loading or failed so placeholders cannot shift scroll
// targets.
fn page_placeholder<'a>(
    page_number: usize,
    page_w: f32,
    page_h: f32,
    failed: bool,
) -> Element<'a, Msg> {
    use enzui_iced_core::async_image::{FailureReason, Generation, ImageState};
    use enzui_iced_widgets::async_image::{EnzAsyncImageFit, enz_async_image_view};

    // One in-flight request per page, so the generation stays at the first. A page poppler
    // produced nothing for is `Failed`, since the reducer stops asking for it.
    let state: ImageState<iced::widget::image::Handle> = if failed {
        ImageState::Failed(FailureReason::Service(PAGE_UNAVAILABLE.to_string()))
    } else {
        ImageState::Loading(Generation::FIRST)
    };
    // The failure sentence goes in the accessible name: the widget renders a failure only
    // as its stable tag (`service-error`).
    let alt = if failed {
        format!("Document page {page_number}: {PAGE_UNAVAILABLE}")
    } else {
        format!("Document page {page_number}")
    };
    enz_async_image_view(&state)
        .fit(EnzAsyncImageFit::Contain)
        .alt_text(alt)
        .width(Length::Fixed(page_w))
        .height(Length::Fixed(page_h))
        .into_element()
}

/// What a page whose rasterisation came back empty is stamped with.
///
/// Pre-redacted per the `FailureReason::Service` contract and free of poppler text,
/// which can echo a password on some builds.
pub const PAGE_UNAVAILABLE: &str = "this page could not be rendered";

fn pages_view<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    if model.page_extents.is_empty() {
        // Loading, nothing open and empty read differently; precedence follows
        // `states::thumbnails_state`.
        let (title, desc) = if model.loading {
            (
                super::states::LOADING,
                super::states::LOADING_DOCUMENT.to_string(),
            )
        } else if let Some(err) = model.error.as_deref() {
            ("Could not open", err.to_string())
        } else if model.file_path.is_none() {
            (
                super::states::NO_DOCUMENT_TITLE,
                super::states::OPEN_FOR_THUMBNAILS.to_string(),
            )
        } else {
            (
                "No pages",
                "This document has no rendered pages.".to_string(),
            )
        };
        return container(enz_status_page(title).icon("file-text").description(desc))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    // Bound the page to a readable width that never overflows the viewport; zoom and
    // rotation apply through `viewport::page_box`.
    let fit_w = page_fit_width(model, theme, window_width);
    let rows = page_row_heights(model, theme, window_width);
    // Only the band builds page elements; spacers hold the rest open (see
    // `run_spacer_height`), so per-frame element count follows the viewport, not the
    // document length.
    let band = crate::raster::band(
        &rows,
        spacing.xl(),
        spacing.lg(),
        model.pages_scroll_offset,
        viewport_height_for(model, window_height),
    )
    .unwrap_or(0..=0);
    let mut col = column![].spacing(spacing.lg()).width(Length::Fill);
    // The widest row realised: zoomed past the pane, the column becomes that
    // wide and scrolls sideways as well as down.
    let mut widest = 0.0f32;
    if *band.start() > 0 {
        col = col.push(
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(run_spacer_height(
                    &rows[..*band.start()],
                    spacing.lg(),
                ))),
        );
    }
    let page_rows = page_rows(model, window_width, model.page_extents.len());
    for row in &page_rows[band.clone()] {
        let left = row
            .left
            .map(|index| page_element(model, theme, index, fit_w));
        let right = row
            .right
            .map(|index| page_element(model, theme, index, fit_w));
        let row_content: Element<'a, Msg> = if row.facing {
            // Facing pages meet at the spine, each aligned to the row's top. A
            // missing side keeps its slot, as wide as the page beside it, so the
            // cover sits on the right and the spine never moves.
            let slot = left.as_ref().or(right.as_ref()).map_or(0.0, |(_, w)| *w);
            widest = widest.max(2.0 * slot + spacing.lg());
            let side = |page: Option<(Element<'a, Msg>, f32)>| {
                page.map_or_else(
                    || iced::widget::Space::new().width(Length::Fixed(slot)).into(),
                    |(page, _)| page,
                )
            };
            iced::widget::Row::with_children([side(left), side(right)])
                .spacing(spacing.lg())
                .into()
        } else {
            let (page, page_w) = left.or(right).expect("a row holds a page");
            widest = widest.max(page_w);
            page
        };
        col = col.push(
            container(row_content)
                .center_x(Length::Fill)
                .width(Length::Fill),
        );
    }
    if *band.end() + 1 < rows.len() {
        col = col.push(
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(run_spacer_height(
                    &rows[*band.end() + 1..],
                    spacing.lg(),
                ))),
        );
    }

    let generation = model.position_restore_generation;
    // A page wider than the pane (zoomed in) gets a column as wide as it and a
    // horizontal scroll, rather than being clipped to the pane.
    let column_width = widest + 2.0 * spacing.xl();
    let wide = column_width > window_width;
    let page_padding = iced::Padding {
        top: page_top_inset(spacing),
        right: spacing.xl(),
        bottom: theme.sizing.control_height + 2.0 * spacing.lg(),
        left: spacing.xl(),
    };
    let pages_column = if wide {
        container(col)
            .padding(page_padding)
            .width(Length::Fixed(column_width))
    } else {
        container(col)
            .padding(page_padding)
            .center_x(Length::Fill)
            .width(Length::Fill)
    };
    let scrolling_pages: Element<'a, Msg> = container(
        enz_scroll_view(pages_column)
            .axis(if wide {
                enzui_iced_widgets::scroll_view::Axis::Both
            } else {
                enzui_iced_widgets::scroll_view::Axis::Vertical
            })
            .id(pages_scroll_id())
            .anchor(move |_, viewport| reflow_scroll_offset(model, theme, viewport.width))
            // PageUp/PageDown are document commands in `shortcuts`; consuming them here too
            // would move twice.
            .keyboard_scroll(false)
            .on_scroll(move |viewport| Msg::PagesScrolled {
                generation,
                offset: viewport.absolute_offset().y,
                viewport_height: viewport.bounds().height,
                viewport_width: viewport.bounds().width,
            })
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    // The pager floats over the bottom of the pages on glass instead of
    // taking a row of its own.
    let placing_bar = model
        .signing
        .as_ref()
        .filter(|draft| draft.placing)
        .map(|_| {
            container(super::sign_dialog::placing_bar(theme))
                .padding(spacing.lg())
                .center_x(Length::Fill)
                .into()
        });
    iced::widget::Stack::with_children(
        [
            scrolling_pages,
            container(floating_pager(model, theme))
                .padding(spacing.lg())
                .align_bottom(Length::Fill)
                .center_x(Length::Fill)
                .into(),
        ]
        .into_iter()
        .chain(placing_bar),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn page_element<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    index: usize,
    fit_w: f32,
) -> (Element<'a, Msg>, f32) {
    // The single rotation read, shared by the layout box and the paint transform.
    let rotation = effective_page_rotation(model, index);
    let extent = model.page_extents[index];
    let (page_w, page_h) =
        page_display_box(model, theme, (extent.width, extent.height), fit_w, rotation);
    let page_number = index + 1;
    let named_page: Element<'a, Msg> = match model.pages.get(index) {
        Some(page) => {
            let raster: Element<'a, Msg> = iced::widget::image(page.handle.clone())
                .width(Length::Fixed(page_w))
                .height(Length::Fixed(page_h))
                // `Solid`, not `Floating`: a floating rotation keeps the portrait
                // layout box and clips a turned page.
                .rotation(iced::Rotation::Solid(iced::Radians(rotation.radians())))
                .into();
            let page = selectable_page(model, theme, index, raster, page_w, page_h, rotation);
            // A right-click on the page opens its menu: bookmark, rotate,
            // extract. The left button stays with text selection.
            let page = enzui_iced_widgets::context_target::enz_context_target(
                pdf_annotation_overlay(model, index, page, page_w, page_h, rotation),
                move |at| Msg::OpenContextMenu(crate::context_menu::ContextTarget::Page(index), at),
            )
            .width(Length::Fixed(page_w))
            .height(Length::Fixed(page_h));
            // While a signature is being placed, a layer over the page
            // takes the click and shows the signature under the pointer.
            // The page stays the stack's first child either way.
            let page: Element<'a, Msg> = iced::widget::Stack::with_children(
                std::iter::once(page.into_element())
                    .chain(signature_placing_layer(model, index, page_w, page_h)),
            )
            .into();
            SemanticsContainer::new(
                page,
                Semantics::new(Role::Group).label(format!("Document page {page_number}")),
            )
            .id(format!("reader-document-page-{page_number}").into())
            .into()
        }
        None => page_placeholder(
            page_number,
            page_w,
            page_h,
            model.raster_failed.contains(&index),
        ),
    };
    (named_page, page_w)
}

// The placing layer over page `index`: the click target, with the
// signature drawn where it would land.
fn signature_placing_layer<'a>(
    model: &'a Model,
    index: usize,
    page_w: f32,
    page_h: f32,
) -> Option<Element<'a, Msg>> {
    let draft = model.signing.as_ref().filter(|draft| draft.placing)?;
    let width_pts = model
        .pdf_text
        .get(index)
        .map(|text| text.width_pts)
        .filter(|w| *w > 0.0)
        .unwrap_or(612.0);
    let ghost: Element<'a, Msg> = match (draft.pointer, draft.preview.as_ref()) {
        (Some((page, at, _)), Some(handle)) if page == index => {
            let (iw, ih) = draft.preview_size;
            let w = crate::signing::SIGNATURE_WIDTH_PTS * page_w / width_pts;
            let h = w * ih as f32 / iw.max(1) as f32;
            container(
                iced::widget::image(handle.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .opacity(0.85_f32),
            )
            .padding(iced::Padding {
                top: (at.y - h / 2.0).max(0.0),
                left: (at.x - w / 2.0).max(0.0),
                right: 0.0,
                bottom: 0.0,
            })
            .into()
        }
        _ => Space::new().into(),
    };
    let displayed = iced::Size::new(page_w, page_h);
    Some(
        iced::widget::mouse_area(
            container(ghost)
                .width(Length::Fixed(page_w))
                .height(Length::Fixed(page_h)),
        )
        .on_move_coalesced(move |at| Msg::SignaturePointer {
            page: index,
            at,
            displayed,
        })
        .on_press(Msg::SignatureClicked(index))
        .interaction(iced::mouse::Interaction::Crosshair)
        .into(),
    )
}

// Navigation overlays the page so it reserves no footer band.
fn floating_pager<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    use enzui_iced_widgets::glass::enz_glass;
    use enzui_iced_widgets::text_field::{TextFieldSize, enz_text_field_owned};
    use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
    let spacing = &theme.spacing;
    let current = model.current_page.saturating_add(1);
    let count = model.page_count.max(1);
    let entry = model
        .page_entry
        .clone()
        .unwrap_or_else(|| current.to_string());
    let first = enz_toolbar_icon("chevrons-left", Msg::GoToPage(0))
        .id(iced::widget::Id::new("reader.page.first"))
        .aria_label("First page")
        .tooltip("First page")
        .enabled(current > 1)
        .into_element();
    let last = enz_toolbar_icon("chevrons-right", Msg::GoToPage(count.saturating_sub(1)))
        .id(iced::widget::Id::new("reader.page.last"))
        .aria_label("Last page")
        .tooltip("Last page")
        .enabled(current < count)
        .into_element();
    let previous = enz_toolbar_icon("chevron-left", Msg::PrevPage)
        .id(iced::widget::Id::new("reader.page.previous"))
        .aria_label("Previous page")
        .tooltip("Previous page")
        .enabled(current > 1)
        .into_element();
    let next = enz_toolbar_icon("chevron-right", Msg::NextPage)
        .id(iced::widget::Id::new("reader.page.next"))
        .aria_label("Next page")
        .tooltip("Next page")
        .enabled(current < count)
        .into_element();
    let field = container(
        enz_text_field_owned("", "", entry, Msg::PageEntryEdited)
            .size(TextFieldSize::Sm)
            .aria_label("Page number")
            .on_submit(Msg::PageEntrySubmitted)
            .id(iced::widget::Id::new("reader-page-entry"))
            .into_element(),
    )
    .width(Length::Fixed(56.0));
    enz_glass(
        container(
            iced::widget::row![
                first,
                previous,
                field,
                enz_text(format!("of {count}")).size(TextSize::Footnote),
                next,
                last,
            ]
            .spacing(spacing.xs())
            .align_y(iced::Alignment::Center),
        )
        .padding([spacing.xs() / 2.0, spacing.xs()]),
    )
    .style(enzui_iced_theme::MaterialStyle::FlatFrosted)
    .tint_opacity(0.18)
    .ink(enzui_iced_theme::GlassInk::Adaptive)
    .radius(crate::view::shape_radius(theme, theme.shape.popover))
    .elevation(3)
    .into()
}

pub(crate) fn reflow_scroll_offset(model: &Model, theme: &EnzTheme, width: f32) -> Option<f32> {
    let old_width = model.pages_viewport_width;
    if old_width <= 0.0
        || (width - old_width).abs() < 0.5
        || model.pending_position_restore.is_some()
    {
        return None;
    }
    let page = page_at_scroll_offset(model, theme, old_width, model.pages_scroll_offset, 0.0);
    let row_height = |width| {
        let row = page_row_of(model, width, page, model.page_extents.len());
        page_row_heights(model, theme, width)
            .get(row)
            .copied()
            .unwrap_or(1.0)
            .max(1.0)
    };
    let fraction = (model.pages_scroll_offset - page_scroll_offset(model, theme, old_width, page))
        / row_height(old_width);
    Some((page_scroll_offset(model, theme, width, page) + fraction * row_height(width)).max(0.0))
}

// Touchpad pixels that count as one wheel notch when turning pages.
const WHEEL_PIXELS_PER_LINE: f32 = 40.0;

/// The top margin inside a document reading pane.
pub fn page_top_inset(spacing: &enzui_iced_theme::SpacingScale) -> f32 {
    spacing.sm()
}
