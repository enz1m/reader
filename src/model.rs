//! Application state and persisted reading palettes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use enzui_iced_core::confirm::ConfirmState;
use enzui_iced_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey};
use enzui_iced_widgets::nav_split::NavSplitState;
use enzui_iced_widgets::toast::ToastTone;
use enzui_iced_widgets::toast_host::{ToastExpirySchedule, ToastHost, ToastSpec, WaitingBudget};

use crate::pages::PageArrangement;
use crate::plumbing::epub_support::{EpubLoadResult, ReadingParagraph};
use crate::plumbing::pdf_ops::{DocProperties, MetadataField, PdfMetadata};
use crate::plumbing::persistence::{AnnotationsFile, AppState, RecentBook};
use crate::undo::UndoStack;

/// The whole live open-document + view state.
pub struct Model {
    pub screen: Screen,
    pub file_path: Option<PathBuf>,
    pub file_name: String,
    /// The document's display title: its metadata title when usable, else the file
    /// basename, matching the recents card.
    ///
    /// Empty until a load resolves it; `view::states::document_name` then falls back to
    /// `file_name`.
    pub display_title: String,
    pub format: Format,
    /// The document's page count: every page, whether or not its raster exists yet.
    pub page_count: usize,
    /// The document's true page count. Equal to [`Self::page_count`]; retained
    /// because the arranger and the split-confirmation prose read it.
    pub total_page_count: usize,
    pub current_page: usize,

    // EPUB reading state.
    pub epub: Option<EpubDoc>,
    pub epub_current_chapter: usize,
    /// Retaining the resolved body lets dismissal leave the chapter and its
    /// scroll/page anchor unchanged.
    pub active_footnote: Option<crate::plumbing::footnotes::Footnote>,
    pub epub_font_size: i32,
    pub epub_line_spacing: f32,
    pub reading_theme: ReadingTheme,
    pub epub_view_mode: EpubViewMode,

    /// The last-known relative scroll position (0.0…1.0) within the current EPUB chapter,
    /// from the pane's `on_scroll`, used to restore the position after a reflow.
    pub epub_scroll_fraction: f32,
    /// The scroll view's absolute offset and viewport height, from the same
    /// callback, so a long chapter builds only the paragraphs near the
    /// viewport.
    pub epub_scroll_offset: f32,
    pub epub_viewport_height: f32,
    /// The current chapter's paragraph geometry, shaped off the UI thread.
    pub epub_geometry: Option<std::sync::Arc<crate::reading_pane::ChapterGeometry>>,
    /// The geometry being shaped now, so it is asked for once.
    pub epub_geometry_pending: Option<crate::reading_pane::GeometryKey>,
    /// The arranger pages selected with click, Ctrl-click and Shift-click,
    /// by entry id in arrangement order: what the selection bar acts on.
    pub arranger_selection: Vec<u64>,
    /// The arranger grid's scroll offset and viewport height, which decide
    /// the thumbnails worth rendering.
    pub arranger_scroll_offset: f32,
    pub arranger_viewport_height: f32,
    pub epub_geometry_estimate:
        std::cell::RefCell<Option<std::sync::Arc<crate::reading_pane::ChapterGeometry>>>,

    /// Chapter-local character anchor shared with search and annotations; reflow changes page
    /// boundaries without changing this passage.
    pub epub_page_offset: usize,

    /// The cached page boundaries of the current chapter. Shaping happens only when the
    /// cache is invalidated (font size, line height, reading theme, window resize or
    /// chapter change), never from `view()`. Empty in [`EpubViewMode::Scrolled`] and for
    /// an empty chapter.
    pub epub_pages: Vec<crate::pagination::PageSpan>,
    /// The paginated page box as last laid out, in pixels. Pages are cut to
    /// it once known; until then the window size is the estimate.
    pub epub_page_box: Option<f32>,
    /// Wheel travel over a paginated page not yet spent on a page turn.
    pub epub_page_wheel: f32,

    /// Borrowable projection of the current character-anchored page; owned by the model for the
    /// view lifetime.
    pub epub_page_content: Vec<crate::plumbing::epub_support::ReadingParagraph>,

