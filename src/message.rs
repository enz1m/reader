//! The single application message enum.

use std::path::PathBuf;

use enzui_iced_core::app_runtime::AccessibilityWake;

use crate::model::{Format, ReadingTheme, SidebarTab};
use crate::plumbing::epub_support::EpubLoadResult;
use crate::plumbing::pdf_ops::MetadataField;

/// The result of an off-thread document load.
#[derive(Debug, Clone)]
pub enum LoadOutcome {
    Epub(Box<EpubLoadResult>),
    Cbz(Box<crate::plumbing::cbz_support::CbzLoadResult>),
    /// The document's page geometry plus the rasters of its first band.
    ///
    /// `pages.len()` is normally far below `total_pages`: `extents` covers every page, so
    /// the column can scroll to a page whose raster arrives later.
    Pages {
        pages: Vec<PageBitmap>,
        total_pages: usize,
        /// The extent of every page of the document, `total_pages` long.
        extents: Vec<crate::raster::PageExtent>,
        /// The selectable text of each rendered page, parallel to `pages`, from the same
        /// off-thread poppler pass as the raster.
        text: Vec<crate::pdf::PdfPageText>,
    },
    /// A bounded, user-facing document admission refusal, shown as is.
    Refused(String),
    /// A raw diagnostic (a parser message, a worker failure). Logged, never
    /// shown: the banner says only which format could not be opened.
    Error(String),
}

/// A rendered page bitmap, produced off-thread and turned into an
/// `image::Handle` in `update` (handles are cloneable + Send).
#[derive(Debug, Clone)]
pub struct PageBitmap {
    pub index: usize,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Outcome of exporting a parent for an Open File request.
#[derive(Debug, Clone)]
pub enum PickOutcome {
    /// Open the chooser with this parent lease.
    Parented(enzui_iced_parent_lease::Handoff<enzui_iced_wayland::ParentExport>),
    /// Open the chooser without a parent.
    Unparented,
    /// Do not open a chooser because the parent became invalid.
    Aborted,
}

/// Path, file presence, previous bookmark count and observed bookmark count from a library scan.
pub type AvailabilityResult = (String, bool, Option<usize>, Option<usize>);

#[derive(Debug, Clone)]
pub enum Msg {
    InstanceWake,
    /// Shared XDG theme-file, watcher and desktop appearance event, forwarded unchanged
    /// to [`enzui_iced_core::theme_file::integration::ThemeIntegration`]. It drives only
    /// the chrome theme, never the reading palette.
    Theme(Box<enzui_iced_core::theme_file::integration::ThemeEvent>),
    /// One enzui-center connector event.
    #[cfg(feature = "center")]
    Center(enzui_iced_agent::center::ConnectorEvent),
    /// A transient-status host event: an auto-dismiss tick from the armed
    /// expiry wake, the reader dismissing an acknowledgement, or a toast
    /// action. Reduced straight into `model.toasts`.
    Toast(enzui_iced_widgets::toast_host::ToastEvent),
    /// Coalesced accessibility wake from the shared bootstrap.
    ///
    /// The receipt names the exact batch of routed actions; the shared applier drains it
    /// (or nothing, if already consumed) in the window that published each target.
    /// Without it an AccessKit action against an idle Reader would wait forever.
    A11yWake(AccessibilityWake),
    // Library / open.
    OpenPath(PathBuf),
    FilesDropped(enzui_iced_widgets::file_drop::FileDrop),
    FilesRetained {
        attempt: u64,
        receipt: enzui_iced_widgets::file_drop::FileDrop,
        copies: Result<enzui_iced_core::drop_files::CopiedFiles, String>,
    },
    OpenDialog,
    /// Drives the portal file chooser's pump; [`crate::app::App::subscription`] emits it
    /// only while [`crate::picker::Picker::is_pending`].
    PortalTick,
    /// Records the current main window for parented requests.
    MainWindowReady(Option<iced::window::Id>),
    /// Invalidates the main window and pending parent exports.
    MainWindowClosed,
    /// Delivers the runtime's owned Wayland window payload.
    PickerParentResolved {
        /// Request generation.
        generation: u64,
        /// Picker target.
        target: crate::picker::PickerTarget,
        /// Parent window.
        window: iced::window::Id,
        /// Move-once native payload.
        payload: enzui_iced_parent_lease::Handoff<Option<Box<dyn std::any::Any + Send>>>,
    },
    /// Delivers a completed parent export.
    PickResolved {
        /// Request generation.
        generation: u64,
        /// Picker target.
        target: crate::picker::PickerTarget,
        /// Parent window.
        window: iced::window::Id,
        /// Export outcome.
        outcome: PickOutcome,
    },
    /// Open the library browser, the app's home screen. The `Browse library` titlebar
    /// control and closing a document both route here.
    GoLibrary,
    /// The library browser's filter field changed. Narrows the list by
    /// case-insensitive title/author substring.
    LibraryQueryChanged(String),
    LibrarySortMenu(enzui_iced_widgets::menu_button::MenuButtonEvent),
    /// The virtualised Library's live viewport, retained so a refresh does not reset the
    /// scroll position.
    LibraryScrolled {
        offset: f32,
        viewport_height: f32,
    },
    /// A whole-document load finished off the reducer.
    ///
    /// `attempt` is the load attempt the request was issued under. A completion that
    /// `Model::admits_document_load` rejects was superseded and is dropped before it
    /// touches any state; a reopen of the same path also supersedes.
    DocumentLoaded {
        attempt: u64,
        path: PathBuf,
        format: Format,
        outcome: LoadOutcome,
    },
    /// A generation-bound annotation sidecar restore finished off-thread.
    AnnotationsRestored {
        generation: u64,
        base_revision: u64,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    },
    /// Native PDF annotations loaded for the current annotation generation.
    NativePdfAnnotationsLoaded {
        generation: u64,
        outcome: Result<crate::plumbing::pdf_ops::PdfAnnotationSnapshot, String>,
    },
    PdfNavigationLoaded {
        generation: u64,
        outcome: Result<crate::plumbing::pdf_navigation::PdfNavigationData, String>,
    },

