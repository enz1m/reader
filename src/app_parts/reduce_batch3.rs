//! Third reducer batch: EPUB scroll, navigation, bookmarks.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn batch3_epub_scrolled(&mut self, generation: u64, fraction: f32) -> Task<Msg> {
        self.apply_epub_scrolled(generation, fraction)
    }

    pub(crate) fn batch3_dismiss_error(&mut self) -> Task<Msg> {
        self.model.error = None;
        Task::none()
    }

    pub(crate) fn batch3_dismiss_picker_failure(&mut self) -> Task<Msg> {
        self.model.picker_failure = None;
        Task::none()
    }

    pub(crate) fn batch3_close_window(&mut self, window: Option<iced::window::Id>) -> Task<Msg> {
        self.begin_close_drain(window)
    }

    pub(crate) fn batch3_retry_close(&mut self) -> Task<Msg> {
        self.retry_close()
    }

    pub(crate) fn batch3_close_drain_tick(&mut self) -> Task<Msg> {
        self.check_close_drain_completion()
    }

    pub(crate) fn batch3_raster_band_refused_busy(
        &mut self,
        generation: u64,
        indices: Vec<usize>,
    ) -> Task<Msg> {
        if generation == self.model.raster_generation {
            // Busy release must not set `raster_failed`, which blanks pages permanently;
            // the coalesced busy-retry wake resubmits, since an immediate call would spin.
            for index in indices {
                self.model.rasterising.remove(&index);
            }
            self.arm_busy_retry();
        }
        Task::none()
    }

    pub(crate) fn batch3_pdf_thumbnails_refused_busy(
        &mut self,
        generation: u64,
        indices: Vec<usize>,
    ) -> Task<Msg> {
        if generation == self.model.thumbnail_generation {
            for page in indices {
                self.model.thumbnail_rasterising.remove(&page);
            }
            self.arm_busy_retry();
        }
        Task::none()
    }

    pub(crate) fn batch3_search_refused_busy(
        &mut self,
        generation: u64,
        format: Format,
    ) -> Task<Msg> {
        if generation == self.model.search_generation
            && self.model.format == format
            && self.model.search_in_flight_generation == Some(generation)
        {
            self.model.search_in_flight_generation = None;
            // The coalesced retry wake routes back through `Msg::SearchTick`, sharing
            // one bounded retry cadence.
            self.model.search_pending = true;
            self.arm_busy_retry();
        }
        Task::none()
    }

    pub(crate) fn batch3_recent_thumbnail_refused_busy(
        &mut self,
        path: String,
        source: crate::app::PreviewSource,
    ) -> Task<Msg> {
        // Requeue the (path, source) intent at the head so a saturated pool neither
        // settles the artwork as absent nor reorders it.
        if self.preview_in_flight.remove(&path)
            && !self.preview_pending.iter().any(|(p, _)| p == &path)
        {
            self.preview_pending.push_front((path, source));
            self.arm_busy_retry();
        }
        Task::none()
    }

    pub(crate) fn batch3_busy_retry_tick(&mut self) -> Task<Msg> {
        self.pump_busy_retry()
    }

    pub(crate) fn batch3_window(
        &mut self,
        command: enzui_iced_widgets::title_bar_gesture::WindowCommand,
    ) -> Task<Msg> {
        // `Close` goes through `Msg::CloseWindow(main_window)` so the async drain flushes
        // `save_position()` before teardown; other arms use `WindowCommand::into_task`.
        use enzui_iced_widgets::title_bar_gesture::WindowCommand;
        match command {
            WindowCommand::Close => self.update(Msg::CloseWindow(self.main_window)),
            other => iced::window::latest().then(move |maybe_id| match maybe_id {
                Some(id) => other.into_task(id),
                None => Task::none(),
            }),
        }
    }

    pub(crate) fn batch3_go_to_annotation(&mut self, id: u64) -> Task<Msg> {
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == id)
            .cloned()
        else {
            return Task::none();
        };
        self.model.selected_annotation = Some(id);
        if let Some(state) = self.model.pdf_annotation_states.get(&id).copied() {
            match state {
                    model::PdfAnnotationState::Orphaned => self.model.refuse(
                        "This annotation is orphaned because the PDF page changed; its sidecar marker is withheld.",
                    ),
                    model::PdfAnnotationState::Conflict => self.model.refuse(
                        "This annotation conflicts with the native PDF annotation; its sidecar marker is withheld.",
                    ),
                    model::PdfAnnotationState::Unverified => self.model.refuse(
                        "This annotation could not be verified against PDF text; its sidecar marker is withheld.",
                    ),
                    model::PdfAnnotationState::Verified
                    | model::PdfAnnotationState::VerifiedAfterDocumentChange => {}
                }
        }
        self.model.nav.dismiss_overlay();
        match persistence::AnnotationPosition::parse(&annotation.position, annotation.page) {
            persistence::AnnotationPosition::EpubCfi {
                chapter,
                char_start,
                ..
            } if chapter < self.model.epub_chapter_count() => {
                self.enter_chapter(chapter);
                let chapter_len = self.chapter_char_len();
                self.model.epub_page_offset = char_start.min(chapter_len.saturating_sub(1));
                self.repaginate();
                if self.model.epub_is_paginated() {
                    Task::none()
                } else {
                    let fraction = if chapter_len == 0 {
                        0.0
                    } else {
                        char_start as f32 / chapter_len as f32
                    };
                    self.model.epub_scroll_fraction = fraction.clamp(0.0, 1.0);
                    iced::widget::operation::snap_to(
                        crate::view::reading::epub_scroll_id(),
                        iced::widget::scrollable::RelativeOffset {
                            x: 0.0,
                            y: self.model.epub_scroll_fraction,
                        },
                        iced::advanced::widget::operation::Animation::Instant,
                    )
                }
            }
            persistence::AnnotationPosition::PdfRect { page, .. } => {
                self.update(Msg::GoToPage(page))
            }
            persistence::AnnotationPosition::None => match self.model.format {
                Format::Epub => self.update(Msg::GoToChapter(annotation.page)),
                Format::Pdf | Format::Cbz => self.update(Msg::GoToPage(annotation.page)),
            },
            // Out-of-range EPUB CFI chapter: fall back to the annotation's page.
            persistence::AnnotationPosition::EpubCfi { .. } => {
                self.update(Msg::GoToChapter(annotation.page))
            }
        }
    }

    pub(crate) fn batch3_remove_annotation(&mut self, id: u64) -> Task<Msg> {
        if self
            .model
            .annotations
            .annotations
            .iter()
            .any(|annotation| annotation.id == id)
        {
            // The confirmation replaces the note's open editor rather than stacking.
            let dismiss = if self.model.note_editor_editing == Some(id) {
                crate::overlay_focus::dismiss(
                    &mut self.model,
                    enzui_iced_core::modal::DismissCause::CloseButton,
                )
            } else {
                Task::none()
            };
            self.ask(model::PendingPageAction::DeleteAnnotation(id));
            return dismiss;
        }
        Task::none()
    }

    pub(crate) fn batch3_cycle_pdf_annotation_color(&mut self, id: u64) -> Task<Msg> {
        self.cycle_pdf_annotation_color(id).0
    }

    pub(crate) fn batch3_pdf_mutation_completed(
        &mut self,
        op_token: u64,
        document_generation: u64,
        intent: crate::app::PdfMutationIntent,
        outcome: Result<
            crate::plumbing::pdf_ops::PdfMutationOutcome,
            enzui_iced_core::blocking_task::BlockingTaskError,
        >,
    ) -> Task<Msg> {
        self.finish_pdf_mutation(op_token, document_generation, intent, outcome)
    }

    pub(crate) fn batch3_pdf_mutation_refused_busy(
        &mut self,
        op_token: u64,
        document_generation: u64,
        intent: crate::app::PdfMutationIntent,
    ) -> Task<Msg> {
        let matches_current = self.pdf_mutation_active.as_ref().is_some_and(|active| {
            active.op_token == op_token && active.document_generation == document_generation
        });
        if !matches_current {
            return Task::none();
        }
        self.pdf_mutation_active = None;
        // History intents never reach the worker, so a Busy for one is dropped.
        if intent.refuses_coalescing() {
            return Task::none();
        }
        if document_generation == self.model.pdf_annotation_generation
            && self.model.file_path.as_deref() == Some(intent.path())
            && self.model.format == Format::Pdf
        {
            // Retain the busied active as the next slot without touching a newer pending.
            self.pdf_mutation_busy_retry = Some(PdfMutationPending {
                document_generation,
                intent,
            });
            self.arm_busy_retry();
        }
        Task::none()
    }

    pub(crate) fn batch3_retry_pdf_mutation_uncertain(&mut self) -> Task<Msg> {
        self.retry_pdf_mutation_uncertain()
    }

    pub(crate) fn batch3_pdf_mutation_uncertain_retried(
        &mut self,
        op_token: u64,
        retry_sequence: u64,
        outcome: Result<
            crate::app::PdfReconciliationClass,
            enzui_iced_core::blocking_task::BlockingTaskError,
        >,
    ) -> Task<Msg> {
        self.finish_pdf_mutation_uncertain_retry(op_token, retry_sequence, outcome)
    }

    pub(crate) fn batch3_dismiss_pdf_mutation_uncertain_warning(&mut self) -> Task<Msg> {
        // Hides only the banner: the record stays on `pdf_mutation_uncertain` across
        // document switches and keeps refusing admits and vetoing close. Retained
        // intents were admitted against pre-uncertainty bytes, so each is refused
        // visibly.
        if self.pdf_mutation_uncertain.is_some() {
            let cancelled_slots = usize::from(self.pdf_mutation_busy_retry.is_some())
                + usize::from(self.pdf_mutation_pending.is_some());
            self.pdf_mutation_busy_retry = None;
            self.pdf_mutation_pending = None;
            if let Some(record) = self.pdf_mutation_uncertain.as_mut() {
                record.banner_visible = false;
            }
            self.model.pdf_mutation_uncertain_summary = None;
            for _ in 0..cancelled_slots {
                self.model
                    .refuse(RETAINED_SLOT_CANCELLED_BY_DISMISS_SUMMARY);
            }
            self.model.announce(UNCERTAIN_DISMISSED_SUMMARY);
        }
        Task::none()
    }

    pub(crate) fn batch3_dismiss_arrangement_uncertain_publication(&mut self) -> Task<Msg> {
        // Dropping the retained authority releases its descriptor; the destination may
        // already hold the arranged bytes.
        if self
            .model
            .arrangement_uncertain_publication
            .take()
            .is_some()
        {
            self.model.error = None;
        }
        self.arrangement_uncertain_authority = None;
        Task::none()
    }

    pub(crate) fn batch3_dismiss_extract_uncertain_publication(&mut self) -> Task<Msg> {
        // Explicit abandonment drops the retained descriptors and leaves on-disk outputs
        // as they are; close no longer vetoes on this record.
        if self.model.extract_uncertain_publication.take().is_some() {
            self.model.error = None;
        }
        self.extract_publication_debt = None;
        Task::none()
    }

    pub(crate) fn batch3_toggle_current_bookmark(&mut self) -> Task<Msg> {
        let page = self.model.current_position();
        self.update(Msg::ToggleBookmark(page))
    }

    pub(crate) fn batch3_toggle_bookmark(&mut self, page: usize) -> Task<Msg> {
        let label = match self.model.format {
            Format::Epub => format!("Chapter {}", page + 1),
            Format::Pdf | Format::Cbz => format!("Page {}", page + 1),
        };
        // Capture the removed record first so undoing a removal restores its label and
        // timestamp.
        let existing = self
            .model
            .annotations
            .bookmarks
            .iter()
            .find(|b| b.page == page)
            .cloned();
        let added = persistence::toggle_bookmark(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            page,
            &label,
        );
        if added {
            // One bookmark per page: capture the inserted one so redo restores its stamp.
            if let Some(record) = self
                .model
                .annotations
                .bookmarks
                .iter()
                .find(|b| b.page == page)
                .cloned()
            {
                self.model
                    .undo_stack
                    .record(UndoableAction::AddBookmark { record });
            }
        } else if let Some(record) = existing {
            self.model
                .undo_stack
                .record(UndoableAction::RemoveBookmark { record });
        }
        self.save_annotations();
        Task::none()
    }

    pub(crate) fn batch3_go_to_bookmark(
        &mut self,
        format: Format,
        document_key: String,
        position: usize,
    ) -> Task<Msg> {
        let current_key = self
            .model
            .file_path
            .as_ref()
            .map(|path| persistence::canonical_key(&path.to_string_lossy()));
        if format != self.model.format || current_key.as_deref() != Some(&document_key) {
            return Task::none();
        }
        match format {
            Format::Epub => self.update(Msg::GoToChapter(position)),
            Format::Pdf | Format::Cbz => self.update(Msg::GoToPage(position)),
        }
    }
}
