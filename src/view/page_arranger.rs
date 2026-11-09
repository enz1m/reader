//! Page ordering, rotation and extraction controls.
//!
//! Merge sources provide read authority; Save As requires a separately granted destination.

use enzui_iced_core::action::Action;
use enzui_iced_core::semantics::HeadingLevel;
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::reorder_list::enz_reorder_list;
use enzui_iced_widgets::scroll_view::enz_scroll_view;
use enzui_iced_widgets::status_page::enz_status_page;
use enzui_iced_widgets::surface::{BorderStyle, LineSize, RadiusSize, SurfaceTone, enz_surface};
use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
use iced::widget::{column, container, row};
use iced::{Alignment, Element, Length, Padding};

use crate::message::Msg;
use crate::model::Model;
use crate::pages::{PageEntry, PageSource};

pub const TITLE: &str = "Page arranger";

pub const SUBTITLE: &str =
    "Page Arranger \u{2014} drag to reorder pages; select pages to rotate, extract or delete them";

pub const EXIT_LABEL: &str = "Exit page arranger";
pub const MOVE_UP_LABEL: &str = "Move up";
pub const MOVE_DOWN_LABEL: &str = "Move down";
/// Per-page rotate controls, used as each card's tooltips.
pub const ROTATE_LEFT_LABEL: &str = "Rotate left";
pub const ROTATE_RIGHT_LABEL: &str = "Rotate right";
pub const DELETE_PAGE_LABEL: &str = "Delete page";
pub const EXTRACT_LABEL: &str = "Extract this page";
pub const EXTRACT_SELECTED_LABEL: &str = "Extract selected pages\u{2026}";
pub const DELETE_SELECTED_LABEL: &str = "Delete selected pages\u{2026}";
pub const MERGE_ANOTHER_LABEL: &str = "Add pages from a PDF\u{2026}";
pub const OPEN_PAGE_LABEL: &str = "Open page";
pub const APPLY_LABEL: &str = "Apply to source";
pub const SAVE_AS_LABEL: &str = "Save As…";
/// Shown when the arranger is entered with nothing open; the rows carry page
/// thumbnails once a PDF is open.
pub const EMPTY_NO_DOCUMENT: &str = "Open a PDF to see page thumbnails.";

/// The row label for `entry`: its own stable label, not its current position, so a
/// move visibly changes the row.
pub fn row_label(entry: &PageEntry) -> &str {
    &entry.label
}

/// The per-row line describing this page's orientation.
///
/// Empty for an upright page; otherwise it shows the rotate button's effect in the row
/// that was pressed.
#[must_use]
pub fn row_rotation_text(entry: &PageEntry) -> String {
    if entry.rotation == crate::viewport::PageRotation::NONE {
        String::new()
    } else {
        format!("Rotated {}°", entry.rotation.degrees())
    }
}

/// Whether the `(Move up, Move down)` controls are enabled for the row at `index` of a
/// list of `total` rows; a function so the boundary rule is assertable.
pub fn move_controls_enabled(index: usize, total: usize) -> (bool, bool) {
    (index > 0 && index < total, index + 1 < total)
}

/// The secondary line: where the page came from.
pub fn row_source_text(entry: &PageEntry) -> String {
    match &entry.source {
        PageSource::Original(i) => format!("From this document, page {}", i + 1),
        PageSource::Merged { file, index, .. } => {
            let name = std::path::Path::new(file)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(file);
            format!("Merged from {name}, page {}", index + 1)
        }
    }
}

/// Persistent arrangement drift summary; transient acknowledgements use the status host.
pub fn status_line(model: &Model) -> String {
    if let Some(publication) = &model.arrangement_publication {
        return format!("Publishing to {}…", publication.destination.display());
    }
    if let Some(failure) = &model.arrangement_publish_failure {
        return failure.clone();
    }
    let original = model.total_page_count.max(model.page_count);
    if model.arrangement.is_identity(original) {
        format!("{} pages, unchanged", model.arrangement.len())
    } else {
        format!(
            "{} pages, rearranged from {original}",
            model.arrangement.len()
        )
    }
}