    // Navigation.
    NextChapter,
    PrevChapter,
    GoToChapter(usize),
    /// Jump the PDF/CBZ all-pages viewport to this 0-based rendered page.
    /// Produced by the framework's 1-based `enz_pagination` control.
    GoToPage(usize),
    /// Begin the viewport operation for a restored document position.
    BeginPositionRestore {
        generation: u64,
    },
    /// Track the page under the viewport centre during ordinary scrolling.
    /// `generation` rejects callbacks from a superseded viewport.
    PagesScrolled {
        generation: u64,
        offset: f32,
        viewport_height: f32,
        viewport_width: f32,
    },
    /// A band of pages finished rasterising off the reducer.
    ///
    /// `generation` rejects retired documents; `requested` retires in-flight indices
    /// even when rendering produced no page.
    PagesRasterised {
        generation: u64,
        requested: Vec<usize>,
        pages: Vec<PageBitmap>,
        text: Vec<crate::pdf::PdfPageText>,
    },
    /// A raster that replaces a page on screen is uploaded and can be shown
    /// without a blank frame. `None` when the renderer could not allocate it.
    PageRasterAllocated {
        generation: u64,
        page: Box<crate::raster::RenderedPage>,
        allocation: Option<iced::widget::image::Allocation>,
    },
    PdfThumbnailsScrolled {
        generation: u64,
        offset: f32,
        viewport_height: f32,
    },
    PdfThumbnailsRendered {
        generation: u64,
        requested: Vec<usize>,
        pages: Vec<PageBitmap>,
    },
    NextPage,
    PrevPage,
    /// The floating pager's page-number field was edited.
    PageEntryEdited(String),
    /// The floating pager's page number was submitted: go to it.
    PageEntrySubmitted,

    // Typography (EPUB).
    FontSizeUp,
    FontSizeDown,
    LineSpacingUp,
    LineSpacingDown,
    /// Select a specific reading theme and close the chooser popup.
    SetReadingTheme(ReadingTheme),
    /// Toggle the anchored reading-theme chooser popup opened by the `palette` titlebar
    /// control.
    ToggleReadingThemeMenu,
    /// Dismiss the reading-theme chooser popup: Escape or an outside press,
    /// routed through `dismissible_anchored_menu`.
    CloseReadingThemeMenu,

    FootnoteClicked(String),
    DismissFootnote,

