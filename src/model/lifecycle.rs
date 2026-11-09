//! Model construction and document lifecycle.

use super::*;

impl Model {
    pub fn new() -> Self {
        Self {
            screen: Screen::Library,
            file_path: None,
            file_name: String::new(),
            display_title: String::new(),
            format: Format::Pdf,
            page_count: 0,
            total_page_count: 0,
            current_page: 0,
            epub: None,
            epub_current_chapter: 0,
            active_footnote: None,
            epub_font_size: 18,
            epub_line_spacing: 1.5,
            reading_theme: ReadingTheme::Light,
            epub_view_mode: EpubViewMode::Scrolled,
            epub_scroll_fraction: 0.0,
            epub_page_offset: 0,
            epub_pages: Vec::new(),
            signing: None,
            epub_page_box: None,
            epub_page_wheel: 0.0,
            epub_page_content: Vec::new(),
            session_start_secs: 0,
            pages: crate::raster::PageRasterCache::default(),
            cbz_entries: Arc::new(Vec::new()),
            page_extents: Vec::new(),
            pages_scroll_offset: 0.0,
            pages_viewport_height: 0.0,
            pages_viewport_width: 0.0,
            rasterising: std::collections::BTreeSet::new(),
            raster_failed: std::collections::BTreeSet::new(),
            raster_generation: 0,
            thumbnail_rasters: crate::raster::PageRasterCache::with_budget(
                crate::raster::THUMBNAIL_BUDGET_BYTES,
            ),
            thumbnail_scroll_offset: 0.0,
            thumbnail_viewport_height: 0.0,
            thumbnail_rasterising: std::collections::BTreeSet::new(),
            thumbnail_failed: std::collections::BTreeSet::new(),
            thumbnail_generation: 0,
            pdf_outline: Vec::new(),
            pdf_navigation_loading: false,
            pdf_navigation_error: None,
            pdf_outline_error: None,
            pdf_navigation_truncated: false,
            pdf_navigation_generation: 0,
            document_load_attempt: 0,
            pdf_password: None,
            position_restore_generation: 0,
            pending_position_restore: None,
            app_state: Arc::new(AppState::default()),
            library_loading: false,
            library_persistence_unavailable: false,
            recents: Vec::new(),
            library: Vec::new(),
            library_projection: LibraryProjection::default(),
            library_revision: 0,
            order_revision: 0,
            library_height_cache: LibraryHeightCache::default(),
            library_scroll_offset: 0.0,
            library_viewport_height: 0.0,
            library_query: String::new(),
            library_sort: LibrarySort::Recent,
            library_sort_menu_model: enzui_iced_core::menu_model::MenuModel::new(
                LibrarySort::menu_entries(LibrarySort::Recent),
            ),
            library_sort_menu: enzui_iced_core::menu_model::MenuState::closed(),
            availability: std::collections::HashMap::new(),
            bookmark_counts: std::collections::HashMap::new(),
            recent_thumbnails: std::collections::HashMap::new(),
            annotations: Arc::new(AnnotationsFile::default()),
            annotations_loading: false,
            annotations_unavailable: false,
            sidebar_tab: SidebarTab::Contents,
            annotation_filter: String::new(),
            annotation_tools_expanded: false,
            active_selection: None,
            selection_clear_generation: 0,
            editing_annotation: None,
            toasts: ToastHost::new()
                .id_prefix(TOAST_ID_PREFIX)
                // One visible slot and no waiting budget make the channel key a
                // replacement, so a burst never queues a backlog.
                .max_visible(1)
                .waiting_budget(WaitingBudget::unbounded().max_waiting(Some(0))),
            toast_expiry: ToastExpirySchedule::default(),
            search_query: String::new(),
            search_scroll: Default::default(),
            search_hits: Vec::new(),
            search_active: 0,
            search_index: crate::search::SearchIndex::default(),
            pdf_search_index: crate::search::PdfSearchIndex::default(),
            pdf_search_hits: Vec::new(),
            pdf_search_truncated: false,
            epub_search_truncated: false,
            search_generation: 0,
            search_in_flight_generation: None,
            search_pending: false,
            nav: NavSplitState::new(),
            overlay: None,
            dialogs: Default::default(),
            context_menu: None,
            context_menu_generation: 0,
            closed_context_menu: Default::default(),
            arrangement: PageArrangement::default(),
            arrangement_publish_generation: 0,
            arrangement_publication: None,
            arrangement_publish_failure: None,
            arrangement_uncertain_publication: None,
            extract_uncertain_publication: None,
            pending_page_action: None,
            page_confirm_state: ConfirmState::new(None),
            metadata: PdfMetadata::default(),
            metadata_baseline: PdfMetadata::default(),
            metadata_save_failure: None,
            properties: DocProperties::default(),
            merge_path: String::new(),
            undo_stack: UndoStack::new(),
            zoom: crate::viewport::Zoom::default(),
            page_view_mode: crate::viewport::PageViewMode::default(),
            rotation: crate::viewport::PageRotation::NONE,
            spread_mode: crate::spread::SpreadMode::Single,
            pdf_text: Vec::new(),
            pdf_selection: None,
            pdf_selection_actions: false,
            page_entry: None,
            epub_scroll_offset: 0.0,
            epub_viewport_height: 0.0,
            epub_geometry: None,
            epub_geometry_pending: None,
            arranger_selection: Vec::new(),
            arranger_scroll_offset: 0.0,
            arranger_viewport_height: 0.0,
            epub_geometry_estimate: std::cell::RefCell::new(None),
            metadata_dates: Default::default(),
            pdf_annotation_snapshot: None,
            pdf_annotation_generation: 0,
            pdf_annotation_states: HashMap::new(),
            pdf_overlay_annotations: HashMap::new(),
            selected_annotation: None,
            pending_pdf_comment: None,
            password_prompt: None,
            password_input: String::new(),
            password_revealed: false,
            password_error: None,
            password_unlocking: false,
            password_attempt: 0,
            note_editor: None,
            pdf_password_form: None,
            note_editor_editing: None,
            note_editor_page: 0,
            note_editor_position: None,
            loading: false,
            error: None,
            picker_failure: None,
            persistence_alert: None,
            persistence_alert_hidden: false,
            pdf_mutation_uncertain_summary: None,
            recovery_authorities: Vec::new(),
            revealed_recovery_location: None,
            toolbar_overflow_open: false,
            reading_theme_menu_open: false,
        }
    }