    /// Unix-epoch seconds at which the current reading session started, used to
    /// accumulate `reading_time_secs` on save. 0 = no session in progress.
    pub session_start_secs: u64,

    // PDF/CBZ rendered pages.
    /// The resident page rasters: a byte-bounded working set, not the document.
    /// [`Self::page_extents`] still sizes pages not rasterised yet.
    pub pages: crate::raster::PageRasterCache,
    /// Validated archive entries used for on-demand CBZ page decoding.
    pub cbz_entries: Arc<Vec<crate::plumbing::cbz_support::CbzPage>>,
    /// The extent of every page of the open document, `page_count` long.
    ///
    /// The reading column is laid out from this, so it is the same before and after any
    /// page rasterises and page jumps land exactly.
    pub page_extents: Vec<crate::raster::PageExtent>,
    /// The scroll offset and measured viewport dimensions, in logical pixels.
    pub pages_scroll_offset: f32,
    pub pages_viewport_height: f32,
    pub pages_viewport_width: f32,
    /// Pages whose rasterisation is in flight, so a second scroll event across
    /// the same band does not queue the same cairo pass twice.
    pub rasterising: std::collections::BTreeSet<usize>,
    /// Pages a rasterisation request came back empty for.
    ///
    /// Recording the failure once stops `ensure_page_rasters` re-asking on every message;
    /// the page renders as unavailable and is retried only when the document reopens.
    pub raster_failed: std::collections::BTreeSet<usize>,
    /// Bumped on every document open, and carried by every rasterisation task,
    /// so a band that comes back after the reader closed the book is dropped
    /// instead of painted into the next document.
    pub raster_generation: u64,
    pub thumbnail_rasters: crate::raster::PageRasterCache,
    pub thumbnail_scroll_offset: f32,
    pub thumbnail_viewport_height: f32,
    pub thumbnail_rasterising: std::collections::BTreeSet<usize>,
    pub thumbnail_failed: std::collections::BTreeSet<usize>,
    pub thumbnail_generation: u64,
    pub pdf_outline: Vec<crate::plumbing::pdf_navigation::PdfOutlineEntry>,
    pub pdf_navigation_loading: bool,
    pub pdf_navigation_error: Option<String>,
    pub pdf_outline_error: Option<String>,
    pub pdf_navigation_truncated: bool,
    pub pdf_navigation_generation: u64,
    /// Latest whole-document load identity, renewed even for same-path reopen; saturation
    /// refuses further admission rather than reusing an id.
    pub document_load_attempt: u64,
    /// The password of the open document, when it is encrypted.
    ///
    /// On-demand rasterisation reopens the PDF long after unlock, and poppler's `Document`
    /// is `!Send`, so the key is retained here. It is cleared by [`Self::reset_document`]
    /// and never carried on a `Msg`, persisted, or shown in the password field.
    pub pdf_password: Option<String>,
    // Generation attached to scroll callbacks and restore commands.
    pub(crate) position_restore_generation: u64,
    // Restored position retained until its viewport reports settlement.
    pub(crate) pending_position_restore: Option<PendingPositionRestore>,

    // Library / recents.
    /// Copy-on-write so a persistence task takes an O(1) snapshot and the clone and
    /// serialise stay off the reducer.
    pub app_state: Arc<AppState>,
    /// True until the generation-bound startup restore has settled.
    pub library_loading: bool,
    /// Blocks library-state publication when no private state root exists.
    pub library_persistence_unavailable: bool,
    pub recents: Vec<RecentBook>,

    /// The whole library: every document in [`Self::app_state`], newest-first,
    /// as a superset of [`Self::recents`]. Rebuilt by
    /// `app::refresh_projection` from the same persisted `app_state` as recents.
    /// Empty when nothing has ever been opened.
    pub library: Vec<RecentBook>,

    /// Persistent filtered/sorted provider consumed by the virtualised shelf.
    /// Rebuilt only when the library, filter or sort changes.
    pub library_projection: LibraryProjection,