    CopiedSelection(String),
    /// Completion is separate so selection feedback follows the settled clipboard write.
    SelectionCopied(Result<(), iced::clipboard::Error>),
    /// The selection within the reading scope changed: `Some` for a new selection with its
    /// text, document range and on-screen rectangle, `None` when cleared.
    SelectionChanged(Option<enzui_iced_widgets::selection_scope::ScopeSelection>),
    /// Persist the live EPUB prose selection as a char-anchored highlight.
    HighlightSelection,
    /// Open the note composer with the live EPUB prose selection as its
    /// durable content anchor.
    AddNoteToSelection,
    /// Copy the live selection from the contextual action surface.
    CopySelection,
    /// Dismiss the contextual selection actions without creating content.
    DismissSelectionActions,

    // Chrome.
    /// Toggle the navigation-split sidebar. Routed through
    /// `NavSplitState::toggle_for_presentation`, so it flips the inline
    /// preference or the overlay drawer depending on the resolved presentation.
    ToggleSidebar,
    /// Dismiss the overlay sidebar drawer (scrim click / navigation).
    DismissSidebar,
    /// The window's new logical size.
    ///
    /// Carries the height because paginated EPUB cuts pages to the viewport.
    WindowResized(iced::Size),
    DismissError,
    DismissPickerFailure,
    /// Hide the persistence notice until the failure changes or a close is vetoed;
    /// the failure itself stays retained until Retry settles it.
    HidePersistenceAlert,
    /// A cooperative close request for one exact window, from the compositor's
    /// `CloseRequested`, the titlebar's Close command or a test. Enters the close FSM
    /// (`crate::app::App::begin_close_drain`), which flushes the session, drains the
    /// blocking owner and then emits `iced::window::close(exact_id)`.
    ///
    /// `None` falls back to the retained main window; a windowless test still enters the
    /// FSM but produces no close task.
    CloseWindow(Option<iced::window::Id>),
    /// Retry a vetoed close on the same window. A no-op outside
    /// [`crate::app::CloseState::Vetoed`]; re-issues the failed persistence entries and
    /// re-enters `Draining`.
    RetryClose,
    /// Close-drain tick, fired only in [`crate::app::CloseState::Draining`].
    /// `crate::app::App::check_close_drain_completion` observes pool occupancy, durable
    /// debt, dirty flags and the deadline: a clean drain emits the real close, an expired
    /// one vetoes with a summary in `model.error`.
    CloseDrainTick,
    /// Typed Busy consequence for a raster band submission, naming the indices whose
    /// in-flight slots must be released for `ensure_page_rasters` to resubmit them.
    RasterBandRefusedBusy {
        generation: u64,
        indices: Vec<usize>,
    },
    /// Typed Busy consequence for a PDF thumbnail submission. See
    /// [`Self::RasterBandRefusedBusy`] for the retry contract.
    PdfThumbnailsRefusedBusy {
        generation: u64,
        indices: Vec<usize>,
    },
    /// Typed Busy consequence for a search index scan. Re-arms `search_pending` and the
    /// shared [`Self::BusyRetryTick`] so the query is not dropped.
    SearchRefusedBusy {
        generation: u64,
        format: Format,
    },
    /// Typed Busy consequence for a recent-book preview render. Carries the intent so the
    /// reducer requeues it on `crate::app::App::preview_pending`, and arms the shared
    /// busy-retry wake.
    RecentThumbnailRefusedBusy {
        path: String,
        source: crate::app::PreviewSource,
    },
    /// Coalesced busy-retry wake, fired only while some class holds Busy-refused intent.
    /// See `crate::app::App::pump_busy_retry`.
    BusyRetryTick,
    /// Window chrome gesture from `enz_title_bar`. `Close` is redirected to
    /// [`Self::CloseWindow`] for the session flush; other commands use
    /// `WindowCommand::into_task`.
    Window(enzui_iced_widgets::title_bar_gesture::WindowCommand),