    /// Announce an advisory acknowledgement such as a completed copy or page move.
    /// The line clears itself after its dwell.
    pub fn announce(&mut self, message: impl Into<String>) {
        self.push_status(message, ToastTone::Info);
    }

    /// Announce a refusal assertively, since the requested action did not happen.
    /// Durable failures with their own retry surface stay on their banners.
    pub fn refuse(&mut self, message: impl Into<String>) {
        self.push_status(message, ToastTone::Warning);
    }

    /// Record whether the library's state can be saved; with `announce`, warn once as it
    /// becomes impossible rather than holding a banner for the session.
    pub fn set_library_persistence_unavailable(&mut self, unavailable: bool, announce: bool) {
        if announce && unavailable && !self.library_persistence_unavailable {
            self.push_toast(
                "Reading progress won't be saved",
                ToastTone::Warning,
                |spec| {
                    spec.description("Reader could not open its private state folder.")
                        .duration(std::time::Duration::from_secs(10))
                },
            );
        }
        self.library_persistence_unavailable = unavailable;
    }

    // The single push all transient status shares. The channel key coalesces
    // a newer line onto the one the reader is currently shown, so repeated
    // acknowledgements replace in place instead of stacking.
    fn push_status(&mut self, message: impl Into<String>, tone: ToastTone) {
        self.push_toast(message, tone, |spec| {
            spec.coalesce_key(READER_STATUS_COALESCE_KEY)
        });
    }

    // The one place a toast is built; `finish` adds what this kind needs.
    fn push_toast(
        &mut self,
        message: impl Into<String>,
        tone: ToastTone,
        finish: impl FnOnce(ToastSpec) -> ToastSpec,
    ) {
        self.toasts.push(finish(ToastSpec::new(message).tone(tone)));
    }

    /// Renew the whole-document load attempt, so every completion issued
    /// before this call is superseded.
    ///
    /// Saturating, never wrapping: a reused id could admit an ancient completion as
    /// current. Past [`DOCUMENT_LOAD_ATTEMPT_EXHAUSTED`], which
    /// [`Self::admits_document_load`] never admits, loads stop landing.
    pub fn renew_document_load_attempt(&mut self) {
        self.document_load_attempt = self.document_load_attempt.saturating_add(1);
    }

    /// Whether a document-load completion stamped `attempt` is still the one
    /// the reader is waiting for.
    ///
    /// The reserved exhausted id is admitted for nothing, so an id that could
    /// have been issued twice can never identify a completion.
    #[must_use]
    pub fn admits_document_load(&self, attempt: u64) -> bool {
        attempt != DOCUMENT_LOAD_ATTEMPT_EXHAUSTED && attempt == self.document_load_attempt
    }

