//! Toolbar labels, action availability and destination view modes.

use crate::model::EpubViewMode;

/// Selection requirement shared by disabled controls and reducer refusals.
pub const SELECT_TEXT_FIRST: &str = "Select text on the page first";

/// Accessible action label including the selection requirement.
pub fn unavailable_label(action: ViewAction) -> String {
    format!("{} — {SELECT_TEXT_FIRST}", action.tooltip())
}

/// One toolbar/menu action, carrying both of its user-visible spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewAction {
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    /// The titlebar rotate control: `Rotate` announced, `Rotate Clockwise` in the menu,
    /// where no icon implies the direction.
    Rotate,
    /// The per-page anticlockwise control in the page arranger.
    RotateLeft,
    /// The per-page clockwise control in the page arranger, which rewrites one page in
    /// the document; [`Self::Rotate`] turns the whole view.
    RotateRight,
    /// Pages side by side in pairs, as a book's facing pages.
    TwoPages,
    SplitPdf,
    /// Switch to paginated mode (offered while scrolled).
    PaginatedMode,
    /// Switch to scrolled mode (offered while paginated).
    ScrolledMode,
    NextPage,
    PreviousPage,
    AddComment,
    PdfHighlight,
    FindSimilar,
}

impl ViewAction {
    /// The Title Case wording the overflow menu shows.
    pub fn label(self) -> &'static str {
        match self {
            ViewAction::ZoomIn => "Zoom In",
            ViewAction::ZoomOut => "Zoom Out",
            ViewAction::FitWidth => "Fit Width",
            ViewAction::FitPage => "Fit Page",
            ViewAction::Rotate => "Rotate Clockwise",
            ViewAction::RotateLeft => "Rotate Left",
            ViewAction::RotateRight => "Rotate Right",
            ViewAction::TwoPages => "Two Pages",
            ViewAction::SplitPdf => "Split PDF",
            ViewAction::PaginatedMode => "Paginated Mode",
            ViewAction::ScrolledMode => "Scrolled Mode",
            ViewAction::NextPage => "Next",
            ViewAction::PreviousPage => "Previous",
            ViewAction::AddComment => "Add Comment",
            ViewAction::PdfHighlight => "PDF Highlight",
            ViewAction::FindSimilar => "Find Similar",
        }
    }

    /// The sentence-case tooltip wording, also the accessible name of every control
    /// built from this row.
    pub fn tooltip(self) -> &'static str {
        match self {
            ViewAction::ZoomIn => "Zoom in",
            ViewAction::ZoomOut => "Zoom out",
            ViewAction::FitWidth => "Fit width",
            ViewAction::FitPage => "Fit page",
            ViewAction::Rotate => "Rotate",
            ViewAction::RotateLeft => "Rotate left",
            ViewAction::RotateRight => "Rotate right",
            ViewAction::TwoPages => "Show two pages side by side",
            ViewAction::SplitPdf => "Split into pages",
            ViewAction::PaginatedMode => "Switch to paginated mode",
            ViewAction::ScrolledMode => "Switch to scrolled mode",
            ViewAction::NextPage => "Next page",
            ViewAction::PreviousPage => "Previous page",
            ViewAction::AddComment => "Add comment on page",
            ViewAction::PdfHighlight => "Highlight selection in PDF",
            ViewAction::FindSimilar => "Find similar",
        }
    }

    /// The Lucide icon name.
    pub fn icon(self) -> &'static str {
        match self {
            ViewAction::ZoomIn => "plus",
            ViewAction::ZoomOut => "minus",
            ViewAction::FitWidth => "maximize-2",
            ViewAction::FitPage => "scan",
            ViewAction::Rotate | ViewAction::RotateRight => "rotate-cw",
            ViewAction::RotateLeft => "rotate-ccw",
            ViewAction::TwoPages => "columns-2",
            ViewAction::SplitPdf => "scissors",
            ViewAction::PaginatedMode => "book-open",
            ViewAction::ScrolledMode => "scroll-text",
            ViewAction::NextPage => "chevron-right",
            ViewAction::PreviousPage => "chevron-left",
            ViewAction::AddComment => "message-square",
            ViewAction::PdfHighlight => "highlighter",
            ViewAction::FindSimilar => "search",
        }
    }

    pub const ALL: &'static [ViewAction] = &[
        ViewAction::ZoomIn,
        ViewAction::ZoomOut,
        ViewAction::FitWidth,
        ViewAction::FitPage,
        ViewAction::Rotate,
        ViewAction::RotateLeft,
        ViewAction::RotateRight,
        ViewAction::TwoPages,
        ViewAction::SplitPdf,
        ViewAction::PaginatedMode,
        ViewAction::ScrolledMode,
        ViewAction::NextPage,
        ViewAction::PreviousPage,
        ViewAction::AddComment,
        ViewAction::PdfHighlight,
        ViewAction::FindSimilar,
    ];
}

/// The mode-switch action offered while reading in `mode`.
///
/// Names the mode being switched to: reading scrolled offers `Paginated Mode` /
/// `Switch to paginated mode`.
pub fn mode_action(mode: EpubViewMode) -> ViewAction {
    match mode {
        EpubViewMode::Scrolled => ViewAction::PaginatedMode,
        EpubViewMode::Paginated => ViewAction::ScrolledMode,
    }
}

/// The paginated-mode next page label, whose glyph stays out of the announced name
/// `Next page`.
pub const NEXT_PAGE_GLYPH_LABEL: &str = "Next ▶";
pub const PREV_PAGE_GLYPH_LABEL: &str = "◀ Prev";

