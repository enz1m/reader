//! Content area, dialogs, and confirmations.

use super::*;

pub(crate) fn content_area<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    content_width: f32,
    window_height: f32,
) -> Element<'a, Msg> {
    let inner: Element<'a, Msg> = match model.screen {
        // The library is the home screen: every known document, filtered and
        // sorted, with a "Continue reading" section of recents at the top.
        Screen::Library => super::super::library::view(
            model,
            theme,
            content_width,
            (window_height - TITLEBAR_HEIGHT).max(240.0),
        ),
        // The page arranger is a MODE over the reading surface, not a floating
        // dialog. Its rows carry five
        // controls each and need the whole content width.
        Screen::Reading if model.overlay == Some(crate::model::Overlay::PageArranger) => {
            super::super::page_arranger::view(model, theme, content_width)
        }
        Screen::Reading => reading::view(model, theme, content_width, window_height),
    };
    let content = container(inner).width(Length::Fill).height(Length::Fill);
    enzui_iced_widgets::file_drop::enz_file_drop_target_with_receipt(content, Msg::FilesDropped)
        .enabled(!model.loading && model.overlay.is_none())
        .copy_only(true)
        .key(("reader-open", model.document_load_attempt))
        .into()
}

// The non-confirm dialog surface, if any, the shell's [`enz_modal_host`]
// presents. The confirm dialog is presented separately in [`view`] through
// the preset's own `into_host`.
pub(crate) fn dialog_surface<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
) -> Option<Element<'a, Msg>> {
    match model.overlay {
        Some(crate::model::Overlay::Metadata) => {
            Some(super::super::metadata_dialog::metadata_view(model, theme))
        }
        Some(crate::model::Overlay::Properties) => {
            Some(super::super::metadata_dialog::properties_view(model, theme))
        }
        Some(crate::model::Overlay::Password) => {
            Some(super::super::password_dialog::view(model, theme))
        }
        Some(crate::model::Overlay::NoteEditor) => model
            .note_editor
            .as_ref()
            .map(|presenter| super::super::note_editor::view(model, presenter, theme)),
        Some(crate::model::Overlay::Shortcuts) => Some(view_shortcuts_dialog()),
        Some(crate::model::Overlay::Sign) => model
            .signing
            .as_ref()
            .map(|draft| super::super::sign_dialog::view(draft, theme)),
        Some(crate::model::Overlay::PdfPassword) => {
            super::super::pdf_password_dialog::view(model, theme)
        }
        _ => None,
    }
}

fn view_shortcuts_dialog<'a>() -> Element<'a, Msg> {
    enz_shortcuts_dialog::<Msg>(true, crate::shortcuts::shortcut_dialog_entries())
        .title(crate::actions::translated_label("Keyboard Shortcuts"))
        .into_surface(Some(Box::new(|| Msg::CloseShortcuts)))
}

/// The confirm dialog's message body, naming the exact page.
///
/// A confirm that does not name what it destroys gets answered without reading.
pub fn confirm_message(model: &Model, action: &crate::model::PendingPageAction) -> String {
    use crate::model::PendingPageAction;
    let page_label = |id: u64| {
        model
            .arrangement
            .position(id)
            .and_then(|i| model.arrangement.get(i))
            .map(|e| e.label.clone())
            .unwrap_or_else(|| "this page".to_string())
    };
    match action {
        PendingPageAction::DeleteAnnotation(id) => {
            let label = model
                .annotations
                .annotations
                .iter()
                .find(|annotation| annotation.id == *id)
                .map(|annotation| {
                    let text = annotation.text.trim();
                    if text.is_empty() {
                        format!("annotation {}", annotation.id)
                    } else {
                        format!("“{}”", text.chars().take(80).collect::<String>())
                    }
                })
                .unwrap_or_else(|| "this annotation".to_string());
            format!("Delete {label}? You can undo this.")
        }
        PendingPageAction::DeletePage(id) => format!(
            "Remove {} from the arrangement? You can undo this.",
            page_label(*id)
        ),
        PendingPageAction::ExtractPage(id) => format!(
            "Write {} to a new PDF beside this one? The original is not changed.",
            page_label(*id)
        ),
        PendingPageAction::DeletePages(ids) => format!(
            "Remove {} pages from the arrangement? You can undo this.",
            ids.len()
        ),
        PendingPageAction::ExtractPages(ids) => format!(
            "Write these {} pages to one new PDF beside this one? The original is not changed.",
            ids.len()
        ),
        PendingPageAction::ClearMetadata => crate::actions::translated_label(
            "Remove every metadata field from this PDF? This rewrites the file.",
        ),
        PendingPageAction::RemoveMetadataDates(removal) => {
            crate::actions::translated_label(match (removal.creation, removal.modification) {
                (true, true) => "Remove the creation and modification dates from this PDF?",
                (true, false) => "Remove the creation date from this PDF?",
                (false, true) => "Remove the modification date from this PDF?",
                (false, false) => "No PDF dates are selected for removal.",
            })
        }
        // Names the COUNT, because "split this PDF" understates what happens
        // when the document has 300 pages and 300 files land beside it.
        PendingPageAction::SplitPdf => format!(
            "Write each of the {} pages as its own PDF beside this one? The original is not changed.",
            model.total_page_count.max(model.page_count)
        ),
    }
}