    /// Reset per-document view state before opening a new file.
    /// Re-seed the metadata date pickers and time fields from the current
    /// values: when the editor opens, and after a date is removed.
    pub fn reset_metadata_date_ui(&mut self) {
        use crate::plumbing::pdf_ops::{MetadataField, pdf_date_parts};
        for field in [MetadataField::CreationDate, MetadataField::ModificationDate] {
            let Some(slot) = metadata_date_slot(field) else {
                continue;
            };
            let parts = pdf_date_parts(field.get(&self.metadata));
            let ui = &mut self.metadata_dates[slot];
            *ui = MetadataDateUi::default();
            if let Some((date, time, _)) = parts {
                ui.picker = enzui_iced_widgets::date_picker::DatePickerState::new(
                    date,
                    enzui_iced_core::civil_date::Weekday::Monday,
                    enzui_iced_core::civil_date::DateBounds::unbounded(),
                );
                ui.time_draft = metadata_time_format().format(time);
            }
        }
    }

    /// What the current chapter's geometry is keyed by at `column_width`,
    /// or `None` when scroll mode builds the chapter whole (paginated mode,
    /// or a chapter short enough to lay out at once).
    pub fn epub_geometry_key(&self, column_width: f32) -> Option<crate::reading_pane::GeometryKey> {
        if self.format != Format::Epub || self.epub_is_paginated() {
            return None;
        }
        let chapter = self.epub_current_chapter;
        let paragraphs = self.epub.as_ref()?.chapter_paragraphs.get(&chapter)?;
        (paragraphs.len() >= crate::reading_pane::WINDOW_MIN_PARAGRAPHS).then(|| {
            crate::reading_pane::GeometryKey::new(
                self.document_load_attempt,
                chapter,
                paragraphs.len(),
                self.epub_font_size as f32,
                self.epub_line_spacing,
                column_width,
            )
        })
    }

    /// The paragraphs scroll mode builds for the current chapter at
    /// `column_width`, or `None` to build it whole.
    pub fn epub_paragraph_window(
        &self,
        column_width: f32,
    ) -> Option<crate::reading_pane::ParagraphWindow> {
        use crate::reading_pane::ChapterGeometry;
        let key = self.epub_geometry_key(column_width)?;
        let geometry = match &self.epub_geometry {
            Some(geometry) if geometry.key() == key => geometry.clone(),
            _ => {
                let mut cache = self.epub_geometry_estimate.borrow_mut();
                match cache.as_ref() {
                    Some(geometry) if geometry.key() == key => geometry.clone(),
                    _ => {
                        let paragraphs =
                            self.epub.as_ref()?.chapter_paragraphs.get(&key.chapter)?;
                        let geometry = std::sync::Arc::new(ChapterGeometry::estimate(
                            key,
                            paragraphs.iter().map(|para| para.text.as_str()),
                            self.epub_font_size as f32,
                            self.epub_line_spacing,
                            column_width,
                        ));
                        *cache = Some(geometry.clone());
                        geometry
                    }
                }
            }
        };
        // Before the first scroll report the viewport is unknown; a tall
        // guess only realises a few more paragraphs.
        let viewport = if self.epub_viewport_height > 0.0 {
            self.epub_viewport_height
        } else {
            1600.0
        };
        // The prose starts below the pane's padding and the chapter heading.
        let prose_top = 48.0 + self.epub_font_size as f32 * 4.0;
        let offset = (self.epub_scroll_offset - prose_top).max(0.0);
        Some(geometry.window(offset, viewport, viewport))
    }

    /// The sidebar view a document opens on: its pages for a paged document
    /// (PDF, comic), its table of contents for a book.
    pub fn default_sidebar_tab(&self) -> SidebarTab {
        match self.format {
            Format::Pdf | Format::Cbz => SidebarTab::Thumbnails,
            Format::Epub => SidebarTab::Contents,
        }
    }

