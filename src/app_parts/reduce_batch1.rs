//! First reducer batch: theme, toast, opens, windows.

use super::*;

use iced::Task;

use crate::message::{PageBitmap, PickOutcome};
use enzui_iced_core::app_runtime::AccessibilityWake;

impl App {
    pub(crate) fn batch1_theme(
        &mut self,
        event: enzui_iced_core::theme_file::integration::ThemeEvent,
    ) -> Task<Msg> {
        let before = self.themes.theme_generation();
        let task = self.themes.update(event);
        debug_assert!(self.themes.theme_generation() >= before);
        task.map(|event| Msg::Theme(Box::new(event)))
    }

    pub(crate) fn batch1_toast(
        &mut self,
        event: enzui_iced_widgets::toast_host::ToastEvent,
    ) -> Task<Msg> {
        // Expiry ticks, dismissals and toast actions reduce into the host; `update`
        // reconciles the wake afterwards.
        self.model.toasts.update(event);
        Task::none()
    }

    pub(crate) fn batch1_a11y_wake(&mut self, receipt: AccessibilityWake) -> Task<Msg> {
        // The drain at the top of `update` may already have consumed this batch, in which
        // case nothing is applied.
        enzui_iced_widgets::a11y_actions::apply_accessibility_wake::<Msg>(receipt)
    }

    pub(crate) fn batch1_open_path(&mut self, path: PathBuf) -> Task<Msg> {
        self.open(path)
    }

    pub(crate) fn batch1_open_dialog(&mut self) -> Task<Msg> {
        self.save_arrangement_pick_source = None;
        // A request that never starts is a terminal outcome routed through `finish_pick`,
        // like a late reply, so the user gets an actionable error.
        if let Some(window) = self.main_window {
            return self.begin_parented_pick(crate::picker::PickerTarget::OpenBook, window);
        }
        self.open_pick_unparented(crate::picker::PickerTarget::OpenBook)
    }

    pub(crate) fn batch1_main_window_ready(&mut self, id: Option<iced::window::Id>) -> Task<Msg> {
        self.main_window = id;
        Task::none()
    }

    pub(crate) fn batch1_main_window_closed(&mut self) -> Task<Msg> {
        // Reader has one window, so closure invalidates every pending parent export.
        self.picker
            .cancel(enzui_iced_core::portal::CancelReason::ParentClosed);
        self.main_window = None;
        self.save_arrangement_pick_source = None;
        if let Some(pick) = self.pending_pick.take() {
            pick.cancel
                .cancel(enzui_iced_core::portal::CancelReason::ParentClosed);
        }
        Task::none()
    }

    pub(crate) fn batch1_picker_parent_resolved(
        &mut self,
        generation: u64,
        target: crate::picker::PickerTarget,
        window: iced::window::Id,
        payload: enzui_iced_parent_lease::Handoff<Option<Box<dyn std::any::Any + Send>>>,
    ) -> Task<Msg> {
        self.reduce_pick_parent_resolved(generation, target, window, &payload)
    }

    pub(crate) fn batch1_pick_resolved(
        &mut self,
        generation: u64,
        target: crate::picker::PickerTarget,
        window: iced::window::Id,
        outcome: PickOutcome,
    ) -> Task<Msg> {
        self.reduce_pick_resolved(generation, target, window, outcome)
    }

    pub(crate) fn batch1_portal_tick(&mut self) -> Task<Msg> {
        if let Some((target, outcome)) = self.picker.pump() {
            return self.finish_pick(target, outcome);
        }
        Task::none()
    }

    pub(crate) fn batch1_go_library(&mut self) -> Task<Msg> {
        // Snapshot pending persistence without writing it on this click.
        let persistence = self.flush_persistence();
        // Rebuild a stale projection so the scan snapshots the current order; no `stat()`.
        self.ensure_projection_fresh();
        let availability = self.request_availability_scan();
        self.model.screen = Screen::Library;
        // `.focus_on_mount(true)` on the library's `enz_search_field` handles focus.
        Task::batch([persistence, availability])
    }

    pub(crate) fn batch1_library_query_changed(&mut self, q: String) -> Task<Msg> {
        self.model.library_query = q;
        self.model.rebuild_library_projection();
        Task::none()
    }