    /// Monotonic version of [`Self::library_projection`], bumped by
    /// [`Self::rebuild_library_projection`]. It keys the [`LibraryHeightCache`]
    /// so a filter/sort/add/remove reshapes the row-height table while a mere
    /// scroll or repaint reuses it.
    pub library_revision: u64,

    /// Monotonic version of [`Self::library`]'s order/content, bumped only when
    /// a full projection rebuild reassigns the list. An in-flight availability
    /// scan pins this so it restarts if the ordering changes between batches;
    /// in-place availability and bookmark-count updates do not bump it.
    pub order_revision: u64,

    /// Cached variable row heights for the shelf, keyed by
    /// `(library_revision, content width, typography)`; see
    /// [`LibraryHeightCache`]. Interior-mutable so the shared-borrow `view`
    /// can populate it lazily.
    pub library_height_cache: LibraryHeightCache,

    /// Live absolute offset and measured viewport of the virtualised shelf.
    /// Kept in model state so refresh/rescan rebuilds rows without resetting
    /// the reader to the top.
    pub library_scroll_offset: f32,
    pub library_viewport_height: f32,

    /// Case-insensitive title/author filter for the library browser.
    /// Empty means "show everything". Distinct from [`Self::search_query`]
    /// (find-in-document) and [`Self::annotation_filter`].
    pub library_query: String,

    /// How the library browser orders its list. Defaults to
    /// [`LibrarySort::Recent`].
    pub library_sort: LibrarySort,
    pub library_sort_menu_model: enzui_iced_core::menu_model::MenuModel,
    pub library_sort_menu: enzui_iced_core::menu_model::MenuState,

    /// Last known file existence, keyed like [`AppState::files`]. An absent key
    /// is unprobed and displayed as available until the off-thread scan replies.
    pub availability: std::collections::HashMap<String, bool>,

    /// Bookmark counts loaded from each document's durable annotation sidecar.
    pub bookmark_counts: std::collections::HashMap<String, usize>,

    /// First-page preview thumbnails for the "Continue reading" cards, keyed by
    /// the book's `path`. Populated off-thread after recents load; a card with
    /// no entry here falls back to the generic book glyph, so the UI never
    /// blocks on rendering and EPUB (which has no page raster) keeps its glyph.
    pub recent_thumbnails: std::collections::HashMap<String, iced::widget::image::Handle>,

    /// Annotations sidecar for the open document, loaded on open and saved after each
    /// change; empty when none is open. Copy-on-write like [`Self::app_state`].
    pub annotations: Arc<AnnotationsFile>,
    /// Defers annotation controls and publication until sidecar admission settles.
    pub annotations_loading: bool,
    /// Blocks annotation controls and publication after admission loses authority.
    pub annotations_unavailable: bool,

    /// Which sidebar panel is showing (outline vs annotations). Defaults to
    /// [`SidebarTab::Contents`].
    pub sidebar_tab: SidebarTab,

    /// Case-insensitive substring filter for the annotations list. Empty means
    /// "show everything". When non-empty and nothing matches, the panel shows
    /// the "No annotations match the filter." empty state.
    pub annotation_filter: String,
    /// Whether secondary annotation management tools are disclosed. The
    /// content-first default keeps filtering, copying, and undo out of the way.
    pub annotation_tools_expanded: bool,

    /// The currently active text selection in the reading scope, or `None` when cleared.
    /// Anchors the contextual actions and supplies the range for annotations.
    pub active_selection: Option<enzui_iced_widgets::selection_scope::ScopeSelection>,
    /// Forwarded to `EnzSelectionScope::clear_generation`. Highlight, Add note, Copy,
    /// Dismiss and the navigation guard bump it with clearing `active_selection`, which
    /// drops the widget's interior selection on the next frame. A fresh drag or `Ctrl+A`
    /// is unaffected.
    pub selection_clear_generation: u64,

    /// The id of the annotation currently being edited inline (`Edit Note`),
    /// plus the in-progress draft of its text. `None` when no edit is open.
    /// Only one annotation is editable at a time.
    pub editing_annotation: Option<(u64, String)>,