    RemoveAnnotation(u64),
    /// Advance a highlight through the supported PDF colour palette.
    CyclePdfAnnotationColor(u64),
    /// One off-reducer native-PDF annotation mutation finished.
    ///
    /// `op_token` is the per-dispatch token from `crate::app::App::admit_pdf_mutation`; a
    /// mismatch drops the completion whole. `document_generation` gates against a
    /// document swap under a live mutation.
    PdfMutationCompleted {
        op_token: u64,
        document_generation: u64,
        intent: Box<crate::app::PdfMutationIntent>,
        outcome: Result<
            crate::plumbing::pdf_ops::PdfMutationOutcome,
            enzui_iced_core::blocking_task::BlockingTaskError,
        >,
    },
    /// The shared bounded owner refused a PDF annotation mutation with
    /// [`enzui_iced_core::blocking_task::BlockingTaskError::Busy`]. At most one retained
    /// intent is retried via `BusyRetryTick`; a stale Busy is dropped whole.
    PdfMutationRefusedBusy {
        op_token: u64,
        document_generation: u64,
        intent: Box<crate::app::PdfMutationIntent>,
    },
    /// `Retry` on the reconciliation banner: reopen the PDF on the bounded owner, classify
    /// the retained `crate::app::UncertainPublication`, apply `Applied` or `NotApplied`,
    /// and keep the record for `StillUncertain`.
    RetryPdfMutationUncertain,
    /// The blocking Retry produced a classification. Only a matching
    /// `(op_token, retry_sequence)` applies. The retained
    /// [`enzui_iced_core::user_file::DocumentAuthority`] returns to `lost_authority` first,
    /// so no outcome drops it.
    PdfMutationUncertainRetried {
        op_token: u64,
        retry_sequence: u64,
        outcome: Result<
            crate::app::PdfReconciliationClass,
            enzui_iced_core::blocking_task::BlockingTaskError,
        >,
    },
    /// `Dismiss` on the uncertain-publication banner: hides the warning and stops
    /// close-drain blocking on this debt, without claiming the file matches. `Keep Open`
    /// is the passive default and sends no message.
    DismissPdfMutationUncertainWarning,
    /// Acknowledge the retained [`crate::model::ArrangementUncertainPublication`] so close
    /// is no longer vetoed; the destination may already have been written.
    DismissArrangementUncertainPublication,
    /// Acknowledge the retained [`crate::model::ExtractUncertainPublication`], releasing
    /// its per-output descriptors so close is no longer vetoed. On-disk outputs are left
    /// as they are.
    DismissExtractUncertainPublication,
    /// Activate a content-first annotation row and jump to its persisted
    /// passage (EPUB CFI or PDF page/rectangle).
    GoToAnnotation(u64),
    /// Toggle a bookmark on the current page/chapter.
    ToggleBookmark(usize),
    /// The page-free form the `Ctrl+D` chord dispatches
    /// ([`crate::shortcuts::command_message`]); the reducer reads the current page and
    /// forwards to [`Self::ToggleBookmark`].
    ToggleCurrentBookmark,
    /// Navigate only while the bookmark's originating document and format remain open.
    GoToBookmark {
        format: Format,
        document_key: String,
        position: usize,
    },
    /// Remove a bookmark by page/chapter index.
    RemoveBookmark(usize),

    SelectSidebarTab(SidebarTab),
    /// Expand/collapse secondary annotation tools (filter, undo, copy).
    ToggleAnnotationTools(bool),
    /// The annotations filter field text changed.
    AnnotationFilterChanged(String),
    /// Copy a single annotation to the clipboard as a one-line citation
    /// (`Copy with citation`). Carries the annotation id.
    CopyCitation(u64),
    /// Completion is separate so citation feedback follows the settled clipboard write.
    CitationCopied(Result<(), iced::clipboard::Error>),
    /// Begin editing an annotation's text inline (`Edit Note`). Carries the id.
    BeginEditAnnotation(u64),
    /// The inline edit field text changed.
    EditAnnotationChanged(String),
    /// Commit the inline edit to the annotation and persist (`Save`).
    CommitEditAnnotation,
    /// Abandon the inline edit without saving (`Cancel`).
    CancelEditAnnotation,
    /// Copy all annotations and bookmarks as JSON.
    CopyAnnotationsJson,
    AnnotationsJsonCopied(Result<(), iced::clipboard::Error>),
    /// Copy all annotations and bookmarks as Markdown.
    CopyAnnotationsMarkdown,
    AnnotationsMarkdownCopied(Result<(), iced::clipboard::Error>),

    DocumentKeyCommand(crate::shortcuts::Command),
    /// A shell keyboard command whose dispatch respects the current modal owner.
    ShellKeyCommand(crate::shortcuts::Command),
    OpenSearch,
    /// The find query changed. Updates the query immediately and schedules a debounced
    /// off-thread scan.
    SearchQueryChanged(String),
    /// The debounce window elapsed for a pending query: dispatch one
    /// off-thread scan. Emitted by [`crate::app::App::subscription`] only while
    /// [`crate::model::Model::search_pending`].
    SearchTick,
    /// An off-thread scan finished; it applies only while `generation` matches the model's.
    SearchCompleted {
        generation: u64,
        hits: Vec<crate::plumbing::epub_support::EpubHit>,
    },
    PdfSearchCompleted {
        generation: u64,
        hits: Vec<crate::search::PdfSearchHit>,
    },
    /// A search result row was activated (clicked or Enter): select that hit,
    /// jump to its passage, and move the transient highlight there.
    SearchResultActivated(usize),
    /// Move to the next match (`Next match`) and navigate to its chapter.
    NextMatch,
    /// Move to the previous match (`Previous match`) and navigate to its chapter.
    PrevMatch,