/// The narrowest a page tile gets before the grid drops a column.
pub const TILE_MIN_WIDTH: f32 = 136.0;
/// A tile's height: the thumbnail box, the label, and room for the rotation
/// line, so every row of the grid is the same height.
pub const TILE_HEIGHT: f32 = 222.0;
// The box a page thumbnail is fitted into, the size thumbnails render at.
const THUMB_BOX: (f32, f32) = (112.0, 158.0);
/// The selection bar's line when nothing is selected.
pub const SELECTION_HINT: &str =
    "Click a page to select it; Ctrl-click or Shift-click to select several.";

/// How many columns of tiles fit across `width`.
#[must_use]
pub fn grid_columns(width: f32, spacing: f32) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    (((width + spacing) / (TILE_MIN_WIDTH + spacing)).floor() as usize).max(1)
}

/// The width of the arranger's grid in a content area `content_width` wide.
#[must_use]
pub fn grid_width(content_width: f32, theme: &EnzTheme) -> f32 {
    (content_width - 2.0 * theme.spacing.lg()).max(TILE_MIN_WIDTH)
}

/// The arranger pages worth rendering thumbnails for: the rows in view and
/// one row either side.
#[must_use]
pub fn visible_entries(model: &Model, columns: usize, spacing: f32) -> std::ops::Range<usize> {
    let pitch = TILE_HEIGHT + spacing;
    let viewport = if model.arranger_viewport_height > 0.0 {
        model.arranger_viewport_height
    } else {
        900.0
    };
    let first_row = (model.arranger_scroll_offset / pitch).floor() as usize;
    let rows = (viewport / pitch).ceil() as usize + 2;
    let start = first_row.saturating_sub(1) * columns;
    let end = ((first_row + rows) * columns).min(model.arrangement.len());
    start.min(end)..end
}