    pub(crate) fn batch1_set_library_sort(&mut self, sort: crate::model::LibrarySort) -> Task<Msg> {
        self.model.library_sort = sort;
        self.model
            .library_sort_menu_model
            .reconcile(crate::model::LibrarySort::menu_entries(sort));
        self.model.rebuild_library_projection();
        Task::none()
    }

    pub(crate) fn batch1_library_sort_menu(
        &mut self,
        event: enzui_iced_widgets::menu_button::MenuButtonEvent,
    ) -> Task<Msg> {
        let picked = enzui_iced_widgets::menu_button::apply_event(
            &mut self.model.library_sort_menu_model,
            &mut self.model.library_sort_menu,
            &event,
        );
        match picked.and_then(|id| crate::model::LibrarySort::from_menu_id(id.as_str())) {
            Some(sort) => self.batch1_set_library_sort(sort),
            None => Task::none(),
        }
    }

    pub(crate) fn batch1_library_scrolled(
        &mut self,
        offset: f32,
        viewport_height: f32,
    ) -> Task<Msg> {
        self.model.library_scroll_offset = offset.max(0.0);
        self.model.library_viewport_height = viewport_height.max(0.0);
        Task::none()
    }

    pub(crate) fn batch1_document_loaded(
        &mut self,
        attempt: u64,
        path: PathBuf,
        format: Format,
        outcome: LoadOutcome,
    ) -> Task<Msg> {
        // A superseded load would install its content under the newer document's
        // `file_path`, so it is rejected before any state moves.
        if !self.model.admits_document_load(attempt) {
            return self.resume_pending_document_load(Task::none());
        }
        self.model.loading = false;
        let task = self.finish_document_load(path, format, outcome);
        let fonts = self.warm_signature_fonts(format);
        self.resume_pending_document_load(Task::batch([task, fonts]))
    }