    /// The shared transient-status host for annotation and page acknowledgements, written
    /// only through [`Self::announce`] / [`Self::refuse`]. Durable failures stay on their
    /// banners.
    pub toasts: ToastHost,
    /// The earliest-deadline one-shot wake for [`Self::toasts`]. Reconciled
    /// after every turn that touched the host; the subscription arms exactly
    /// one timer from it while a toast dwells and none when the host is idle.
    pub toast_expiry: ToastExpirySchedule,

    pub search_query: String,
    /// The search results list's scroll position, kept across views.
    pub search_scroll: enzui_iced_widgets::scroll_view::ScrollTranslation,
    /// EPUB chapter matches for [`Self::search_query`]. PDF page matches are
    /// held separately because they carry page geometry instead of character offsets.
    pub search_hits: Vec<crate::plumbing::epub_support::EpubHit>,
    /// Index into [`Self::search_hits`] of the selected result, which
    /// [`Self::active_hit`] returns and the transient highlight covers. Left at 0 when
    /// there are no hits.
    pub search_active: usize,
    /// The off-reducer EPUB search index, built once when the book opens.
    pub search_index: crate::search::SearchIndex,
    pub pdf_search_index: crate::search::PdfSearchIndex,
    pub pdf_search_hits: Vec<crate::search::PdfSearchHit>,
    pub pdf_search_truncated: bool,
    /// Set when an EPUB scan hit [`crate::plumbing::epub_support::MAX_EPUB_SEARCH_HITS`]
    /// and later matches were dropped, so the panel can say the list is partial.
    pub epub_search_truncated: bool,
    /// The generation of the most recent query, bumped on every `SearchQueryChanged`. A
    /// completed scan applies only while its generation matches, so a slow earlier scan
    /// cannot land on a later query.
    pub search_generation: u64,
    pub search_in_flight_generation: Option<u64>,
    /// A query has changed and its format-specific scan has not yet been dispatched.
    /// While set, [`crate::app::App::subscription`] runs a short coalescing
    /// tick that fires one off-thread scan for the latest query; an idle app
    /// (no pending query) runs no such timer.
    pub search_pending: bool,

    /// The framework navigation-split state: durable sidebar preference and overlay
    /// drawer flag, resolved by `nav_split::resolve_for_width`.
    pub nav: NavSplitState,
    pub overlay: Option<Overlay>,
    pub dialogs: enzui_iced_widgets::modal_host::DialogStack<crate::overlay_focus::DialogKey>,
    /// The right-click context menu that is up, or `None`.
    ///
    /// Not an [`Overlay`]: modal overlays make the page inert, while a menu dismisses on
    /// outside press and leaves the page live. `document_focused()` ignores it.
    pub context_menu: Option<crate::context_menu::OpenContextMenu>,
    pub(crate) context_menu_generation: u64,
    pub(crate) closed_context_menu: enzui_iced_widgets::context_menu_host::ContextMenu<
        crate::context_menu::ContextTarget,
        crate::context_menu::ContextAction,
    >,
    /// The page arranger's ordered page list. Rebuilt from the document's
    /// page count when a PDF opens; empty for EPUB (which has chapters, not
    /// a page tree to rearrange).
    pub arrangement: PageArrangement,
    pub arrangement_publish_generation: u64,
    pub arrangement_publication: Option<PageArrangementPublication>,
    pub arrangement_publish_failure: Option<String>,
    /// Retained debt for a Save As arrangement publication that landed on
    /// disk while the reducer had already retired the open document. It is
    /// not cleared by [`Self::reset_document`]: an uncertain destination
    /// file survives the swap so close is vetoed and the shell can name the
    /// destination the user must reconcile.
    pub arrangement_uncertain_publication: Option<ArrangementUncertainPublication>,
    /// Retained debt from a failing extract or split run. See
    /// [`ExtractUncertainPublication`] and
    /// `crate::app::App::extract_publication_debt`.
    pub extract_uncertain_publication: Option<ExtractUncertainPublication>,
    /// The destructive page/metadata action awaiting confirmation, plus the
    /// framework confirm-dialog presentation state. `None` = no dialog.
    pub pending_page_action: Option<PendingPageAction>,
    /// Framework `enz_confirm_dialog` lifecycle state for
    /// [`Self::pending_page_action`]. Rebuilt on each open so the dialog
    /// resets rather than remembering the last answer.
    pub page_confirm_state: ConfirmState,
    /// The metadata as last read from (or written to) the open PDF. The
    /// dialog edits this in place and `Save metadata` writes it back.
    pub metadata: PdfMetadata,
    /// The metadata as it was when the dialog opened, so `Cancel` can restore
    /// it without a reload and so `Save` can be a no-op when nothing changed.
    pub metadata_baseline: PdfMetadata,
    /// Retry state retained after metadata publication or reopen failure.
    pub metadata_save_failure: Option<MetadataSaveFailure>,
    /// Read-only document properties for the `Document Properties` overlay.
    pub properties: DocProperties,
    /// The merge source typed into the `Merge another PDF` field. Publication uses the
    /// framework save portal, so this never grants a write destination.
    pub merge_path: String,

