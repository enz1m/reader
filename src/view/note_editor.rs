//! Multiline note/comment drafts with explicit commit and cancellation.

use enzui_iced_core::form::{FieldId, FieldValue, FormField, FormModel};
use enzui_iced_core::semantics::HeadingLevel;
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::form_dialog::FormPresenter;
use enzui_iced_widgets::modal_surface::enz_modal_surface;
use enzui_iced_widgets::text_area::enz_text_area;
use iced::widget::{Space, column, row};
use iced::{Alignment, Element, Length};

use crate::message::Msg;
use crate::model::{Format, Model};

/// The heading when composing a brand-new note.
pub const NEW_NOTE_TITLE: &str = "New Note";
pub const NEW_PDF_COMMENT_TITLE: &str = "Add PDF Comment";
/// The heading when editing a comment stored in the PDF.
pub const EDIT_PDF_COMMENT_TITLE: &str = "Edit PDF Comment";
/// The heading when editing an existing one. Same string the annotations
/// panel's inline `Edit Note` button carries, re-exported so the two cannot
/// disagree.
pub use super::annotations_panel::EDIT_NOTE_LABEL as EDIT_NOTE_TITLE;
/// The field's label.
pub const NOTE_FIELD_LABEL: &str = "Note";
/// The field's PLACEHOLDER (see the module docs).
pub const NOTE_PLACEHOLDER: &str = "Type your note…";
pub const SAVE_LABEL: &str = "Save";
pub const CANCEL_LABEL: &str = "Cancel";
/// Removes the note being edited, after the usual confirmation.
pub const DELETE_LABEL: &str = "Delete";

/// Widget and session id of the composer's single field, shared by the view
/// and the reducer so both address the same editor.
pub const NOTE_FIELD_ID: &str = "reader-note";

/// Visible height of the note field, in text rows.
pub const NOTE_ROWS: usize = 4;

/// The [`FieldId`] of the note field.
#[must_use]
pub fn note_field_id() -> FieldId {
    FieldId::from(NOTE_FIELD_ID)
}

/// Build the composer's one-field presenter model, seeded with `initial`.
///
/// The field is not `required`; emptiness is handled by the disabled Save
/// button and the reducer, not a validation error.
#[must_use]
pub fn note_form_model(initial: &str) -> FormModel {
    FormModel::new(vec![
        FormField::multiline(NOTE_FIELD_ID, NOTE_FIELD_LABEL, NOTE_ROWS as u16)
            .placeholder(NOTE_PLACEHOLDER)
            .initial(FieldValue::Text(initial.to_string())),
    ])
    .expect("the composer's single field id is unique")
}

