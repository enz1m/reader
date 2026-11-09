//! Captured context targets with live capability checks before typed dispatch.

use enzui_iced_core::context_menu::{InvocationKind, Point as CorePoint};
use enzui_iced_widgets::context_menu_host::FocusRestoreTarget;
use enzui_iced_widgets::context_menu_host::{ContextMenu, ContextMenuItem};
use iced::Point;

use crate::message::Msg;
use crate::model::Model;

/// The id namespace a library card's pressable derives its widget id under, shared by
/// the view and [`invoker_id`]; a mismatch would silently skip focus restoration.
pub const LIBRARY_CARD_ID_PREFIX: &str = "reader-library-card";

/// The id namespace of an annotation row's `Edit Note` button, the row's first focusable
/// control and so the focus target when its menu closes.
pub const ANNOTATION_ROW_ID_PREFIX: &str = "reader-annotation-row";

/// What was right-clicked.
///
/// Keyed by the subject's model key, not a list position: both lists filter or sort
/// live, so a captured position can name another row a frame later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextTarget {
    /// A library or recents card, keyed by the book's path as `Msg::RemoveFromRecents`
    /// and `AppState` are.
    LibraryBook(String),
    /// An annotation row in the sidebar panel, keyed by annotation id.
    Annotation(u64),
    /// A page of the open PDF or comic, by its index in the document.
    Page(usize),
    /// A page tile in the arranger, by entry id, with the pages the menu acts on: the
    /// selection when the tile is part of one, else the tile, fixed when the menu opens.
    ArrangerPage { id: u64, pages: Vec<u64> },
}

/// One row a context menu can offer.
///
/// Every variant maps to an action with a visible control elsewhere; the menu adds a
/// route at the cursor but no right-click-only capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextAction {
    /// Open the right-clicked book.
    OpenBook,
    /// Drop the right-clicked book from the shelf.
    RemoveFromLibrary,
    /// Begin the inline edit on the right-clicked annotation.
    EditNote,
    /// Advance the right-clicked highlight through the colour palette.
    ChangeHighlightColor,
    /// Copy the right-clicked annotation as a one-line citation.
    CopyCitation,
    /// Seed the find panel from the right-clicked annotation's text.
    FindSimilar,
    /// Delete the right-clicked annotation.
    DeleteAnnotation,
    /// Bookmark the right-clicked page (the toolbar's bookmark button).
    AddBookmark,
    /// Remove the right-clicked page's bookmark.
    RemoveBookmark,
    /// Turn the right-clicked page, as its card in the arranger does.
    RotatePageLeft,
    RotatePageRight,
    /// Write the right-clicked page to a file of its own, asking first.
    ExtractPage,
    /// Open the page arranger.
    ArrangePages,
    /// Leave the arranger at the right-clicked page.
    OpenArrangerPage,
    /// Move the right-clicked arranger page one place earlier or later.
    MovePageEarlier,
    MovePageLater,
    /// Delete the right-clicked arranger pages, asking first.
    DeletePages,
}

impl ContextAction {
    /// The visible row label.
    ///
    /// Read from the in-panel buttons' `pub const`s where one exists
    /// (`annotations_panel::EDIT_NOTE_LABEL` etc.), so menu and button agree.
    pub fn label(self) -> String {
        use crate::view::annotations_panel as panel;
        match self {
            Self::OpenBook => "Open".to_string(),
            Self::RemoveFromLibrary => "Remove from library".to_string(),
            Self::EditNote => panel::EDIT_NOTE_LABEL.to_string(),
            Self::ChangeHighlightColor => panel::CHANGE_COLOR_LABEL.to_string(),
            Self::CopyCitation => panel::COPY_CITATION_LABEL.to_string(),
            Self::FindSimilar => panel::find_similar_label().to_string(),
            Self::DeleteAnnotation => panel::DELETE_LABEL.to_string(),
            Self::AddBookmark => "Add Bookmark".to_string(),
            Self::RemoveBookmark => "Remove Bookmark".to_string(),
            Self::RotatePageLeft => "Rotate Left".to_string(),
            Self::RotatePageRight => "Rotate Right".to_string(),
            Self::ExtractPage => "Extract Page\u{2026}".to_string(),
            Self::ArrangePages => "Arrange Pages\u{2026}".to_string(),
            Self::OpenArrangerPage => crate::view::page_arranger::OPEN_PAGE_LABEL.to_string(),
            Self::MovePageEarlier => crate::view::page_arranger::MOVE_UP_LABEL.to_string(),
            Self::MovePageLater => crate::view::page_arranger::MOVE_DOWN_LABEL.to_string(),
            Self::DeletePages => crate::view::page_arranger::DELETE_PAGE_LABEL.to_string(),
        }
    }