    /// Undo/redo history for annotation and bookmark mutations, in memory only and
    /// cleared on document open and close (see [`crate::undo`]).
    pub undo_stack: UndoStack,

    pub zoom: crate::viewport::Zoom,
    pub page_view_mode: crate::viewport::PageViewMode,
    /// The rendered-page rotation, in quarter turns clockwise, applied through
    /// [`crate::viewport::page_box`].
    pub rotation: crate::viewport::PageRotation,
    /// Whether pages sit side by side as a book's facing pages. A reading
    /// preference, so it outlives the document; it engages only in a pane wide
    /// enough for two ([`crate::view::reading`]'s `spread_engaged`).
    pub spread_mode: crate::spread::SpreadMode,

    pub pdf_text: Vec<crate::pdf::PdfPageText>,
    /// The live PDF text selection, or `None` when nothing is selected.
    ///
    /// The PDF geometry and quoted text captured from the page. Both PDF
    /// annotation actions require it; comments collect their `/Contents`
    /// separately in the note composer.
    pub pdf_selection: Option<PdfSelection>,
    /// Whether the actions strip under the PDF selection is showing: from
    /// the end of a selecting gesture until it is used or dismissed.
    pub pdf_selection_actions: bool,
    /// The floating pager's page number while it is being typed; `None`
    /// shows the current page.
    pub page_entry: Option<String>,
    /// The creation and modification dates' picker and time-field state in
    /// the metadata editor.
    pub metadata_dates: [MetadataDateUi; 2],
    // Native PDF annotations and the page identities they were validated
    // against. Absent for EPUB/CBZ and while background import is pending.
    pub(crate) pdf_annotation_snapshot: Option<crate::plumbing::pdf_ops::PdfAnnotationSnapshot>,
    // Generation of the newest native-annotation scan or local PDF mutation.
    pub(crate) pdf_annotation_generation: u64,
    // Reconciliation result for each sidecar annotation id.
    pub(crate) pdf_annotation_states: HashMap<u64, PdfAnnotationState>,
    // Verified annotations' marks, by page, for the reading view to draw.
    pub(crate) pdf_overlay_annotations: HashMap<usize, Arc<Vec<PdfPageMark>>>,
    // Annotation selected from its page marker or accessible sidebar row.
    pub(crate) selected_annotation: Option<u64>,
    // Selection retained while the PDF comment composer collects authored text.
    pub(crate) pending_pdf_comment: Option<PdfSelection>,

    pub password_prompt: Option<PathBuf>,
    /// The password being typed: never logged, formatted or persisted (see
    /// `view::password_dialog`). Cleared on both exits of the dialog.
    pub password_input: String,
    /// Whether the secure field's reveal toggle is on. Purely visual; the
    /// value is withheld from the accessibility tree either way.
    pub password_revealed: bool,
    /// The unlock error shown on the field (`Wrong password. Try again.`).
    pub password_error: Option<String>,
    /// An unlock attempt is running off-thread; `Unlock` is disabled so a second press
    /// cannot start a second render.
    pub password_unlocking: bool,
    /// Generation counter for unlock attempts.
    ///
    /// Bumped on every submit and on cancel; an older `PasswordUnlockResult` is
    /// discarded, so a slow attempt cannot open behind a dismissed dialog.
    pub password_attempt: u64,

