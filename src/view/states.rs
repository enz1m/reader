//! Loading, empty and unsupported-format states for reading panels.

use crate::model::{Format, Model, Screen};

/// The titlebar's text when no document is open.
pub const NO_FILE_OPEN: &str = "No file open";

/// The generic short loading title (thumbnails panel heading).
pub const LOADING: &str = "Loading...";
/// The full-surface loading line, while a document is being parsed/rendered.
pub const LOADING_DOCUMENT: &str = "Loading document...";
/// The thumbnails panel's loading description.
pub const RENDERING_THUMBNAILS: &str = "Rendering thumbnails...";

/// Outline panel, nothing open.
pub const OPEN_FOR_OUTLINE: &str = "Open a PDF to see its outline.";
/// Bookmarks panel, nothing open.
pub const OPEN_FOR_BOOKMARKS: &str = "Open a PDF to start bookmarking.";
/// Footnotes panel, nothing open.
pub const OPEN_FOR_FOOTNOTES: &str = "Open an EPUB to see its footnotes.";
pub const EPUB_FOOTNOTES_TITLE: &str = "EPUB footnotes";
pub const EPUB_FOOTNOTES_BODY: &str = "Linked footnotes are available for EPUB books.";
/// Thumbnails panel, nothing open.
pub const OPEN_FOR_THUMBNAILS: &str = "Open a PDF to see page thumbnails.";

/// Footnotes panel title, EPUB open with no notes.
pub const NO_FOOTNOTES_TITLE: &str = "No Footnotes";
/// Footnotes panel description for the same state.
pub const NO_FOOTNOTES_BODY: &str = "This book has no footnotes or endnotes.";
/// Outline panel, document open with no TOC.
pub const NO_TABLE_OF_CONTENTS: &str = "This document has no table of contents.";
pub const OUTLINE_UNAVAILABLE: &str = "The PDF outline could not be read safely.";
/// Outline panel title for the same state.
pub const NO_OUTLINE_TITLE: &str = "No Outline";
/// The shared "there is no document" panel title.
pub const NO_DOCUMENT_TITLE: &str = "No Document";
/// Bookmarks panel title, document open with no bookmarks.
pub const NO_BOOKMARKS_TITLE: &str = "No Bookmarks";

/// Encrypted-PDF flow.
pub const PASSWORD_REQUIRED: &str = "Password Required";
pub const PASSWORD_PLACEHOLDER: &str = "Enter password…";
pub const PASSWORD_LABEL: &str = "Password";
pub const UNLOCK_LABEL: &str = "Unlock";
pub const PASSWORD_MESSAGE: &str = "This PDF is password-protected. Enter the password to open it.";
pub const PASSWORD_WRONG: &str = "Wrong password. Try again.";
/// Shown while the off-thread unlock rasterises pages, which can take a second.
pub const PASSWORD_UNLOCKING: &str = "Unlocking…";

/// Row actions.
pub const REMOVE_BOOKMARK: &str = "Remove bookmark";
/// The titlebar button that saves page edits made outside the arranger.
pub const SAVE_PAGE_CHANGES: &str = "Save page changes";
pub const ADD_BOOKMARK: &str = "Add bookmark";
pub const REMOVE_FROM_RECENTS: &str = "Remove from recents";

/// One panel's nothing-state: a title plus a description.
///
/// A title and description pair, since each renders as
/// `enz_status_page(title).description(body)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyState {
    pub title: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
}