    /// The leading icon name.
    pub fn icon(self) -> &'static str {
        match self {
            Self::OpenBook => "book-open",
            Self::RemoveFromLibrary | Self::DeleteAnnotation => "trash-2",
            Self::EditNote => "edit",
            Self::ChangeHighlightColor => "palette",
            Self::CopyCitation => "copy",
            Self::FindSimilar => "search",
            Self::AddBookmark => "bookmark-plus",
            Self::RemoveBookmark => "bookmark-minus",
            Self::RotatePageLeft => "rotate-ccw",
            Self::RotatePageRight => "rotate-cw",
            Self::ExtractPage => "file-output",
            Self::ArrangePages => "layout-grid",
            Self::OpenArrangerPage => "book-open",
            Self::MovePageEarlier => "arrow-left",
            Self::MovePageLater => "arrow-right",
            Self::DeletePages => "trash-2",
        }
    }

    /// The separator group; a rule is drawn wherever it changes, so dropped rows leave no
    /// dangling rule.
    pub fn group(self) -> u8 {
        match self {
            Self::OpenBook
            | Self::EditNote
            | Self::ChangeHighlightColor
            | Self::CopyCitation
            | Self::FindSimilar
            | Self::AddBookmark
            | Self::RemoveBookmark => 0,
            Self::RemoveFromLibrary | Self::DeleteAnnotation => 1,
            Self::RotatePageLeft | Self::RotatePageRight | Self::ExtractPage => 2,
            Self::ArrangePages => 3,
            Self::OpenArrangerPage | Self::MovePageEarlier | Self::MovePageLater => 0,
            Self::DeletePages => 1,
        }
    }

    /// Whether the row takes the destructive tone.
    pub fn is_destructive(self) -> bool {
        matches!(
            self,
            Self::RemoveFromLibrary | Self::DeleteAnnotation | Self::DeletePages
        )
    }
}

/// The rows `target` offers, against the model as it is now.
///
/// Guards mirror the in-page controls: a missing file offers no `Open`, and a target the
/// model no longer has offers nothing, so a stale menu cannot act.
pub fn items_for(target: &ContextTarget, model: &Model) -> Vec<ContextAction> {
    match target {
        ContextTarget::LibraryBook(path) => {
            let Some(book) = book_for(path, model) else {
                return Vec::new();
            };
            let mut rows = Vec::new();
            if !book.file_missing {
                rows.push(ContextAction::OpenBook);
            }
            rows.push(ContextAction::RemoveFromLibrary);
            rows
        }
        ContextTarget::Annotation(id) => {
            let Some(annotation) = model
                .annotations
                .annotations
                .iter()
                .find(|annotation| annotation.id == *id)
            else {
                return Vec::new();
            };
            let editable = annotation.pdf_binding.is_none()
                || model.pdf_annotation_states.get(id).is_some_and(|state| {
                    !matches!(
                        state,
                        crate::model::PdfAnnotationState::Orphaned
                            | crate::model::PdfAnnotationState::Conflict
                    )
                });
            let mut rows = Vec::new();
            if editable {
                rows.push(ContextAction::EditNote);
            }
            if editable
                && model.format == crate::model::Format::Pdf
                && annotation.kind == "highlight"
                && annotation.pdf_binding.is_some()
            {
                rows.push(ContextAction::ChangeHighlightColor);
            }
            rows.extend([
                ContextAction::CopyCitation,
                ContextAction::FindSimilar,
                ContextAction::DeleteAnnotation,
            ]);
            rows
        }
        ContextTarget::Page(page) => page_items(*page, model),
        ContextTarget::ArrangerPage { id, pages } => arranger_items(*id, pages, model),
    }
}