    /// Move keyboard focus to the next focusable control (Tab).
    ///
    /// iced does not advance focus on Tab itself, so the app drives
    /// [`iced::widget::operation::focus_next`]. Screens have no modal focus trap.
    FocusNext,
    /// Move keyboard focus to the previous focusable control (Shift+Tab).
    FocusPrevious,
    /// Scoped focus and dismissal from the shell's standard modal host.
    ShellDialogEvent(enzui_iced_widgets::modal_host::DialogEvent<crate::overlay_focus::DialogKey>),

    Undo,
    /// Redo the most recently undone action (`Ctrl+Shift+Z` / the Redo
    /// button). Same empty-state semantics as [`Self::Undo`].
    Redo,

    OpenPageArranger,
    /// Leave the page arranger (`Exit page arranger`).
    ClosePageArranger,
    /// `Move up` on the arranger entry with this id. A no-op on the first row
    /// (where the control is also disabled).
    MovePageUp(u64),
    /// `Move down`. A no-op on the last row.
    MovePageDown(u64),
    /// A drag-reorder completed in the `enz_reorder_list`.
    PageDropped(enzui_iced_core::drag::DropRequest),
    /// `Open page` — leave the arranger and scroll the reading surface to
    /// this arranger entry.
    OpenPage(u64),
    /// `Delete page`: asks; the mutation runs only on an affirmative confirm.
    RequestDeletePage(u64),
    /// `Extract this page` — likewise asks first.
    RequestExtractPage(u64),
    /// Open `Protect with Password…`, or `Remove Password…` when the PDF
    /// already has one.
    OpenPdfPasswordForm,
    /// `Sign…`: open the sign dialog for the open PDF.
    OpenSignDialog,
    SignNameChanged(String),
    SignFontSelected(String),
    /// Choose a PKCS #12 certificate to sign with.
    ChooseSigningCertificate,
    ClearSigningCertificate,
    SignPasswordChanged(String),
    ToggleSignPasswordReveal,
    /// Leave the sign flow at any step.
    CancelSigning,
    /// Close the dialog and wait for a click on a page.
    PlaceSignature,
    /// The pointer over page `page` while placing, in the page's displayed
    /// pixels, with the page's displayed size.
    SignaturePointer {
        page: usize,
        at: iced::Point,
        displayed: iced::Size,
    },
    /// A click on page `page` while placing: the signature goes there.
    SignatureClicked(usize),
    /// The signed copy was written (its path), or why not.
    SignedCopyWritten(Result<std::path::PathBuf, String>),
    /// An event from the password dialog's form.
    PdfPasswordForm(enzui_iced_widgets::form_dialog::FormDialogEvent),
    /// The arranger grid's selection changed: the keys of the selected pages.
    ArrangerSelectionChanged(Vec<String>),
    /// The arranger grid scrolled.
    ArrangerScrolled {
        offset: f32,
        viewport_height: f32,
    },
    /// Turn every page in `ids` a quarter turn.
    RotatePages {
        ids: Vec<u64>,
        clockwise: bool,
    },
    /// Delete the pages `ids`, asking first.
    RequestDeletePages(Vec<u64>),
    /// Extract the pages `ids` into one new PDF, asking first.
    RequestExtractPages(Vec<u64>),
    /// `Apply` atomically publishes the current draft over the open PDF.
    ApplyPageArrangement,
    /// `Save As…` asks the desktop portal for the publication destination.
    SavePageArrangementAs,
    /// Completion of one generation-keyed arranger publication.
    PageArrangementPublished {
        generation: u64,
        source: PathBuf,
        destination: PathBuf,
        arrangement: crate::pages::PageArrangement,
        result: Result<
            crate::plumbing::pdf_ops::ArrangementPublication,
            crate::plumbing::pdf_ops::PdfWriteError,
        >,
    },
    /// Request another PDF through the parented merge chooser.
    RequestMergePdf,
    /// `Merge PDF` — commit the merge of the typed path.
    MergePdf,
    /// The merge-path field text changed.
    MergePathChanged(String),
    /// Any exit reported by the framework confirm dialog presenting a
    /// [`crate::model::PendingPageAction`].
    PageConfirmEvent(enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent),