    // Find the signature fonts off the UI thread the first time a PDF
    // opens, so `Sign…` opens at once even with thousands installed.
    fn warm_signature_fonts(&self, format: Format) -> Task<Msg> {
        static WARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if format != Format::Pdf || WARMED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return Task::none();
        }
        self.dispatch_blocking(|| {
            let _ = crate::signing::installed_fonts();
        })
        .map(|_| Msg::Noop)
    }

    pub(crate) fn batch1_annotations_restored(
        &mut self,
        generation: u64,
        base_revision: u64,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    ) -> Task<Msg> {
        self.apply_annotation_load_settlement(generation, base_revision, settlement)
    }

    pub(crate) fn batch1_pdf_navigation_loaded(
        &mut self,
        generation: u64,
        outcome: Result<crate::plumbing::pdf_navigation::PdfNavigationData, String>,
    ) -> Task<Msg> {
        if generation != self.model.pdf_navigation_generation
            || self.model.format != Format::Pdf
            || self.model.screen != Screen::Reading
        {
            return Task::none();
        }
        self.model.pdf_navigation_loading = false;
        match outcome {
            Ok(data) => {
                if data.page_count != self.model.page_count {
                    self.model.pdf_navigation_error =
                        Some("The PDF changed while its navigation data was loading.".into());
                    self.model.pdf_outline_error = self.model.pdf_navigation_error.clone();
                    self.model.pdf_outline.clear();
                    self.model.pdf_search_index = crate::search::PdfSearchIndex::default();
                    self.model.pdf_search_hits.clear();
                    return Task::none();
                }
                self.model.pdf_navigation_error = None;
                self.model.pdf_navigation_truncated = data.outline_truncated;
                self.model.pdf_search_truncated = data.text_truncated;
                self.model.pdf_outline_error = data.outline_error.map(|diagnostic| {
                    eprintln!("reader-app PDF outline load failure: diagnostic={diagnostic:?}");
                    "The PDF outline could not be loaded.".to_string()
                });
                self.model.pdf_outline = data.outline;
                for (page, text) in data.text.iter().cloned().enumerate() {
                    if let Some(slot) = self.model.pdf_text.get_mut(page) {
                        slot.absorb_search_text(text);
                    }
                }
                self.model.pdf_search_index = crate::search::PdfSearchIndex::build(data.text);
                if !self.model.annotations_loading
                    && let Some(snapshot) = self.model.pdf_annotation_snapshot.clone()
                {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
                if !self.model.search_query.trim().is_empty() {
                    let query = self.model.search_query.clone();
                    let _ = self.model.begin_search(query);
                    return Task::done(Msg::SearchTick);
                }
            }
            Err(diagnostic) => {
                eprintln!("reader-app PDF navigation load failure: diagnostic={diagnostic:?}");
                let summary = "PDF navigation could not be loaded.".to_string();
                self.model.pdf_outline_error = Some(summary.clone());
                self.model.pdf_navigation_error = Some(summary);
                self.model.pdf_outline.clear();
                self.model.pdf_search_index = crate::search::PdfSearchIndex::default();
                self.model.pdf_search_hits.clear();
                self.model.pdf_search_truncated = false;
            }
        }
        Task::none()
    }

    pub(crate) fn batch1_native_pdf_annotations_loaded(
        &mut self,
        generation: u64,
        outcome: Result<crate::plumbing::pdf_ops::PdfAnnotationSnapshot, String>,
    ) -> Task<Msg> {
        if generation != self.model.pdf_annotation_generation
            || self.model.format != Format::Pdf
            || self.model.screen != Screen::Reading
        {
            return Task::none();
        }
        match outcome {
            Ok(snapshot) if self.model.annotations_loading => {
                // Initial load: reconcile runs later against the full state.
                self.model.pdf_annotation_snapshot = Some(snapshot);
            }
            Ok(snapshot) => {
                // A pathname reopen is advisory and never clears a reconciliation record;
                // only [`Self::retry_pdf_mutation_uncertain`] can, from retained bytes.
                self.reconcile_native_pdf_annotations(snapshot);
            }
            Err(_) => self
                .model
                .refuse("Native PDF annotations could not be loaded."),
        }
        Task::none()
    }

    pub(crate) fn batch1_next_chapter(&mut self) -> Task<Msg> {
        // Page over content chapters, never onto excluded front matter.
        if let Some(next) = self.model.epub_next_content_chapter() {
            self.enter_chapter(next);
            self.save_position();
        }
        Task::none()
    }

    pub(crate) fn batch1_prev_chapter(&mut self) -> Task<Msg> {
        // Previous stops at the first content chapter.
        if let Some(prev) = self.model.epub_prev_content_chapter() {
            self.enter_chapter(prev);
            self.save_position();
        }
        Task::none()
    }

    pub(crate) fn batch1_go_to_chapter(&mut self, idx: usize) -> Task<Msg> {
        if idx < self.model.epub_chapter_count() {
            self.enter_chapter(idx);
            self.save_position();
        }
        Task::none()
    }

    pub(crate) fn batch1_go_to_page(&mut self, target: usize) -> Task<Msg> {
        if !matches!(self.model.format, Format::Pdf | Format::Cbz)
            || target >= self.model.page_count
        {
            return Task::none();
        }
        self.model.current_page = target;
        self.save_position();
        let y = self.page_offset_of(target);
        // A jump is unbounded in distance, so request its band from the destination offset
        // now rather than waiting for the scroll report.
        self.model.pages_scroll_offset = y;
        // Move the real viewport; `current_page` alone would only repaint the pager.
        iced::widget::operation::scroll_to(
            crate::view::reading::pages_scroll_id(),
            iced::widget::scrollable::AbsoluteOffset { x: 0.0, y },
            iced::advanced::widget::operation::Animation::Instant,
        )
    }

    pub(crate) fn batch1_begin_position_restore(&mut self, generation: u64) -> Task<Msg> {
        self.begin_position_restore(generation)
    }

    pub(crate) fn batch1_pages_scrolled(
        &mut self,
        generation: u64,
        offset: f32,
        viewport_height: f32,
        viewport_width: f32,
    ) -> Task<Msg> {
        self.apply_pages_scrolled(generation, offset, viewport_height, viewport_width)
    }

    pub(crate) fn batch1_pages_rasterised(
        &mut self,
        generation: u64,
        requested: Vec<usize>,
        pages: Vec<PageBitmap>,
        text: Vec<crate::pdf::PdfPageText>,
    ) -> Task<Msg> {
        let swaps = self.finish_rasterised_band(generation, requested, pages, text);
        Task::batch([swaps, self.pump_busy_retry()])
    }
}
