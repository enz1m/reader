//! Annotation navigation, editing and export controls.

use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::expander::{ExpanderVariant, enz_expander};
use enzui_iced_widgets::scroll_view::enz_scroll_view;
use enzui_iced_widgets::sidebar_row::enz_sidebar_row;
use enzui_iced_widgets::text_field::enz_text_field;
use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
use iced::widget::{column, container, row};
use iced::{Alignment, Element, Length, Padding};

use crate::context_menu::ContextTarget;
use crate::message::Msg;
use crate::model::{Model, Screen};

pub const EMPTY_NO_DOCUMENT: &str = "Open a document to start adding notes.";
pub const LOADING_ANNOTATIONS: &str = "Loading annotations…";
pub const ANNOTATIONS_UNAVAILABLE: &str = "Annotations are unavailable until recovery completes.";
pub const EMPTY_NO_NOTES: &str =
    "No annotations yet. Select a passage to highlight it or add a note.";
pub const EMPTY_NO_BOOKMARKS: &str = "No Bookmarks";
pub const EMPTY_NO_FILTER_MATCH: &str = "No annotations match the filter.";
pub const ADD_NOTE_LABEL: &str = "Add note";
pub const FILTER_LABEL: &str = "Filter";

/// Narrows the annotations list, unlike document search's Find in document field.
pub const FILTER_PLACEHOLDER: &str = "Filter…";
pub const COPY_CITATION_LABEL: &str = "Copy with citation";
pub const DELETE_LABEL: &str = "Delete";
pub const EDIT_NOTE_LABEL: &str = "Edit Note";
pub const CHANGE_COLOR_LABEL: &str = "Change highlight color";
pub const SAVE_LABEL: &str = "Save";
pub const CANCEL_LABEL: &str = "Cancel";
pub const COPY_JSON_LABEL: &str = "Copy as JSON";
pub const COPY_MARKDOWN_LABEL: &str = "Copy as Markdown";
pub const UNDO_LABEL: &str = "Undo";
pub const REDO_LABEL: &str = "Redo";

/// The composer button's label, the control that opens the modal note composer
/// (`Overlay::NoteEditor`). It sits beside the one-line `Add note` field: the field takes
/// a quick line, the composer a long note. The ellipsis marks a control that opens
/// something.
pub const COMPOSE_NOTE_LABEL: &str = "Compose long note…";

/// The per-annotation `Find similar` button's label.
///
/// The row supplies the annotation id the
/// [`ViewAction::FindSimilar`](crate::actions::ViewAction::FindSimilar) reducer arm needs
/// to seed the find panel with that annotation's text.
pub fn find_similar_label() -> &'static str {
    crate::actions::ViewAction::FindSimilar.tooltip()
}

/// Resolve sidecar admission before ordinary document and filter empty states.
pub fn list_empty_state(model: &Model) -> Option<&'static str> {
    if model.screen != Screen::Reading {
        return Some(EMPTY_NO_DOCUMENT);
    }
    if model.annotations_loading {
        return Some(LOADING_ANNOTATIONS);
    }
    if model.annotations_unavailable {
        return Some(ANNOTATIONS_UNAVAILABLE);
    }
    if model.annotations.annotations.is_empty() {
        return Some(EMPTY_NO_NOTES);
    }
    if model.filtered_annotations().is_empty() {
        return Some(EMPTY_NO_FILTER_MATCH);
    }
    None
}

/// Whether the bookmark list shows [`EMPTY_NO_BOOKMARKS`]: an admitted document with no
/// bookmarks.
pub fn bookmarks_empty(model: &Model) -> bool {
    model.screen == Screen::Reading
        && !model.annotations_loading
        && !model.annotations_unavailable
        && model.annotations.bookmarks.is_empty()
}

