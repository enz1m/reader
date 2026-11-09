//! Document opening and position restore.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn open(&mut self, path: PathBuf) -> Task<Msg> {
        self.model.picker_failure = None;
        let Some(format) = services::detect_format(&path) else {
            self.model.error = Some(format!(
                "Unsupported file type: {}",
                crate::user_copy::path_name(&path)
            ));
            return Task::none();
        };
        // Snapshot persistence before the model swaps, so the write never observes the new
        // document.
        let persistence = self.flush_persistence();
        self.model.reset_document();
        // A swap invalidates mutation slots for the outgoing file; uncertainty debt
        // survives it.
        self.pdf_mutation_active = None;
        self.pdf_mutation_busy_retry = None;
        self.pdf_mutation_pending = None;
        // The reconciliation record survives every open; only
        // `attempt_pdf_mutation_reconciliation`, on a matching retained snapshot, clears it.
        self.model.file_path = Some(path.clone());
        self.model.file_name = services::file_name(&path);
        self.model.format = format;
        self.model.sidebar_tab = self.model.default_sidebar_tab();
        // Start the reading-session clock so time spent in this book accrues.
        self.model.session_start_secs = persistence::now_unix_secs();
        self.model.loading = true;
        self.model.screen = Screen::Reading;

        // `reset_document` renewed the attempt, so earlier loads, even of this path, are
        // stale.
        let attempt = self.model.document_load_attempt;
        let annotations = self.request_annotations_for_document(&path);

        let load = self.request_document_load(attempt, path, format);
        Task::batch([persistence, annotations, load])
    }

    pub(crate) fn request_document_load(
        &mut self,
        attempt: u64,
        path: PathBuf,
        format: Format,
    ) -> Task<Msg> {
        if self.document_load_controller.is_loading() {
            self.document_load_controller.invalidate();
            self.pending_document_load = Some(PendingDocumentLoad {
                attempt,
                path,
                format,
            });
            return Task::none();
        }
        self.pending_document_load = None;
        let worker_path = path.clone();
        // Document open (poppler/EPUB/CBZ parse + first raster band)
        // is submitted through the shared bounded owner.
        let task = self
            .document_load_controller
            .begin(&self.blocking_dispatcher, move || {
                load_document_blocking(worker_path, format)
            });
        match task {
            Ok(task) => task.map(move |settlement| {
                let outcome = match settlement {
                    persistence::BlockingLoadSettlement::Applied(outcome) => outcome,
                    persistence::BlockingLoadSettlement::WorkerFailed(error) => {
                        LoadOutcome::Error(format!("document worker failed: {error}"))
                    }
                    persistence::BlockingLoadSettlement::DiscardedStale => {
                        LoadOutcome::Error("document load was superseded".into())
                    }
                };
                Msg::DocumentLoaded {
                    attempt,
                    path: path.clone(),
                    format,
                    outcome,
                }
            }),
            Err(error) => Task::done(Msg::DocumentLoaded {
                attempt,
                path,
                format,
                outcome: LoadOutcome::Error(format!("document worker failed: {error}")),
            }),
        }
    }

    pub(crate) fn resume_pending_document_load(&mut self, task: Task<Msg>) -> Task<Msg> {
        if self.document_load_controller.is_loading() {
            return task;
        }
        let Some(pending) = self.pending_document_load.take() else {
            return task;
        };
        let next = self.request_document_load(pending.attempt, pending.path, pending.format);
        Task::batch([task, next])
    }

    pub(crate) fn finish_document_load(
        &mut self,
        path: PathBuf,
        format: Format,
        outcome: LoadOutcome,
    ) -> Task<Msg> {
        match outcome {
            LoadOutcome::Epub(load) => {
                self.finish_epub(path, *load);
                self.restored_position_task()
            }
            LoadOutcome::Cbz(load) => {
                let allocations = self.finish_cbz(path, *load);
                Task::batch([allocations, self.restored_position_task()])
            }
            LoadOutcome::Pages {
                pages,
                total_pages,
                extents,
                text,
            } => {
                let allocations = self.finish_pages(path, pages, total_pages, extents, text);
                Task::batch([
                    allocations,
                    self.restored_position_task(),
                    self.request_native_pdf_annotations(),
                    self.request_pdf_navigation(),
                ])
            }
            LoadOutcome::Refused(summary)
                if format == Format::Pdf && pdf::is_password_error(&summary) =>
            {
                self.update(Msg::PasswordRequired(path))
            }
            LoadOutcome::Refused(summary) => {
                self.fail_document_load();
                self.model.error = Some(summary);
                Task::none()
            }
            LoadOutcome::Error(diagnostic) => {
                eprintln!(
                    "reader-app document load failure: format={format:?} diagnostic={diagnostic:?}"
                );
                self.fail_document_load();
                self.model.error = Some(
                    match format {
                        Format::Pdf => "The PDF could not be opened.",
                        Format::Epub => "The EPUB could not be opened.",
                        Format::Cbz => "The comic archive could not be opened.",
                    }
                    .into(),
                );
                Task::none()
            }
        }
    }

    // Leave a failed open on the Library with no document or annotation
    // authority; the caller sets the banner.
    fn fail_document_load(&mut self) {
        self.model.file_path = None;
        self.model.file_name.clear();
        self.annotation_recovery_generation = self.annotation_recovery_generation.wrapping_add(1);
        self.model.annotations = std::sync::Arc::new(persistence::AnnotationsFile::default());
        self.model.annotations_loading = false;
        self.model.annotations_unavailable = false;
        self.annotation_store = None;
        self.model.screen = Screen::Library;
    }

    pub(crate) fn restored_position_task(&mut self) -> Task<Msg> {
        match self.model.pending_position_restore {
            Some(model::PendingPositionRestore::Pages { generation, .. })
            | Some(model::PendingPositionRestore::Epub { generation, .. }) => {
                Task::done(Msg::BeginPositionRestore { generation })
            }
            None => Task::none(),
        }
    }

    pub(crate) fn cancel_position_restore(&mut self) {
        if self.model.pending_position_restore.take().is_some() {
            self.model.position_restore_generation =
                self.model.position_restore_generation.wrapping_add(1);
        }
    }

    pub(crate) fn begin_position_restore(&mut self, generation: u64) -> Task<Msg> {
        if generation != self.model.position_restore_generation {
            return Task::none();
        }
        match self.model.pending_position_restore {
            Some(model::PendingPositionRestore::Pages {
                generation: pending_generation,
                page,
                bootstrap_events_remaining,
                ..
            }) if pending_generation == generation => {
                let expected_offset = self.page_offset_of(page);
                self.model.pages_scroll_offset = expected_offset;
                self.model.pending_position_restore = Some(model::PendingPositionRestore::Pages {
                    generation,
                    page,
                    expected_offset,
                    operation_issued: true,
                    bootstrap_events_remaining,
                });
                self.scroll_to_current_page()
            }
            Some(model::PendingPositionRestore::Epub {
                generation: pending_generation,
                chapter,
                anchor,
                expected_fraction,
                bootstrap_events_remaining,
                ..
            }) if pending_generation == generation => {
                self.model.pending_position_restore = Some(model::PendingPositionRestore::Epub {
                    generation,
                    chapter,
                    anchor,
                    expected_fraction,
                    operation_issued: true,
                    bootstrap_events_remaining,
                });
                self.scroll_to_epub_position()
            }
            _ => Task::none(),
        }
    }
}