    pub note_editor: Option<enzui_iced_widgets::form_dialog::FormPresenter>,
    /// The `Protect with Password…` / `Remove Password…` dialog, while open.
    pub pdf_password_form: Option<PdfPasswordForm>,
    /// The sign flow, from its dialog through placing and writing the copy.
    pub signing: Option<crate::signing::SignDraft>,
    /// The id of the existing note the composer is editing, or `None` for a new
    /// note. Set when the composer opens on a persisted note so Save updates
    /// that note (id/page/anchor kept) instead of adding one.
    pub note_editor_editing: Option<u64>,
    /// Which page/chapter the composer will attach its note to.
    pub note_editor_page: usize,
    /// Durable passage anchor captured when the note composer was opened from
    /// a live EPUB selection. `None` for the ordinary current-location action.
    pub note_editor_position: Option<crate::plumbing::persistence::AnnotationPosition>,

    pub loading: bool,
    /// Last document or editing error surfaced to the user.
    pub error: Option<String>,
    /// A retained file-chooser failure shown as a bottom-right notice.
    pub picker_failure: Option<String>,
    /// Bounded safe summary of a retained persistence failure, separate from
    /// [`Self::error`] so its notice never shares a dismiss control.
    ///
    /// The [`enzui_iced_core::persist::PersistError::user_summary`] of the latest failure,
    /// with no paths or raw diagnostic.
    pub persistence_alert: Option<String>,
    /// The user closed the persistence notice; it stays hidden until the failure changes
    /// or a close is vetoed.
    pub persistence_alert_hidden: bool,
    /// Bounded safe-for-UI summary mirroring the App's live
    /// `crate::app::UncertainPublication` slot, published as its own
    /// banner with Retry / Dismiss / Keep Open controls. `None` when
    /// there is no uncertainty debt.
    pub pdf_mutation_uncertain_summary: Option<String>,
    /// Load failures whose original destinations remain write-protected.
    pub recovery_authorities: Vec<crate::plumbing::persistence::RecoveryAuthority>,
    /// Recovery location disclosed by the user's explicit action.
    pub revealed_recovery_location: Option<(crate::plumbing::persistence::StateOrigin, PathBuf)>,

    /// Whether the titlebar action bar's overflow menu is open. `enz_action_bar` publishes
    /// one toggle message and reads this back through `.overflow_open(..)`.
    ///
    /// Reset by [`Model::reset_document`] so a menu cannot survive into the next book.
    pub toolbar_overflow_open: bool,

    /// Whether the anchored reading-theme chooser popup is open.
    ///
    /// Reset by [`Model::reset_document`] so a menu cannot survive into the next book.
    pub reading_theme_menu_open: bool,
}

// Pending viewport settlement for one document-load generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PendingPositionRestore {
    Pages {
        generation: u64,
        page: usize,
        expected_offset: f32,
        operation_issued: bool,
        bootstrap_events_remaining: u8,
    },
    Epub {
        generation: u64,
        chapter: usize,
        anchor: crate::plumbing::persistence::EpubProgressAnchor,
        expected_fraction: f32,
        operation_issued: bool,
        bootstrap_events_remaining: u8,
    },
}

/// One metadata date's editing state: its calendar popover and the text in
/// its time field.
#[derive(Debug)]
pub struct MetadataDateUi {
    pub picker: enzui_iced_widgets::date_picker::DatePickerState,
    pub time_draft: String,
}

impl Default for MetadataDateUi {
    fn default() -> Self {
        let origin = enzui_iced_core::civil_date::Date::new(2000, 1, 1).expect("a valid date");
        Self {
            picker: enzui_iced_widgets::date_picker::DatePickerState::new(
                origin,
                enzui_iced_core::civil_date::Weekday::Monday,
                enzui_iced_core::civil_date::DateBounds::unbounded(),
            ),
            time_draft: String::new(),
        }
    }
}