/// Build the annotations panel body (everything below the tab switcher).
pub fn view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    // Admission must settle before controls can mutate the sidecar.
    if model.screen != Screen::Reading || model.annotations_loading || model.annotations_unavailable
    {
        return empty_state(list_empty_state(model).unwrap_or(EMPTY_NO_DOCUMENT), theme);
    }

    let mut body = column![]
        .spacing(spacing.sm())
        .width(Length::Fill)
        .padding(Padding {
            top: spacing.sm(),
            right: spacing.lg(),
            bottom: spacing.sm(),
            left: spacing.lg(),
        });

    // Content first: one primary control adds a note at the current location; a prose
    // selection offers the anchored Add note action instead.
    body = body.push(
        enz_button(ADD_NOTE_LABEL, Msg::OpenNoteEditor)
            .id(iced::widget::Id::new("reader.annotations.add"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
    );

    // Activating a row jumps to its persisted anchor; edit, cite, find and delete live in
    // the row's context actions.
    body = body.push(annotation_list(model, theme));
    body = body.push(bookmark_list(model, theme));

    let tools: Element<'a, Msg> = column![
        enz_text_field(
            FILTER_LABEL,
            FILTER_PLACEHOLDER,
            &model.annotation_filter,
            Msg::AnnotationFilterChanged,
        )
        .into_element(),
        undo_redo_row(model, theme),
        copy_row(theme),
    ]
    .spacing(spacing.sm())
    .into();
    body = body.push(
        enz_expander(
            "Annotation tools",
            model.annotation_tools_expanded,
            Msg::ToggleAnnotationTools,
        )
        .variant(ExpanderVariant::Plain)
        .content(tools)
        .into_element(),
    );

    enz_scroll_view(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn annotation_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    // `list_empty_state` decides precedence, so rendered copy and helper agree.
    if let Some(msg) = list_empty_state(model) {
        return empty_state(msg, theme);
    }
    let matches = model.filtered_annotations();

    let editing_id = model.editing_annotation.as_ref().map(|(id, _)| *id);

    let mut col = column![].spacing(spacing.xs()).width(Length::Fill);
    for ann in matches {
        let id = ann.id;
        // A format-appropriate location badge, stable across EPUB reflow.
        let mut meta = format!("{} · {}", kind_label(ann), location_label(model, ann));
        if !ann.color.is_empty() {
            meta.push_str(&format!(" ({})", ann.color));
        }
        if let Some(state) = model.pdf_annotation_states.get(&id) {
            meta.push_str(match state {
                crate::model::PdfAnnotationState::Verified if ann.pdf_binding.is_none() => {
                    " · Verified sidecar highlight"
                }
                crate::model::PdfAnnotationState::Verified => " · Native PDF annotation",
                crate::model::PdfAnnotationState::VerifiedAfterDocumentChange => {
                    " · PDF changed; page identity still verified"
                }
                crate::model::PdfAnnotationState::Unverified => {
                    " · Overlay withheld: text identity unverified"
                }
                crate::model::PdfAnnotationState::Orphaned => {
                    " · Orphaned after the PDF page changed; overlay withheld"
                }
                crate::model::PdfAnnotationState::Conflict => {
                    " · Conflict with the native PDF annotation; overlay withheld"
                }
            });
        }

        // The row being edited swaps its text line for an edit field with Save/Cancel.
        let row_body: Element<'a, Msg> = if editing_id == Some(id) {
            let draft = model
                .editing_annotation
                .as_ref()
                .map(|(_, d)| d.as_str())
                .unwrap_or("");
            let field = enz_text_field(EDIT_NOTE_LABEL, "Edit note text", draft, |t| {
                Msg::EditAnnotationChanged(t)
            })
            .on_submit(Msg::CommitEditAnnotation)
            .into_element();
            let edit_actions = row![
                enz_button(SAVE_LABEL, Msg::CommitEditAnnotation)
                    .id(iced::widget::Id::from(format!(
                        "reader.annotation.{id}.save"
                    )))
                    .kind(ButtonKind::Accent)
                    .size(ButtonSize::Sm)
                    .into_element(),
                enz_button(CANCEL_LABEL, Msg::CancelEditAnnotation)
                    .id(iced::widget::Id::from(format!(
                        "reader.annotation.{id}.cancel"
                    )))
                    .kind(ButtonKind::Ghost)
                    .size(ButtonSize::Sm)
                    .into_element(),
            ]
            .spacing(spacing.xs())
            .align_y(Alignment::Center);
            column![
                field,
                enz_text(meta).size(TextSize::Caption).tone(TextTone::Muted),
                edit_actions,
            ]
            .spacing(spacing.space1)
            .width(Length::Fill)
            .into()
        } else {
            let primary = if ann.text.trim().is_empty() {
                "(empty)".to_string()
            } else if ann.kind == "highlight" && !ann.note.trim().is_empty() {
                format!("{} — {}", ann.text, ann.note)
            } else {
                ann.text.clone()
            };
            enz_sidebar_row(primary, Msg::GoToAnnotation(id))
                .count(meta)
                .icon(if ann.kind == "highlight" {
                    "highlighter"
                } else {
                    "sticky-note"
                })
                .sizing(theme.sizing)
                .selected(model.selected_annotation == Some(id))
                .id(crate::context_menu::annotation_row_id(id))
                .on_context(move |at| Msg::OpenContextMenu(ContextTarget::Annotation(id), at))
                .context_label(crate::context_menu::annotation_label(id, model))
                .into_element()
        };

        col = col.push(row_body);
    }
    col.into()
}