// An arranger tile's rows. On one page: open it, move it, turn it,
// extract or delete it. On several selected pages: the operations that
// make sense for a set, which act on all of them.
fn arranger_items(id: u64, pages: &[u64], model: &Model) -> Vec<ContextAction> {
    let arrangement = &model.arrangement;
    let Some(index) = arrangement.position(id) else {
        return Vec::new();
    };
    if model.arrangement_publication.is_some() {
        return Vec::new();
    }
    let total = arrangement.len();
    let mut rows = Vec::new();
    if pages.len() <= 1 {
        rows.push(ContextAction::OpenArrangerPage);
        let (earlier, later) = crate::view::page_arranger::move_controls_enabled(index, total);
        if earlier {
            rows.push(ContextAction::MovePageEarlier);
        }
        if later {
            rows.push(ContextAction::MovePageLater);
        }
    }
    rows.extend([
        ContextAction::RotatePageLeft,
        ContextAction::RotatePageRight,
    ]);
    let all_original = pages.iter().all(|page| {
        arrangement
            .position(*page)
            .and_then(|index| arrangement.get(index))
            .is_some_and(|entry| matches!(entry.source, crate::pages::PageSource::Original(_)))
    });
    if all_original {
        rows.push(ContextAction::ExtractPage);
    }
    if pages.len() < total {
        rows.push(ContextAction::DeletePages);
    }
    rows
}

// A row's label: an arranger menu on several pages says how many.
fn label_for(target: &ContextTarget, action: ContextAction) -> String {
    match (target, action) {
        (ContextTarget::ArrangerPage { pages, .. }, ContextAction::ExtractPage)
            if pages.len() > 1 =>
        {
            format!("Extract {} Pages\u{2026}", pages.len())
        }
        (ContextTarget::ArrangerPage { pages, .. }, ContextAction::DeletePages)
            if pages.len() > 1 =>
        {
            format!("Delete {} Pages\u{2026}", pages.len())
        }
        (ContextTarget::ArrangerPage { .. }, ContextAction::ExtractPage) => {
            crate::view::page_arranger::EXTRACT_LABEL.to_string()
        }
        _ => action.label(),
    }
}

// A page's rows: its bookmark, then the page operations the arranger
// offers on the page's card, which edit the same arrangement.
fn page_items(page: usize, model: &Model) -> Vec<ContextAction> {
    use crate::model::Format;
    if !matches!(model.format, Format::Pdf | Format::Cbz) || page >= model.page_extents.len() {
        return Vec::new();
    }
    let mut rows = vec![
        if crate::plumbing::persistence::is_bookmarked(&model.annotations, page) {
            ContextAction::RemoveBookmark
        } else {
            ContextAction::AddBookmark
        },
    ];
    if model.page_editing_supported() {
        if page_entry(page, model).is_some() {
            rows.extend([
                ContextAction::RotatePageLeft,
                ContextAction::RotatePageRight,
                ContextAction::ExtractPage,
            ]);
        }
        rows.push(ContextAction::ArrangePages);
    }
    rows
}

// The arranger entry that shows the document's page `page`.
fn page_entry(page: usize, model: &Model) -> Option<u64> {
    model
        .arrangement
        .entries()
        .iter()
        .find(|entry| entry.source == crate::pages::PageSource::Original(page))
        .map(|entry| entry.id)
}

// The book `path` names, from the same shelf both the library screen and the
// welcome screen read.
fn book_for<'a>(
    path: &str,
    model: &'a Model,
) -> Option<&'a crate::plumbing::persistence::RecentBook> {
    model
        .library
        .iter()
        .chain(model.recents.iter())
        .find(|b| b.path == path)
}

// Whether `id` still names a live annotation.
fn has_annotation(id: u64, model: &Model) -> bool {
    model.annotations.annotations.iter().any(|a| a.id == id)
}