    OpenMetadata,
    /// `Document Properties` — read the read-only properties and show them.
    OpenProperties,
    /// Dismiss whichever of the two is showing.
    CloseOverlay,
    /// One metadata field was edited; empty dates request confirmed removal.
    MetadataFieldChanged(MetadataField, String),
    /// A metadata date's calendar picker.
    MetadataDatePicker(
        MetadataField,
        enzui_iced_widgets::date_picker::DatePickerEvent,
    ),
    /// A metadata date's time field.
    MetadataTime(
        MetadataField,
        enzui_iced_widgets::time_field::TimeFieldEvent,
    ),
    /// Remove a metadata date from the document.
    ClearMetadataDate(MetadataField),
    /// `Save` in the metadata editor — write `/Info` back to the file.
    SaveMetadata,
    /// Retry the exact metadata destination after a write or reopen failure.
    RetryMetadataSave,
    /// `Clear all metadata`: asks before wiping `/Info`.
    RequestClearMetadata,

    ZoomIn,
    /// `Zoom out`. Saturates at [`crate::viewport::ZOOM_MIN`].
    ZoomOut,
    /// `Fit width`: back to the identity factor and to `FitWidth` mode.
    ZoomFitWidth,
    ZoomFitPage,
    /// `Rotate` / `Rotate Clockwise`: turns the whole reading surface a quarter turn
    /// clockwise.
    RotateClockwise,
    /// `Two pages`: show pages side by side in pairs, or back to one column.
    ToggleSpread,
    TogglePageViewMode,
    /// `Rotate left` on one arranger page.
    RotatePageLeft(u64),
    /// `Rotate right` on one arranger page.
    RotatePageRight(u64),
    /// `Split into pages` / `Split PDF`: asks first, since it writes N files.
    RequestSplitPdf,
    /// Toggle between `Paginated Mode` and `Scrolled Mode` (EPUB).
    ToggleEpubViewMode,

    PasswordRequired(PathBuf),
    /// The secure field's text changed. Never logged.
    PasswordChanged(String),
    /// The secure field's reveal-eye was pressed.
    TogglePasswordReveal,
    /// `Unlock`: retry the open with the typed password, off-thread so the window keeps
    /// painting.
    SubmitPassword,
    /// The off-thread unlock attempt finished.
    ///
    /// `attempt` is the generation the attempt started with; any other is dropped, so a
    /// slow attempt cannot land after cancel or retry. The password is not carried here.
    PasswordUnlockResult {
        attempt: u64,
        path: PathBuf,
        outcome: LoadOutcome,
    },
    /// `Cancel` in the unlock dialog. Clears the typed secret and invalidates
    /// any unlock attempt still running.
    CancelPassword,

    FlushPersistence,
    /// Persistence settlement keyed by destination generation; errors retain debt and only
    /// matching Saved retires it.
    PersistenceFinished {
        destination: String,
        generation: u64,
        outcome: crate::plumbing::persistence::PersistenceOutcome,
    },
    /// User activated the persistence Danger banner's Retry action.
    /// Re-registers every failed destination under a fresh generation and
    /// dispatches its write task; a Saved outcome then clears the alert.
    RetryPersistence,
    /// Re-read one write-protected state origin without authorizing replacement.
    RetryStateRecovery(crate::plumbing::persistence::StateOrigin),
    /// The initial library restore finished off the UI thread.
    StartupLibraryRestored {
        generation: u64,
        base_revision: u64,
        open_path: Option<PathBuf>,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    },
    /// A recovery read tied to both its request generation and model revision.
    StateRecoveryReloaded {
        generation: u64,
        base_revision: u64,
        origin: crate::plumbing::persistence::StateOrigin,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    },
    /// Open the preserved copy's directory or protected source directory.
    RevealRecoveryLocation(crate::plumbing::persistence::StateOrigin),
    /// The OpenURI backend signalled a request-state change.
    RecoveryLocationTick,
    /// Opening the recovery directory reached a generation-keyed terminal outcome.
    RecoveryLocationFinished {
        generation: u64,
        outcome: enzui_iced_core::portal::PortalOutcome<()>,
    },
    /// Explicitly retire recovery authority and authorize an empty replacement.
    StartEmptyState(crate::plumbing::persistence::StateOrigin),