impl EmptyState {
    const fn new(title: &'static str, description: &'static str, icon: &'static str) -> Self {
        EmptyState {
            title,
            description,
            icon,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateFamily {
    /// Something is in flight; the user should wait.
    Loading,
    /// Nothing is open; the user should open something.
    NothingOpen,
    /// A document is open and genuinely has none of this.
    GenuinelyEmpty,
    /// The open document's format does not support this panel.
    UnsupportedFormat,
}

/// The family a resolved [`EmptyState`] belongs to, decided by which of the
/// named constants it was built from.
///
/// Derived from identity rather than a stored tag, so classification and copy cannot
/// disagree.
pub fn family(state: EmptyState) -> StateFamily {
    if state.title == EPUB_FOOTNOTES_TITLE {
        return StateFamily::UnsupportedFormat;
    }
    if state.title == LOADING
        || state.description == RENDERING_THUMBNAILS
        || state.description == LOADING_DOCUMENT
    {
        return StateFamily::Loading;
    }
    if state.description == OPEN_FOR_OUTLINE
        || state.description == OPEN_FOR_BOOKMARKS
        || state.description == OPEN_FOR_FOOTNOTES
        || state.description == OPEN_FOR_THUMBNAILS
    {
        return StateFamily::NothingOpen;
    }
    StateFamily::GenuinelyEmpty
}

// Whether a document is open on screen; the reading screen is reachable while `loading`
// is still true.
fn has_document(model: &Model) -> bool {
    model.screen == Screen::Reading && model.file_path.is_some()
}

/// The outline (table of contents) panel's state, or `None` when there are
/// entries to render.
///
/// Precedence: loading → nothing open → genuinely empty.
pub fn outline_state(model: &Model) -> Option<EmptyState> {
    if model.loading || (model.format == Format::Pdf && model.pdf_navigation_loading) {
        return Some(EmptyState::new(LOADING, LOADING_DOCUMENT, "loader"));
    }
    if !has_document(model) {
        return Some(EmptyState::new(
            NO_DOCUMENT_TITLE,
            OPEN_FOR_OUTLINE,
            "file-text",
        ));
    }
    if model.format == Format::Pdf && model.pdf_outline_error.is_some() {
        return Some(EmptyState::new(
            "Outline unavailable",
            OUTLINE_UNAVAILABLE,
            "triangle-alert",
        ));
    }
    if outline_len(model) == 0 {
        return Some(EmptyState::new(
            NO_OUTLINE_TITLE,
            NO_TABLE_OF_CONTENTS,
            "list",
        ));
    }
    None
}

fn outline_len(model: &Model) -> usize {
    match model.format {
        Format::Epub => model.epub.as_ref().map(|e| e.load.toc.len()).unwrap_or(0),
        Format::Pdf => model.pdf_outline.len(),
        Format::Cbz => 0,
    }
}

/// The thumbnails panel's state, or `None` when there are thumbnails.
pub fn thumbnails_state(model: &Model) -> Option<EmptyState> {
    if model.loading {
        return Some(EmptyState::new(LOADING, RENDERING_THUMBNAILS, "loader"));
    }
    if !has_document(model) {
        return Some(EmptyState::new(
            NO_DOCUMENT_TITLE,
            OPEN_FOR_THUMBNAILS,
            "grid-2x2",
        ));
    }
    // Page geometry, not resident rasters: eviction would otherwise make the panel read as
    // loading forever.
    if model.page_extents.is_empty() {
        return Some(EmptyState::new(LOADING, RENDERING_THUMBNAILS, "loader"));
    }
    None
}

/// The bookmarks panel's state, or `None` when there are bookmarks.
///
/// The genuinely-empty description quotes the dispatcher's chord (see
/// [`bookmarks_hint`]).
pub fn bookmarks_state(model: &Model) -> Option<(&'static str, String)> {
    if model.loading {
        return Some((LOADING, LOADING_DOCUMENT.to_string()));
    }
    if model.annotations_loading {
        return Some((
            LOADING,
            super::annotations_panel::LOADING_ANNOTATIONS.to_string(),
        ));
    }
    if model.annotations_unavailable {
        return Some((
            "Annotations unavailable",
            super::annotations_panel::ANNOTATIONS_UNAVAILABLE.to_string(),
        ));
    }
    if !has_document(model) {
        return Some((NO_BOOKMARKS_TITLE, OPEN_FOR_BOOKMARKS.to_string()));
    }
    if model.annotations.bookmarks.is_empty() {
        return Some((NO_BOOKMARKS_TITLE, bookmarks_hint()));
    }
    None
}

/// `Ctrl+D to bookmark the current page.`, with the chord read from
/// [`crate::shortcuts::bindings`], the table `App::update` dispatches from.
pub fn bookmarks_hint() -> String {
    let chord = crate::shortcuts::chord_display(crate::shortcuts::Command::ToggleBookmark);
    if chord.is_empty() {
        // No binding: say what to do without naming a key that does not exist.
        return "Bookmark the current page from the toolbar.".to_string();
    }
    format!("{chord} to bookmark the current page.")
}

/// The footnotes panel's state, or `None` when there are notes to render.
pub fn footnotes_state(model: &Model) -> Option<EmptyState> {
    if model.loading {
        return Some(EmptyState::new(LOADING, LOADING_DOCUMENT, "loader"));
    }
    if !has_document(model) {
        return Some(EmptyState::new(
            NO_DOCUMENT_TITLE,
            OPEN_FOR_FOOTNOTES,
            "hash",
        ));
    }
    if model.format != Format::Epub {
        return Some(EmptyState::new(
            EPUB_FOOTNOTES_TITLE,
            EPUB_FOOTNOTES_BODY,
            "hash",
        ));
    }
    let notes = model
        .epub
        .as_ref()
        .map(|e| e.load.footnotes.len())
        .unwrap_or(0);
    if notes == 0 {
        return Some(EmptyState::new(
            NO_FOOTNOTES_TITLE,
            NO_FOOTNOTES_BODY,
            "hash",
        ));
    }
    None
}

/// The titlebar's document name: the open file's name, or [`NO_FILE_OPEN`].
///
/// The slot is never left empty.
pub fn document_name(model: &Model) -> String {
    if model.screen != Screen::Reading {
        return NO_FILE_OPEN.to_string();
    }
    // Prefer the resolved metadata title, as the recents card does.
    if !model.display_title.is_empty() {
        return model.display_title.clone();
    }
    if !model.file_name.is_empty() {
        return model.file_name.clone();
    }
    NO_FILE_OPEN.to_string()
}

/// `Resumed: page 5` / `Resumed: chapter 3` — the resume chip shown when a
/// book reopens past its first page.
///
/// `page` is 0-based internally and displayed 1-based, and the unit follows the format.
/// Page 0 returns `None`, since there is nothing to resume to.
pub fn resume_notice(format: Format, page: usize) -> Option<String> {
    if page == 0 {
        return None;
    }
    let unit = match format {
        Format::Epub => "chapter",
        Format::Pdf | Format::Cbz => "page",
    };
    Some(format!("Resumed: {unit} {}", page + 1))
}

/// `Chapter 3` — the fallback title for an EPUB chapter with no `<title>`.
///
/// 0-based index in, 1-based display out; the same substitution as
/// `plumbing::footnotes::group_for_sidebar`.
pub fn chapter_label(index: usize) -> String {
    format!("Chapter {}", index + 1)
}