/// Typed commands offered when a menu opens; dispatch rechecks live capabilities.
pub fn menu_items(target: &ContextTarget, model: &Model) -> Vec<ContextMenuItem<ContextAction>> {
    let mut entries = Vec::new();
    let mut previous_group = None;
    for action in items_for(target, model) {
        if previous_group.is_some_and(|group| group != action.group()) {
            entries.push(ContextMenuItem::separator());
        }
        previous_group = Some(action.group());
        entries.push(
            ContextMenuItem::command(action, label_for(target, action))
                .icon(action.icon())
                .destructive(action.is_destructive()),
        );
    }
    entries
}

/// The message an activated row fires against the right-clicked target.
///
/// No arm reads `model.editing_annotation`, `model.file_path` or another current pointer:
/// a menu on annotation 7 deletes annotation 7 while annotation 3 is being edited.
pub fn message_for(target: &ContextTarget, action: ContextAction, model: &Model) -> Option<Msg> {
    match (target, action) {
        (ContextTarget::LibraryBook(path), ContextAction::OpenBook) => {
            let book = book_for(path, model)?;
            if book.file_missing {
                return None;
            }
            Some(Msg::OpenPath(std::path::PathBuf::from(path)))
        }
        (ContextTarget::LibraryBook(path), ContextAction::RemoveFromLibrary) => {
            book_for(path, model)?;
            Some(Msg::RemoveFromRecents(path.clone()))
        }
        (ContextTarget::Annotation(id), action) => {
            if !has_annotation(*id, model) {
                return None;
            }
            match action {
                // A note opens the dedicated multiline composer; other kinds
                // keep the inline field.
                ContextAction::EditNote => {
                    let uses_composer = model
                        .annotations
                        .annotations
                        .iter()
                        .find(|a| a.id == *id)
                        .is_some_and(|a| a.kind == "note" || a.pdf_binding.is_some());
                    if uses_composer {
                        Some(Msg::OpenNoteEditorForEdit(*id))
                    } else {
                        Some(Msg::BeginEditAnnotation(*id))
                    }
                }
                ContextAction::ChangeHighlightColor => Some(Msg::CyclePdfAnnotationColor(*id)),
                ContextAction::CopyCitation => Some(Msg::CopyCitation(*id)),
                ContextAction::FindSimilar => Some(Msg::FindSimilar(*id)),
                ContextAction::DeleteAnnotation => Some(Msg::RemoveAnnotation(*id)),
                _ => None,
            }
        }
        (ContextTarget::Page(page), action) => {
            if !page_items(*page, model).contains(&action) {
                return None;
            }
            match action {
                ContextAction::AddBookmark | ContextAction::RemoveBookmark => {
                    Some(Msg::ToggleBookmark(*page))
                }
                ContextAction::RotatePageLeft => {
                    Some(Msg::RotatePageLeft(page_entry(*page, model)?))
                }
                ContextAction::RotatePageRight => {
                    Some(Msg::RotatePageRight(page_entry(*page, model)?))
                }
                ContextAction::ExtractPage => {
                    Some(Msg::RequestExtractPage(page_entry(*page, model)?))
                }
                ContextAction::ArrangePages => Some(Msg::OpenPageArranger),
                _ => None,
            }
        }
        (ContextTarget::ArrangerPage { id, pages }, action) => {
            if !arranger_items(*id, pages, model).contains(&action) {
                return None;
            }
            let single = pages.len() <= 1;
            match action {
                ContextAction::OpenArrangerPage => Some(Msg::OpenPage(*id)),
                ContextAction::MovePageEarlier => Some(Msg::MovePageUp(*id)),
                ContextAction::MovePageLater => Some(Msg::MovePageDown(*id)),
                ContextAction::RotatePageLeft | ContextAction::RotatePageRight => {
                    Some(Msg::RotatePages {
                        ids: pages.clone(),
                        clockwise: action == ContextAction::RotatePageRight,
                    })
                }
                ContextAction::ExtractPage if single => Some(Msg::RequestExtractPage(*id)),
                ContextAction::ExtractPage => Some(Msg::RequestExtractPages(pages.clone())),
                ContextAction::DeletePages if single => Some(Msg::RequestDeletePage(*id)),
                ContextAction::DeletePages => Some(Msg::RequestDeletePages(pages.clone())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The menu's accessible name: the subject, not a bare "Actions".
pub fn menu_label(target: &ContextTarget, model: &Model) -> Option<String> {
    match target {
        ContextTarget::LibraryBook(path) => {
            book_for(path, model).map(|b| format!("{} actions", b.title))
        }
        ContextTarget::Annotation(id) => {
            has_annotation(*id, model).then(|| annotation_label(*id, model))
        }
        ContextTarget::Page(page) => {
            (*page < model.page_extents.len()).then(|| format!("Page {} actions", page + 1))
        }
        ContextTarget::ArrangerPage { id, pages } => {
            let index = model.arrangement.position(*id)?;
            let entry = model.arrangement.get(index)?;
            Some(if pages.len() > 1 {
                format!("{} selected pages actions", pages.len())
            } else {
                format!("{} actions", entry.label)
            })
        }
    }
}

/// The accessible name for annotation `id`'s actions.
///
/// Numbered by file order (`AnnotationsFile::annotations`), not the live filtered list.
/// The capture site and the menu share this derivation.
pub fn annotation_label(id: u64, model: &Model) -> String {
    let n = model
        .annotations
        .annotations
        .iter()
        .position(|a| a.id == id)
        .map(|i| i + 1)
        .unwrap_or(0);
    format!("Annotation {n} actions")
}

/// The widget id of the control focus returns to when `target`'s menu closes.
///
/// Must match the view's `.id(..)`. Pages and
/// arranger tiles have no focusable control of their own, so they return `None`.
pub fn invoker_id(target: &ContextTarget) -> Option<iced::advanced::widget::Id> {
    match target {
        ContextTarget::LibraryBook(path) => Some(library_card_id(path)),
        ContextTarget::Annotation(id) => Some(annotation_row_id(*id)),
        ContextTarget::Page(_) | ContextTarget::ArrangerPage { .. } => None,
    }
}

/// The widget id of a library card's pressable.
pub fn library_card_id(path: &str) -> iced::advanced::widget::Id {
    enzui_iced_core::identity::derive_id(LIBRARY_CARD_ID_PREFIX, path)
}

/// The widget id of an annotation row's `Edit Note` button.
pub fn annotation_row_id(id: u64) -> iced::advanced::widget::Id {
    enzui_iced_core::identity::derive_id(ANNOTATION_ROW_ID_PREFIX, &id.to_string())
}

/// The id a capture site advertises its routable `ShowContextMenu` node under.
///
/// The capture affordance must differ from both the invoker and the menu host.
pub fn context_target_id(invoker: &iced::advanced::widget::Id) -> iced::advanced::widget::Id {
    enzui_iced_core::identity::derive_id(&format!("{invoker:?}"), "context-invocation")
}

/// A showing-bound typed menu and its document authority.
#[derive(Debug)]
pub struct OpenContextMenu {
    pub menu: ContextMenu<ContextTarget, ContextAction>,
    pub generation: u64,
    pub document_generation: u64,
}

impl OpenContextMenu {
    /// Capture offered commands and document authority, or refuse an unavailable target.
    pub fn new(target: ContextTarget, at: Point, model: &Model, generation: u64) -> Option<Self> {
        let rows = menu_items(&target, model);
        if rows.is_empty() {
            return None;
        }
        let mut menu = ContextMenu::new();
        let invocation = InvocationKind::Pointer(CorePoint::new(at.x, at.y));
        match invoker_id(&target) {
            Some(invoker) => menu.open_restoring(
                target,
                invocation,
                rows,
                FocusRestoreTarget::widget(invoker),
            ),
            None => menu.open(target, invocation, rows),
        }
        Some(Self {
            menu,
            generation,
            document_generation: model.pdf_annotation_generation,
        })
    }
}