pub(crate) fn translated_label(label: &str) -> String {
    let key = match label {
        "Open document" => "reader-open",
        "Browse library" => "reader-library",
        "Previous chapter" => "reader-previous-chapter",
        "Next chapter" => "reader-next-chapter",
        "Smaller text" => "reader-font-smaller",
        "Larger text" => "reader-font-larger",
        "Reading theme" => "reader-reading-theme",
        "More line spacing" => "reader-line-more",
        "Less line spacing" => "reader-line-less",
        "Zoom in" => "reader-zoom-in",
        "Zoom out" => "reader-zoom-out",
        "Fit width" => "reader-fit-width",
        "Fit page" => "reader-fit-page",
        "Rotate" => "reader-rotate",
        "Rotate left" => "reader-rotate-left",
        "Rotate right" => "reader-rotate-right",
        "Rotate Clockwise" => "reader-rotate-clockwise",
        "Show two pages side by side" => "reader-two-pages",
        "Split into pages" => "reader-split",
        "Switch to paginated mode" => "reader-paginated",
        "Switch to scrolled mode" => "reader-scrolled",
        "Next page" => "reader-next-page",
        "Previous page" => "reader-previous-page",
        "Add comment on page" => "reader-comment",
        "Highlight selection in PDF" => "reader-highlight",
        "Find similar" => "reader-similar",
        "Continuous pages" => "reader-continuous",
        "Single page" => "reader-single",
        "Page arranger" => "reader-arranger",
        "Edit metadata" => "reader-metadata",
        "Document Properties" => "reader-properties",
        "Sign…" => "reader-sign",
        "Protect with Password…" => "reader-protect",
        "Remove Password…" => "reader-remove-password",
        "Save page changes" => "reader-save-pages",
        "Find in document" => "reader-find",
        "Add bookmark" => "reader-bookmark-add",
        "Remove bookmark" => "reader-bookmark-remove",
        "Move up" => "reader-move-up",
        "Move down" => "reader-move-down",
        "Delete page" => "reader-delete-page",
        "Extract this page" => "reader-extract",
        "Exit page arranger" => "reader-exit-arranger",
        "Open a document" => "reader-chooser-open",
        "Save arranged PDF" => "reader-chooser-arranged",
        "Add pages from a PDF" => "reader-chooser-merge",
        "Choose a certificate" => "reader-chooser-certificate",
        "Save signed PDF" => "reader-chooser-signed",
        "Documents" => "reader-documents",
        "Certificates (PKCS #12)" => "reader-certificates",
        "No file chooser on this system, so the arranged PDF was not saved." => {
            "reader-save-arranged-unavailable"
        }
        "This desktop's file portal cannot choose a save destination." => {
            "reader-save-arranged-unsupported"
        }
        "This build is not permitted to open a save destination chooser." => {
            "reader-save-arranged-not-declared"
        }
        "The save destination chooser was refused permission." => "reader-save-arranged-denied",
        "The save destination chooser could not be attached to this window." => {
            "reader-save-arranged-parent"
        }
        "The save destination chooser failed to open." => "reader-save-arranged-failed",
        "Keyboard Shortcuts" => "reader-shortcuts",
        "Edit Metadata" => "reader-edit-metadata-title",
        "Edit the PDF metadata fields." => "reader-edit-metadata-description",
        "Clear Metadata" => "reader-clear-metadata",
        "Clear all metadata" => "reader-clear-all-metadata",
        "Save" => "reader-save",
        "Close" => "reader-close",
        "Cancel" => "reader-cancel",
        "Document title" => "reader-document-title",
        "Author name" => "reader-author-name",
        "Document subject" => "reader-document-subject",
        "Keywords" => "reader-keywords",
        "Creator" => "reader-creator",
        "Producer" => "reader-producer",
        "Creation date" => "reader-creation-date",
        "Modification date" => "reader-modification-date",
        "Comma-separated keywords" => "reader-keywords-placeholder",
        "Creating application" => "reader-creating-application",
        "PDF producer" => "reader-pdf-producer",
        "No date" => "reader-no-date",
        "Remove date" => "reader-remove-date",
        "Creation date time" => "reader-creation-time",
        "Modification date time" => "reader-modification-time",
        "Remove creation date" => "reader-remove-creation-date",
        "Remove modification date" => "reader-remove-modification-date",
        "Couldn’t save metadata" => "reader-metadata-save-failed",
        "Retry" => "reader-retry",
        "File name" => "reader-filename",
        "Pages" => "reader-pages",
        "File size" => "reader-file-size",
        "PDF version" => "reader-pdf-version",
        "Encrypted" => "reader-encrypted",
        "Yes" => "reader-yes",
        "No" => "reader-no",
        "Password Required" => "reader-password-required",
        "Enter password…" => "reader-password-placeholder",
        "Password" => "reader-password",
        "Unlock" => "reader-unlock",
        "This PDF is password-protected. Enter the password to open it." => {
            "reader-password-message"
        }
        "Wrong password. Try again." => "reader-password-wrong",
        "Unlocking…" => "reader-unlocking",
        "Remove every metadata field from this PDF? This rewrites the file." => {
            "reader-confirm-clear-metadata"
        }
        "Remove the creation and modification dates from this PDF?" => {
            "reader-confirm-remove-both-dates"
        }
        "Remove the creation date from this PDF?" => "reader-confirm-remove-creation-date",
        "Remove the modification date from this PDF?" => "reader-confirm-remove-modification-date",
        "No PDF dates are selected for removal." => "reader-confirm-no-dates",
        "Pass a document path on the command line instead." => "reader-chooser-cli-hint",
        _ => return label.to_string(),
    };
    match enzui_iced_core::localization::scoped() {
        Some(localizer) => localizer.text(key),
        None => label.to_string(),
    }
}