/// The composer's current draft, trimmed, or `None` when it is closed.
#[must_use]
pub fn committed_note(model: &Model) -> Option<String> {
    model.note_editor.as_ref().map(|presenter| {
        presenter
            .session()
            .value(&note_field_id())
            .and_then(FieldValue::as_text)
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

fn dialog_width(theme: &EnzTheme) -> f32 {
    theme.sizing.dialog_width_md
}

/// The composer's heading for `model`: `New Note` unless an existing
/// annotation is being edited.
///
/// A pure function so the branch is assertable.
pub fn title(model: &Model) -> &'static str {
    if model.pending_pdf_comment.is_some() {
        NEW_PDF_COMMENT_TITLE
    } else if model
        .note_editor_editing
        .is_some_and(|id| editing_pdf_comment(model, id))
    {
        EDIT_PDF_COMMENT_TITLE
    } else if model.note_editor_editing.is_some() {
        EDIT_NOTE_TITLE
    } else {
        NEW_NOTE_TITLE
    }
}

fn editing_pdf_comment(model: &Model, id: u64) -> bool {
    model
        .annotations
        .annotations
        .iter()
        .find(|annotation| annotation.id == id)
        .is_some_and(|annotation| annotation.pdf_binding.is_some())
}

/// Names the note's page or displayed EPUB chapter, including the passage when selected.
pub fn message(model: &Model) -> String {
    if model.pending_pdf_comment.is_some() {
        return format!(
            "Comment on selected text on page {}. It will be stored in the PDF and listed in Annotations.",
            model.note_editor_page + 1
        );
    }
    if model
        .note_editor_editing
        .is_some_and(|id| editing_pdf_comment(model, id))
    {
        return format!(
            "Edit the PDF comment on page {}. The PDF and annotation list will update together.",
            model.note_editor_page + 1
        );
    }
    let location = match model.format {
        Format::Epub => model
            .epub
            .as_ref()
            .and_then(|epub| epub.load.chapters.get(model.note_editor_page))
            .map(|chapter| chapter.display_heading())
            .filter(|heading| !heading.trim().is_empty())
            .map_or_else(
                || format!("chapter {}", model.note_editor_page + 1),
                |heading| format!("“{heading}”"),
            ),
        Format::Pdf | Format::Cbz => format!("page {}", model.note_editor_page + 1),
    };
    let prefix = if model.note_editor_position.is_some() {
        "Note on the selected passage in"
    } else {
        "Note on"
    };
    format!("{prefix} {location}. Reader saves your note with this document.")
}

/// Whether `Save` can be pressed. A blank draft has nothing to save, and a
/// Save that silently no-ops is a control the user can operate that changes
/// nothing.
pub fn can_save(model: &Model) -> bool {
    committed_note(model).is_some_and(|text| !text.is_empty())
}

/// Render the composer over its live `presenter`.
///
/// The caller presents this only while
/// [`Model::note_editor`](crate::model::Model::note_editor) is `Some`.
pub fn view<'a>(
    model: &'a Model,
    presenter: &'a FormPresenter,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    // The modal heading is the only accessible page heading while the presenting screen is
    // inert.
    let heading = enz_text(title(model))
        .size(TextSize::Title3)
        .heading(HeadingLevel::Page);
    let supporting = enz_text(message(model))
        .size(TextSize::Body)
        .tone(TextTone::Muted);

    // Plain Enter inserts a newline: `on_submit` (which fires on unshifted
    // Enter) is deliberately left unset.
    let content = presenter
        .bridge()
        .content(&note_field_id())
        .expect("the multiline note field always has editor content");
    let field = enz_text_area(content, Msg::NoteEditorAction)
        .label(NOTE_FIELD_LABEL)
        .placeholder(NOTE_PLACEHOLDER)
        .rows(NOTE_ROWS)
        .id(NOTE_FIELD_ID.to_owned())
        .into_element();

    // An existing note can be removed from where it is read and edited.
    let delete: Element<'a, Msg> = match model.note_editor_editing {
        Some(id) => enz_button(DELETE_LABEL, Msg::RemoveAnnotation(id))
            .id(iced::widget::Id::new("reader.note.delete"))
            .kind(ButtonKind::QuietDestructive)
            .size(ButtonSize::Sm)
            .into_element(),
        None => Space::new().into(),
    };
    let buttons = row![
        delete,
        Space::new().width(Length::Fill),
        enz_button(CANCEL_LABEL, Msg::CloseOverlay)
            .id(iced::widget::Id::new("reader.note.cancel"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        enz_button(SAVE_LABEL, Msg::CommitNoteEditor)
            .id(iced::widget::Id::new("reader.note.save"))
            .kind(ButtonKind::Accent)
            .size(ButtonSize::Sm)
            .enabled(can_save(model))
            .into_element(),
    ]
    .spacing(spacing.sm())
    .align_y(Alignment::Center);

    enz_modal_surface(
        column![heading, supporting, field, buttons]
            .spacing(spacing.md())
            .width(Length::Fill)
            .into(),
    )
    .width(Length::Fixed(dialog_width(theme)))
    .padding(spacing.lg())
    .into_element()
}