/// The slot in [`Model::metadata_dates`] for a date field.
pub fn metadata_date_slot(field: crate::plumbing::pdf_ops::MetadataField) -> Option<usize> {
    use crate::plumbing::pdf_ops::MetadataField;
    match field {
        MetadataField::CreationDate => Some(0),
        MetadataField::ModificationDate => Some(1),
        _ => None,
    }
}

/// The time format the metadata time fields read and write.
pub fn metadata_time_format() -> enzui_iced_core::time_of_day::TimeFormat {
    enzui_iced_core::time_of_day::TimeFormat::new(enzui_iced_core::time_of_day::HourCycle::H23)
        .with_precision(enzui_iced_core::time_of_day::Precision::HourMinuteSecond)
}

/// A live text selection on a rendered PDF page.
///
/// Produced only by a real gesture over the page's glyph selection layer; nothing
/// constructs one from the current page.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfSelection {
    /// The 0-based page the selection lies on.
    pub page: usize,
    /// The selected glyphs, as indices into the page's selectable glyphs.
    pub range: std::ops::Range<usize>,
    /// The union of the selected text's boxes, in PDF user space (points,
    /// origin bottom-left) — the space `/Rect` is measured in.
    pub rect: crate::plumbing::pdf_ops::PdfRect,
    /// One box per selected line, in PDF user space: what a highlight
    /// paints (`/QuadPoints`), so a selection that starts or ends mid-line
    /// highlights only the words selected.
    pub quads: Vec<crate::plumbing::pdf_ops::PdfRect>,
    /// The selected text, lines joined by a newline.
    pub text: String,
}

// Whether a PDF sidecar annotation is safe to project onto the current page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PdfAnnotationState {
    Verified,
    VerifiedAfterDocumentChange,
    Unverified,
    Orphaned,
    Conflict,
}

/// Where a verified PDF annotation sits on its page and what the reading
/// view marks it with.
///
/// The page raster already shows a highlight's own colour; a note (a
/// comment, or a highlight with a note) also gets a marker in the margin
/// that shows the note on hover and opens it for editing.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageMark {
    pub id: u64,
    /// `[x, y, width, height]` as fractions of the page.
    pub rect: [f32; 4],
    /// The note's text, when the annotation carries one.
    pub note: Option<String>,
    /// Whether the annotation is a comment on a passage, whose words are
    /// tinted because the raster draws nothing for it.
    pub tints_text: bool,
    /// The annotation colour, as the PDF stores it (`/C`, 0 to 1).
    pub color: [f32; 3],
}

impl PdfAnnotationState {
    pub(crate) fn is_projectable(self) -> bool {
        matches!(self, Self::Verified | Self::VerifiedAfterDocumentChange)
    }
}

/// The parsed EPUB document plus per-chapter cached reading paragraphs.
pub struct EpubDoc {
    pub load: EpubLoadResult,
    /// Lazily built reading paragraphs for the current and adjacent chapters.
    pub chapter_paragraphs: HashMap<usize, Vec<ReadingParagraph>>,
    /// The chapter whose cached paragraphs carry the transient search highlight. The
    /// highlight is baked into the cache, so it is invalidated by chapter when the
    /// selected result moves or the search clears.
    pub highlighted_chapter: Option<usize>,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

mod lifecycle;
mod types;
pub use types::{
    ArrangementUncertainPublication, DOCUMENT_LOAD_ATTEMPT_EXHAUSTED, EpubViewMode,
    ExtractUncertainPublication, Format, LibraryHeightCache, LibraryProjection, LibrarySort,
    MetadataSaveFailure, Overlay, PageArrangementPublication, PasswordPurpose, PdfPasswordForm,
    PendingPageAction, READER_STATUS_COALESCE_KEY, ReadingTheme, ReadingThemeColors, Screen,
    SidebarTab, TOAST_ID_PREFIX, rgb,
};
mod library;
mod search;