fn bookmark_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    // The no-document case returned earlier in `view`, so this is the empty state.
    if bookmarks_empty(model) {
        return empty_state(EMPTY_NO_BOOKMARKS, theme);
    }

    let mut col = column![].spacing(spacing.xs()).width(Length::Fill);
    col = col.push(
        enz_text("Bookmarks")
            .size(TextSize::Caption)
            .tone(TextTone::Subtle),
    );
    for bm in &model.annotations.bookmarks {
        let page = bm.page;
        let label = if bm.label.trim().is_empty() {
            format!("Page {}", bm.page + 1)
        } else {
            bm.label.clone()
        };
        let rowel = row![
            container(enz_text(label).size(TextSize::Body)).width(Length::Fill),
            enz_toolbar_icon("x", Msg::RemoveBookmark(page))
                .id(iced::widget::Id::from(format!(
                    "reader.bookmark.{page}.remove"
                )))
                .aria_label(super::states::REMOVE_BOOKMARK)
                .tooltip(super::states::REMOVE_BOOKMARK)
                .into_element(),
        ]
        .spacing(spacing.xs())
        .align_y(Alignment::Center);
        col = col.push(container(rowel).width(Length::Fill).padding(spacing.space1));
    }
    col.into()
}

// Undo / Redo as the toolbar's icon buttons, then copying the annotations out as
// JSON or Markdown. Both history buttons dispatch unconditionally so the
// empty-stack message reaches the notice line; the enabled state mirrors the stack.
fn undo_redo_row<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    row![
        enz_toolbar_icon("undo-2", Msg::Undo)
            .id(iced::widget::Id::new("reader.annotations.undo"))
            .aria_label(UNDO_LABEL)
            .tooltip(UNDO_LABEL)
            .enabled(!model.undo_stack.undo_empty())
            .into_element(),
        enz_toolbar_icon("redo-2", Msg::Redo)
            .id(iced::widget::Id::new("reader.annotations.redo"))
            .aria_label(REDO_LABEL)
            .tooltip(REDO_LABEL)
            .enabled(!model.undo_stack.redo_empty())
            .into_element(),
    ]
    .spacing(spacing.xs())
    .align_y(Alignment::Center)
    .into()
}

fn copy_row<'a>(theme: &'a EnzTheme) -> Element<'a, Msg> {
    let _ = theme;
    column![
        enz_button(COPY_JSON_LABEL, Msg::CopyAnnotationsJson)
            .id(iced::widget::Id::new("reader.annotations.copy-json"))
            .kind(ButtonKind::Quiet)
            .size(ButtonSize::Sm)
            .icon("braces")
            .into_element(),
        enz_button(COPY_MARKDOWN_LABEL, Msg::CopyAnnotationsMarkdown)
            .id(iced::widget::Id::new("reader.annotations.copy-markdown"))
            .kind(ButtonKind::Quiet)
            .size(ButtonSize::Sm)
            .icon("file-text")
            .into_element(),
    ]
    .into()
}

fn empty_state<'a>(message: &'a str, theme: &'a EnzTheme) -> Element<'a, Msg> {
    container(enz_text(message).size(TextSize::Body).tone(TextTone::Muted))
        .padding(theme.spacing.lg())
        .width(Length::Fill)
        .into()
}

fn location_label(model: &Model, ann: &crate::plumbing::persistence::Annotation) -> String {
    match crate::plumbing::persistence::AnnotationPosition::parse(&ann.position, ann.page) {
        crate::plumbing::persistence::AnnotationPosition::EpubCfi { chapter, .. } => {
            format!("Chapter {}", chapter + 1)
        }
        crate::plumbing::persistence::AnnotationPosition::PdfRect { page, .. } => {
            format!("Page {}", page + 1)
        }
        crate::plumbing::persistence::AnnotationPosition::None => match model.format {
            crate::model::Format::Epub => format!("Chapter {}", ann.page + 1),
            crate::model::Format::Pdf | crate::model::Format::Cbz => {
                format!("Page {}", ann.page + 1)
            }
        },
    }
}

fn kind_label(ann: &crate::plumbing::persistence::Annotation) -> &'static str {
    if ann.kind == "highlight" {
        match ann.style.as_str() {
            "underline" => "Underline",
            "strikethrough" => "Strikethrough",
            "squiggly" => "Squiggly",
            _ => "Highlight",
        }
    } else {
        "Note"
    }
}