pub fn view<'a>(model: &'a Model, theme: &'a EnzTheme, content_width: f32) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let compact = content_width < 520.0;
    let exit: Element<'a, Msg> = if compact {
        enz_toolbar_icon("x", Msg::ClosePageArranger)
            .id(iced::widget::Id::new("reader.arranger.exit"))
            .aria_label(EXIT_LABEL)
            .tooltip(EXIT_LABEL)
            .into_element()
    } else {
        enz_button(EXIT_LABEL, Msg::ClosePageArranger)
            .id(iced::widget::Id::new("reader.arranger.exit"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element()
    };
    let header = row![
        column![
            enz_text(TITLE)
                .size(TextSize::Title3)
                .heading(HeadingLevel::Page),
            enz_text(SUBTITLE)
                .size(TextSize::Caption)
                .tone(TextTone::Muted),
        ]
        .spacing(spacing.space1)
        .width(Length::Fill),
        exit,
    ]
    .spacing(spacing.sm())
    .align_y(Alignment::Center);

    // No pages and no editable document: the empty state. A document whose pages were
    // all deleted still shows the merge prompt.
    if model.arrangement.is_empty() && !model.page_editing_supported() {
        return container(
            column![
                header,
                enz_status_page(EMPTY_NO_DOCUMENT)
                    .icon("file-text")
                    .primary_action(
                        Action::new("reader.open", Msg::OpenDialog).with_label("Open File"),
                    )
                    .into_element(),
            ]
            .spacing(spacing.md()),
        )
        .padding(spacing.lg())
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    let publishing = model.arrangement_publication.is_some();
    let gap = spacing.md();
    let columns = grid_columns(grid_width(content_width, theme), gap);
    let tiles: Vec<(String, Element<'a, Msg>)> = model
        .arrangement
        .entries()
        .iter()
        .map(|entry| (entry.key(), page_tile(model, entry, theme)))
        .collect();

    let body: Element<'a, Msg> = if tiles.is_empty() {
        enz_status_page(EMPTY_NO_DOCUMENT)
            .icon("file-text")
            .primary_action(Action::new("reader.open", Msg::OpenDialog).with_label("Open File"))
            .into_element()
    } else {
        // Dragging publishes one stable-id drop for `PageArrangement::move_to_slot`; the
        // grid's selection feeds the selection bar and tile menus.
        enz_reorder_list(tiles, Msg::PageDropped)
            .grid(columns)
            .spacing(gap)
            .collection_id("reader-pages")
            .on_selection_change(Msg::ArrangerSelectionChanged)
            .into()
    };

    let merge = enz_button(MERGE_ANOTHER_LABEL, Msg::RequestMergePdf)
        .id(iced::widget::Id::new("reader.arranger.merge"))
        .kind(ButtonKind::Secondary)
        .icon("file-plus")
        .enabled(!publishing)
        .into_element();
    let save_actions = row![
        iced::widget::Space::new().width(Length::Fill),
        enz_button(SAVE_AS_LABEL, Msg::SavePageArrangementAs)
            .id(iced::widget::Id::new("reader.arranger.save-as"))
            .kind(ButtonKind::Secondary)
            .enabled(!publishing && !model.arrangement.is_empty())
            .into_element(),
        enz_button(APPLY_LABEL, Msg::ApplyPageArrangement)
            .id(iced::widget::Id::new("reader.arranger.apply"))
            .kind(ButtonKind::Accent)
            .enabled(
                !publishing
                    && !model
                        .arrangement
                        .is_identity(model.total_page_count.max(model.page_count)),
            )
            .into_element(),
    ]
    .spacing(spacing.xs())
    .align_y(Alignment::Center);
    let publication_actions: Element<'a, Msg> = if compact {
        column![merge, save_actions].spacing(spacing.sm()).into()
    } else {
        row![merge, save_actions]
            .spacing(spacing.xs())
            .align_y(Alignment::Center)
            .into()
    };
    let status = enz_text(status_line(model))
        .size(TextSize::Caption)
        .tone(TextTone::Muted);

    container(
        column![
            header,
            selection_bar(model, theme),
            status,
            enz_scroll_view(container(body).width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .on_scroll(|viewport| Msg::ArrangerScrolled {
                    offset: viewport.absolute_offset().y,
                    viewport_height: viewport.bounds().height,
                }),
            publication_actions,
        ]
        .spacing(spacing.md())
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(Padding {
        top: spacing.md(),
        right: spacing.lg(),
        bottom: spacing.md(),
        left: spacing.lg(),
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// What the selection bar says about `selected` pages.
#[must_use]
pub fn selection_text(selected: usize) -> String {
    match selected {
        0 => SELECTION_HINT.to_string(),
        1 => "1 page selected".to_string(),
        n => format!("{n} pages selected"),
    }
}

fn selection_bar<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    let publishing = model.arrangement_publication.is_some();
    let ids = &model.arranger_selection;
    let some = !ids.is_empty() && model.arrangement_publication.is_none();
    let all_original = ids.iter().all(|id| {
        model
            .arrangement
            .position(*id)
            .and_then(|index| model.arrangement.get(index))
            .is_some_and(|entry| matches!(entry.source, PageSource::Original(_)))
    });
    row![
        enz_text(selection_text(ids.len()))
            .size(TextSize::Caption)
            .tone(TextTone::Muted),
        iced::widget::Space::new().width(Length::Fill),
        selection_icon(
            "undo-2",
            "Undo",
            Msg::Undo,
            !publishing && !model.undo_stack.undo_empty()
        ),
        selection_icon(
            "redo-2",
            "Redo",
            Msg::Redo,
            !publishing && !model.undo_stack.redo_empty()
        ),
        selection_icon(
            "rotate-ccw",
            ROTATE_LEFT_LABEL,
            Msg::RotatePages {
                ids: ids.clone(),
                clockwise: false,
            },
            some,
        ),
        selection_icon(
            "rotate-cw",
            ROTATE_RIGHT_LABEL,
            Msg::RotatePages {
                ids: ids.clone(),
                clockwise: true,
            },
            some,
        ),
        selection_icon(
            "file-output",
            EXTRACT_SELECTED_LABEL,
            Msg::RequestExtractPages(ids.clone()),
            some && all_original,
        ),
        selection_icon(
            "trash-2",
            DELETE_SELECTED_LABEL,
            Msg::RequestDeletePages(ids.clone()),
            some && ids.len() < model.arrangement.len(),
        ),
    ]
    .spacing(spacing.xs())
    .align_y(Alignment::Center)
    .into()
}

fn selection_icon<'a>(
    icon: &'static str,
    label: &'static str,
    message: Msg,
    enabled: bool,
) -> Element<'a, Msg> {
    enz_toolbar_icon(icon, message)
        .id(enzui_iced_core::identity::derive_id(
            "reader.arranger.selection",
            label,
        ))
        .aria_label(label)
        .tooltip(label)
        .enabled(enabled)
        .into_element()
}

// One page tile: its thumbnail, turned as the page is, and its label. A
// selected tile carries the accent ring. A right-click opens the tile's
// menu, on the whole selection when the tile is part of it.
fn page_tile<'a>(model: &'a Model, entry: &'a PageEntry, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    let id = entry.id;
    let selected = model.arranger_selection.contains(&id);

    let picture: Element<'a, Msg> = match &entry.source {
        PageSource::Original(page) => match model.thumbnail_rasters.get(*page) {
            Some(thumbnail) => {
                let quarter = !entry.rotation.degrees().is_multiple_of(180);
                let (w, h) = (thumbnail.width as f32, thumbnail.height as f32);
                let (shown_w, shown_h) = if quarter { (h, w) } else { (w, h) };
                let scale = (THUMB_BOX.0 / shown_w).min(THUMB_BOX.1 / shown_h).min(1.0);
                iced::widget::image(thumbnail.handle.clone())
                    .width(Length::Fixed(shown_w * scale))
                    .height(Length::Fixed(shown_h * scale))
                    .rotation(iced::Rotation::Solid(iced::Radians(
                        entry.rotation.radians(),
                    )))
                    .into()
            }
            None => {
                use enzui_iced_core::async_image::{FailureReason, Generation, ImageState};
                use enzui_iced_widgets::async_image::{EnzAsyncImageFit, enz_async_image_view};
                let state: ImageState<iced::widget::image::Handle> =
                    if model.thumbnail_failed.contains(page) {
                        ImageState::Failed(FailureReason::Service("thumbnail unavailable".into()))
                    } else {
                        ImageState::Loading(Generation::FIRST)
                    };
                enz_async_image_view(&state)
                    .fit(EnzAsyncImageFit::Contain)
                    .alt_text(format!("{} thumbnail", row_label(entry)))
                    .width(Length::Fixed(THUMB_BOX.0))
                    .height(Length::Fixed(THUMB_BOX.1))
                    .into_element()
            }
        },
        PageSource::Merged { .. } => enz_status_page(row_source_text(entry))
            .icon("file-plus")
            .variant(enzui_iced_widgets::status_page::StatusPageVariant::Inline)
            .compact(true)
            .into_element(),
    };

    let mut caption =
        column![enz_text(row_label(entry)).size(TextSize::Caption)].align_x(Alignment::Center);
    // The rotation shows on this tile, so turning the wrong page is distinguishable.
    let rotation_text = row_rotation_text(entry);
    if !rotation_text.is_empty() {
        caption = caption.push(
            enz_text(rotation_text)
                .size(TextSize::Caption)
                .tone(TextTone::Muted),
        );
    }

    let body = column![
        container(picture)
            .center_x(Length::Fixed(THUMB_BOX.0))
            .center_y(Length::Fixed(THUMB_BOX.1)),
        caption,
    ]
    .spacing(spacing.xs())
    .align_x(Alignment::Center)
    .width(Length::Fill);

    let tile = enz_surface(body)
        .tone(if selected {
            SurfaceTone::Selected
        } else {
            SurfaceTone::Surface
        })
        .border_style(if selected {
            BorderStyle::Accent
        } else {
            BorderStyle::Thin
        })
        .line_size(if selected {
            LineSize::Thick
        } else {
            LineSize::Thin
        })
        .radius_size(RadiusSize::Md)
        .padding(spacing.sm())
        .width(Length::Fill)
        .height(Length::Fixed(TILE_HEIGHT))
        .into_element();

    let pages = if selected && model.arranger_selection.len() > 1 {
        model.arranger_selection.clone()
    } else {
        vec![id]
    };
    enzui_iced_widgets::context_target::enz_context_target(tile, move |at| {
        Msg::OpenContextMenu(
            crate::context_menu::ContextTarget::ArrangerPage {
                id,
                pages: pages.clone(),
            },
            at,
        )
    })
    .width(Length::Fill)
    .height(Length::Fixed(TILE_HEIGHT))
    .into_element()
}