    OpenNoteEditor,
    /// `Edit Note` — open the composer seeded with an existing note's text.
    OpenNoteEditorForEdit(u64),
    /// An editor action from the multiline note composer, applied through its
    /// presenter.
    NoteEditorAction(iced::widget::text_editor::Action),
    /// `Save` in the composer.
    CommitNoteEditor,
    /// `Add comment on page` — collect authored text for a `/Text` annotation.
    /// Refused while no selection is live.
    AddPdfComment,
    /// `Highlight selection in PDF` — write a `/Highlight` into the PDF.
    /// Refused while no selection is live.
    AddPdfHighlight,
    /// Text was selected on a rendered PDF page: `range` indexes the page's selectable
    /// glyphs and `finished` marks the end of the gesture. The resulting
    /// [`crate::model::PdfSelection`] is the only source of the rectangles and string the
    /// PDF writer uses.
    PdfTextSelected {
        page: usize,
        range: std::ops::Range<usize>,
        finished: bool,
    },
    /// Copy the PDF selection's text.
    CopyPdfSelection,
    /// Search the document for the selected text (PDF or EPUB).
    FindSelection,
    /// The actions strip under the PDF selection was dismissed (outside
    /// press or Escape); the selection itself stays.
    DismissPdfSelectionActions,
    /// A click on empty page background cleared the selection. Also
    /// re-disables the Highlight / Comment controls.
    ClearPdfSelection,
    /// `Find similar` — seed the find panel with an annotation's text.
    FindSimilar(u64),
    /// `Remove from recents` — drop a book from the welcome list.
    RemoveFromRecents(String),

    /// A recent-book first-page thumbnail finished rendering off-thread.
    /// `thumb` is `None` when the format has no page raster (EPUB) or the
    /// render failed; the card keeps its generic glyph in that case.
    RecentThumbnailRendered {
        path: String,
        thumb: Option<PageThumbnail>,
    },

    /// One bounded batch of an off-thread file-availability scan finished.
    ///
    /// `generation` keys the owning scan; a batch for a superseded generation
    /// is dropped whole. Each result carries the prior cached bookmark count so
    /// a concurrent successful sidecar save cannot be overwritten by an older
    /// disk read. A path removed from the store is ignored.
    AvailabilityBatchDone {
        generation: u64,
        next_index: usize,
        total: usize,
        outcome: Option<Vec<AvailabilityResult>>,
    },

    OpenContextMenu(crate::context_menu::ContextTarget, iced::Point),
    /// A typed menu host event stamped to reject queued events from another showing.
    ContextMenuEvent(u64, enzui_iced_widgets::context_menu_host::ContextMenuEvent),

    /// Open the Keyboard Shortcuts help dialog, also reachable by F1.
    OpenShortcuts,
    /// Close the Keyboard Shortcuts dialog.
    CloseShortcuts,

    /// A long chapter's paragraph geometry, shaped off the UI thread, or `None` when the
    /// worker could not run it.
    EpubGeometryMeasured(
        crate::reading_pane::GeometryKey,
        Option<std::sync::Arc<crate::reading_pane::ChapterGeometry>>,
    ),
    /// The paginated page box was laid out at this height; pages are cut
    /// to it.
    EpubPageBoxMeasured(f32),
    /// The wheel moved over a paginated page, in notches (positive is up).
    /// Whole notches turn pages; a touchpad's fractions add up first.
    EpubPageWheel(f32),
    EpubScrolled {
        generation: u64,
        fraction: f32,
        /// The absolute offset and the viewport height, which decide the
        /// paragraphs a long chapter builds.
        offset: f32,
        viewport_height: f32,
    },

    /// The titlebar overflow trigger was activated.
    ToggleToolbarOverflow,
    /// Native action-menu dismissal, including activation, Escape and outside press.
    CloseToolbarOverflow,

    Noop,
}

#[path = "message/kind.rs"]
mod kind;

/// A small first-page raster for a "Continue reading" card.
///
/// Rendered off-thread and kept small (see [`crate::pdf::THUMBNAIL_WIDTH`]) so every
/// recent is cheap to render and hold.
#[derive(Debug, Clone)]

pub struct PageThumbnail {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