    pub fn reset_document(&mut self) {
        // Cleared with the rest of the document, or the previous
        // book's title would head the next one until its metadata arrived.
        self.display_title.clear();
        self.epub = None;
        self.epub_current_chapter = 0;
        self.active_footnote = None;
        self.epub_scroll_fraction = 0.0;
        // The page boundaries and the anchored offset belong to the
        // document being closed. Carrying them into the next book would open it
        // on "page 12" of a chapter that may not have twelve pages.
        self.epub_page_offset = 0;
        self.epub_pages.clear();
        self.epub_page_content.clear();
        self.pages.clear();
        self.cbz_entries = Arc::new(Vec::new());
        self.page_extents.clear();
        self.pages_scroll_offset = 0.0;
        self.pages_viewport_height = 0.0;
        self.pages_viewport_width = 0.0;
        self.rasterising.clear();
        self.raster_failed.clear();
        // Any band still in flight belongs to the document being closed.
        self.raster_generation = self.raster_generation.wrapping_add(1);
        self.thumbnail_rasters.clear();
        self.thumbnail_scroll_offset = 0.0;
        self.thumbnail_viewport_height = 0.0;
        self.thumbnail_rasterising.clear();
        self.thumbnail_failed.clear();
        self.thumbnail_generation = self.thumbnail_generation.wrapping_add(1);
        self.pdf_outline.clear();
        self.pdf_navigation_loading = false;
        self.pdf_navigation_error = None;
        self.pdf_outline_error = None;
        self.pdf_navigation_truncated = false;
        self.pdf_navigation_generation = self.pdf_navigation_generation.wrapping_add(1);
        // A parse still running belongs to the document being replaced.
        self.renew_document_load_attempt();
        // The session key dies with the document it unlocked.
        self.pdf_password = None;
        self.position_restore_generation = self.position_restore_generation.wrapping_add(1);
        self.pending_position_restore = None;
        self.current_page = 0;
        self.page_count = 0;
        self.total_page_count = 0;
        self.annotations = Arc::new(AnnotationsFile::default());
        self.annotations_loading = false;
        self.annotations_unavailable = false;
        self.recovery_authorities.retain(|authority| {
            authority.origin == crate::plumbing::persistence::StateOrigin::Library
        });
        self.revealed_recovery_location = None;
        self.pdf_annotation_snapshot = None;
        self.pdf_annotation_generation = self.pdf_annotation_generation.wrapping_add(1);
        self.pdf_annotation_states.clear();
        self.pdf_overlay_annotations.clear();
        self.selected_annotation = None;
        self.pending_pdf_comment = None;
        self.annotation_filter.clear();
        self.annotation_tools_expanded = false;
        self.active_selection = None;
        // The selection scope also owns interior selection state.
        // Bumping the counter across document boundaries drops that state,
        // matching the app-side `active_selection = None` above.
        self.selection_clear_generation = self.selection_clear_generation.wrapping_add(1);
        self.editing_annotation = None;
        // Transient acknowledgements live in the shared status host and
        // self-expire; a document reset does not need to clear them.
        self.search_query.clear();
        self.search_hits.clear();
        self.search_active = 0;
        self.search_index = crate::search::SearchIndex::default();
        self.pdf_search_index = crate::search::PdfSearchIndex::default();
        self.pdf_search_hits.clear();
        self.pdf_search_truncated = false;
        self.epub_search_truncated = false;
        self.search_pending = false;
        self.search_in_flight_generation = None;
        // Bump the generation so any in-flight scan from the previous document
        // is discarded when it returns (its generation will be stale).
        self.search_generation = self.search_generation.wrapping_add(1);
        drop(crate::overlay_focus::close_for_host(self));
        // A menu raised on the previous document's annotation must not survive
        // into the next one: its target names an id the new sidecar does not
        // have, so it would be a menu over nothing.
        self.context_menu = None;
        self.arrangement = PageArrangement::default();
        self.arrangement_publish_generation = self.arrangement_publish_generation.wrapping_add(1);
        self.arrangement_publication = None;
        self.arrangement_publish_failure = None;
        self.pending_page_action = None;
        self.page_confirm_state = ConfirmState::new(None);
        self.metadata = PdfMetadata::default();
        self.metadata_baseline = PdfMetadata::default();
        self.metadata_save_failure = None;
        self.properties = DocProperties::default();
        self.merge_path.clear();
        // Zoom and rotation are per-document view state: carrying a 300% zoom
        // and a 90° turn into the next book would open it sideways.
        self.zoom = crate::viewport::Zoom::default();
        self.page_view_mode = crate::viewport::PageViewMode::default();
        self.rotation = crate::viewport::PageRotation::NONE;
        // A selection belongs to its page; carried over it would re-arm Highlight with
        // another book's rectangle.
        self.pdf_text.clear();
        self.pdf_selection = None;
        self.clear_password();
        self.note_editor = None;
        self.note_editor_editing = None;
        self.note_editor_page = 0;
        self.note_editor_position = None;
        // Undo history does not cross documents.
        self.undo_stack.clear();
        self.error = None;
        // An open overflow menu may name commands the next format lacks.
        self.toolbar_overflow_open = false;
        // Likewise close the reading-theme chooser popup.
        self.reading_theme_menu_open = false;
    }
}
