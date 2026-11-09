//! Sixth reducer batch: zoom, rotation, arrangement.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn batch6_request_clear_metadata(&mut self) -> Task<Msg> {
        self.ask(model::PendingPageAction::ClearMetadata);
        Task::none()
    }

    pub(crate) fn batch6_zoom_in(&mut self) -> Task<Msg> {
        if self.model.page_view_controls_supported() {
            self.leave_page_fit();
            self.model.zoom.zoom_in();
            return self.realign_current_page();
        }
        Task::none()
    }

    pub(crate) fn batch6_zoom_out(&mut self) -> Task<Msg> {
        if self.model.page_view_controls_supported() {
            self.leave_page_fit();
            self.model.zoom.zoom_out();
            return self.realign_current_page();
        }
        Task::none()
    }

    pub(crate) fn batch6_zoom_fit_width(&mut self) -> Task<Msg> {
        if self.model.page_view_controls_supported() {
            self.model.zoom.fit_width();
            return self.realign_current_page();
        }
        Task::none()
    }

    fn leave_page_fit(&mut self) {
        if self.model.zoom.mode() != crate::viewport::ZoomMode::FitPage {
            return;
        }
        let Some(extent) = self.model.page_extents.get(self.model.current_page) else {
            return;
        };
        let theme = self.themes.theme();
        let width =
            crate::view::reading::page_fit_width(&self.model, theme, self.reading_content_width());
        let rotation =
            crate::view::reading::effective_page_rotation(&self.model, self.model.current_page);
        let native = (extent.width, extent.height);
        let displayed =
            crate::view::reading::page_display_box(&self.model, theme, native, width, rotation);
        let base =
            crate::viewport::page_box(native, width, crate::viewport::Zoom::default(), rotation);
        self.model.zoom.set(displayed.0 / base.0);
    }

    fn realign_current_page(&mut self) -> Task<Msg> {
        self.model.pages_scroll_offset = self.page_offset_of(self.model.current_page);
        self.scroll_to_current_page()
    }

    pub(crate) fn batch6_zoom_fit_page(&mut self) -> Task<Msg> {
        if self.model.page_view_controls_supported() {
            self.model.zoom.fit_page();
            self.model.pages_viewport_height =
                (self.window_height - crate::view::widgets::TITLEBAR_HEIGHT).max(1.0);
            self.model.pages_scroll_offset = self.page_offset_of(self.model.current_page);
            return self.scroll_to_current_page();
        }
        Task::none()
    }

    pub(crate) fn batch6_rotate_clockwise(&mut self) -> Task<Msg> {
        if self.model.page_view_controls_supported() {
            self.model.rotation = self.model.rotation.clockwise();
            return self.realign_current_page();
        }
        Task::none()
    }

    pub(crate) fn toggle_page_view_mode(&mut self) -> Task<Msg> {
        if !self.model.page_view_controls_supported() {
            return Task::none();
        }
        use crate::viewport::PageViewMode;
        self.model.page_view_mode = match self.model.page_view_mode {
            PageViewMode::Continuous => PageViewMode::SinglePage,
            PageViewMode::SinglePage => PageViewMode::Continuous,
        };
        if self.model.page_view_mode == PageViewMode::SinglePage {
            self.model.spread_mode = crate::spread::SpreadMode::Single;
        }
        self.realign_current_page()
    }

    pub(crate) fn batch6_toggle_spread(&mut self) -> Task<Msg> {
        if !self.model.page_view_controls_supported() {
            return Task::none();
        }
        use crate::spread::SpreadMode;
        self.model.page_view_mode = crate::viewport::PageViewMode::Continuous;
        self.model.spread_mode = match self.model.spread_mode {
            SpreadMode::Single => SpreadMode::TwoPage,
            SpreadMode::TwoPage => SpreadMode::Single,
        };
        // The column's geometry changed under the reader; keep them on their page.
        self.update(Msg::GoToPage(self.model.current_page))
    }

    pub(crate) fn batch6_rotate_page_left(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        self.rotate_page(id, false);
        Task::none()
    }

    pub(crate) fn batch6_rotate_page_right(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        self.rotate_page(id, true);
        Task::none()
    }

    pub(crate) fn batch6_request_split_pdf(&mut self) -> Task<Msg> {
        if self.model.page_editing_supported() {
            self.ask(model::PendingPageAction::SplitPdf);
        }
        Task::none()
    }

    pub(crate) fn batch6_toggle_epub_view_mode(&mut self) -> Task<Msg> {
        if self.model.format == Format::Epub {
            self.model.epub_view_mode = match self.model.epub_view_mode {
                EpubViewMode::Scrolled => EpubViewMode::Paginated,
                EpubViewMode::Paginated => EpubViewMode::Scrolled,
            };
            // Carry the reading position across the switch through the char-offset anchor.
            match self.model.epub_view_mode {
                EpubViewMode::Paginated => {
                    self.model.epub_page_offset = self.offset_for_scroll_fraction();
                    self.repaginate();
                }
                EpubViewMode::Scrolled => {
                    self.model.epub_scroll_fraction = self.scroll_fraction_for_offset();
                    self.repaginate();
                }
            }
            self.save_position();
            // The scroll view is rebuilt at offset 0, so returning to it needs the same
            // `snap_to` the resize path uses.
            if self.model.epub_view_mode == EpubViewMode::Scrolled {
                return iced::widget::operation::snap_to(
                    crate::view::reading::epub_scroll_id(),
                    iced::widget::scrollable::RelativeOffset {
                        x: 0.0,
                        y: self.model.epub_scroll_fraction,
                    },
                    iced::advanced::widget::operation::Animation::Instant,
                );
            }
        }
        Task::none()
    }

    pub(crate) fn batch6_password_required(&mut self, path: PathBuf) -> Task<Msg> {
        self.model.loading = false;
        self.model.file_path = None;
        self.model.file_name.clear();
        self.annotation_recovery_generation = self.annotation_recovery_generation.wrapping_add(1);
        self.model.annotations = std::sync::Arc::new(persistence::AnnotationsFile::default());
        self.model.annotations_loading = false;
        self.model.annotations_unavailable = false;
        self.annotation_store = None;
        self.model.clear_password();
        self.model.password_prompt = Some(path);
        let focus = crate::overlay_focus::open(&mut self.model, model::Overlay::Password);
        // Not an error banner: the document is locked, not broken.
        self.model.error = None;
        self.model.screen = Screen::Library;
        focus
    }

    pub(crate) fn batch6_password_changed(&mut self, text: String) -> Task<Msg> {
        self.model.password_input = text;
        // Clear the error on the first keystroke so the field stops reading invalid.
        self.model.password_error = None;
        Task::none()
    }

    pub(crate) fn batch6_toggle_password_reveal(&mut self) -> Task<Msg> {
        self.model.password_revealed = !self.model.password_revealed;
        Task::none()
    }

    pub(crate) fn batch6_submit_password(&mut self) -> Task<Msg> {
        let Some(path) = self.model.password_prompt.clone() else {
            return Task::none();
        };
        if self.model.password_input.is_empty() || self.model.password_unlocking {
            // A running unlock is not restarted, which would rasterise the first band twice.
            return Task::none();
        }
        // Poppler open, rasterisation and text extraction stay on the blocking owner.
        self.model.password_attempt = self.model.password_attempt.wrapping_add(1);
        let attempt = self.model.password_attempt;
        self.model.password_unlocking = true;
        self.model.password_error = None;
        // The secret moves into the future and never rides the returning message. It is
        // kept in `pdf_password` for later page rasters; see that field's docs.
        let password = self.model.password_input.clone();
        self.model.pdf_password = Some(password.clone());
        let result_path = path.clone();
        self.dispatch_blocking(move || unlock_pdf_blocking(path, password))
            .map(move |result| Msg::PasswordUnlockResult {
                attempt,
                path: result_path.clone(),
                outcome: result.unwrap_or_else(|error| {
                    LoadOutcome::Error(format!("PDF worker failed: {error}"))
                }),
            })
    }

    pub(crate) fn batch6_password_unlock_result(
        &mut self,
        attempt: u64,
        path: PathBuf,
        outcome: LoadOutcome,
    ) -> Task<Msg> {
        if attempt != self.model.password_attempt {
            // Cancelled or superseded, so a slow attempt never lands over a dismissed dialog.
            return Task::none();
        }
        self.model.password_unlocking = false;
        match outcome {
            LoadOutcome::Pages {
                pages,
                total_pages,
                extents,
                text,
            } => {
                // The session key survives the reset below; later bands reopen the file.
                let session_key = self.model.pdf_password.clone();
                // Wipe the typed secret before anything else observes the model.
                self.model.clear_password();
                // A new document replaces `model.annotations`, so deferred sidecars flush
                // first.
                let persistence = self.flush_persistence();
                self.model.reset_document();
                self.model.file_path = Some(path.clone());
                self.model.file_name = services::file_name(&path);
                self.model.format = Format::Pdf;
                self.model.session_start_secs = persistence::now_unix_secs();
                self.model.pdf_password = session_key;
                let allocations =
                    self.finish_pages(path.clone(), pages, total_pages, extents, text);
                let annotations = self.request_annotations_for_document(&path);
                let native_annotations = self.request_native_pdf_annotations();
                let pdf_navigation = self.request_pdf_navigation();
                return Task::batch([
                    allocations,
                    persistence,
                    annotations,
                    native_annotations,
                    pdf_navigation,
                    self.restored_position_task(),
                ]);
            }
            _ => {
                // A key that unlocked nothing is not a session key.
                self.model.pdf_password = None;
                // Poppler's message can echo the password on some builds, so a fixed
                // string is shown.
                self.model.password_error = Some(crate::view::states::PASSWORD_WRONG.to_string());
                self.model.password_input.clear();
            }
        }
        Task::none()
    }

    pub(crate) fn batch6_cancel_password(&mut self) -> Task<Msg> {
        crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::CloseButton,
        )
    }

    pub(crate) fn batch6_flush_persistence(&mut self) -> Task<Msg> {
        self.flush_persistence()
    }

    pub(crate) fn batch6_persistence_finished(
        &mut self,
        destination: String,
        generation: u64,
        outcome: crate::plumbing::persistence::PersistenceOutcome,
    ) -> Task<Msg> {
        self.record_persistence_completion(&destination, generation, outcome);
        // A durable settlement may complete the close drain, so re-check now; the freed
        // owner slot also lets the busy-retry pump run.
        Task::batch([self.check_close_drain_completion(), self.pump_busy_retry()])
    }

    pub(crate) fn batch6_retry_persistence(&mut self) -> Task<Msg> {
        // Retry re-issues every failed destination; after a vetoed close it also re-enters
        // the drain, so one action can retire the debt and finish the close.
        let retry = self.retry_persistence();
        if matches!(self.close_state, CloseState::Vetoed { .. }) {
            Task::batch([retry, Task::done(Msg::RetryClose)])
        } else {
            retry
        }
    }

    pub(crate) fn batch6_retry_state_recovery(
        &mut self,
        origin: crate::plumbing::persistence::StateOrigin,
    ) -> Task<Msg> {
        if self.load_controller.is_loading() {
            self.load_controller.invalidate();
            self.pending_load = Some(PendingLoad::Recovery(origin));
            return Task::none();
        }
        let generation = self.advance_recovery_generation(&origin);
        let base_revision = match &origin {
            persistence::StateOrigin::Library => {
                self.model.library_loading = true;
                self.library_state_revision
            }
            persistence::StateOrigin::Annotations { .. } => {
                self.model.annotations_loading = true;
                self.annotation_state_revision
            }
        };
        let resolver = self.xdg_resolver.clone();
        let data_resolver = self.app_data_resolver.clone();
        let annotations_root = self.annotations_root.clone();
        let worker_origin = origin.clone();
        let message_origin = origin;
        self.active_load_origin = Some(message_origin.clone());
        let task = self
            .load_controller
            .begin(&self.blocking_dispatcher, move || {
                persistence::reload_recovery_with_data(
                    worker_origin,
                    &resolver,
                    &data_resolver,
                    annotations_root.as_deref(),
                )
            });
        let failed_origin = message_origin.clone();
        let settlement = match task {
            Ok(task) => {
                return task.map(move |settlement| Msg::StateRecoveryReloaded {
                    generation,
                    base_revision,
                    origin: message_origin.clone(),
                    settlement,
                });
            }
            Err(error) => persistence::BlockingLoadSettlement::WorkerFailed(error),
        };
        Task::done(Msg::StateRecoveryReloaded {
            generation,
            base_revision,
            origin: failed_origin,
            settlement,
        })
    }

    pub(crate) fn batch6_startup_library_restored(
        &mut self,
        generation: u64,
        base_revision: u64,
        open_path: Option<PathBuf>,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    ) -> Task<Msg> {
        if !self.load_controller.is_loading() {
            self.active_load_origin = None;
        }
        let reload = match settlement {
            persistence::BlockingLoadSettlement::Applied(reload) => reload,
            persistence::BlockingLoadSettlement::DiscardedStale => {
                return self.resume_pending_load(Task::none());
            }
            persistence::BlockingLoadSettlement::WorkerFailed(error) => {
                eprintln!("reader-app library restore worker failed: {error}");
                persistence::recovery_worker_failure_with_data(
                    persistence::StateOrigin::Library,
                    &self.xdg_resolver,
                    &self.app_data_resolver,
                    None,
                )
            }
        };
        let task = self.apply_startup_library_restore(generation, base_revision, open_path, reload);
        self.resume_pending_load(task)
    }

    pub(crate) fn batch6_state_recovery_reloaded(
        &mut self,
        generation: u64,
        base_revision: u64,
        origin: crate::plumbing::persistence::StateOrigin,
        settlement: crate::plumbing::persistence::BlockingLoadSettlement,
    ) -> Task<Msg> {
        if !self.load_controller.is_loading() {
            self.active_load_origin = None;
        }
        let reload = match settlement {
            persistence::BlockingLoadSettlement::Applied(reload) => reload,
            persistence::BlockingLoadSettlement::DiscardedStale => {
                return self.resume_pending_load(Task::none());
            }
            persistence::BlockingLoadSettlement::WorkerFailed(error) => {
                eprintln!("reader-app recovery worker failed: {error}");
                persistence::recovery_worker_failure_with_data(
                    origin,
                    &self.xdg_resolver,
                    &self.app_data_resolver,
                    self.annotations_root.as_deref(),
                )
            }
        };
        let current_revision = match &reload {
            persistence::RecoveryReload::Library { .. } => self.library_state_revision,
            persistence::RecoveryReload::Annotations { .. } => self.annotation_state_revision,
        };
        if generation != self.recovery_generation_for(&reload) {
            return Task::none();
        }
        if base_revision != current_revision {
            match &reload {
                persistence::RecoveryReload::Library { .. } => {
                    self.model.library_loading = false;
                }
                persistence::RecoveryReload::Annotations { .. } => {
                    self.model.annotations_loading = false;
                    self.model.annotations_unavailable = true;
                }
            }
            self.resume_pending_load(Task::none())
        } else {
            let task = self.apply_recovery_reload(reload);
            self.resume_pending_load(task)
        }
    }

    pub(crate) fn batch6_reveal_recovery_location(
        &mut self,
        origin: crate::plumbing::persistence::StateOrigin,
    ) -> Task<Msg> {
        let location = self
            .model
            .recovery_authorities
            .iter()
            .find(|authority| authority.origin == origin)
            .and_then(|authority| {
                if let Some(path) = authority.recovery_path() {
                    path.parent()
                        .map(|directory| (path.to_path_buf(), directory.to_path_buf()))
                } else {
                    authority
                        .source_path
                        .parent()
                        .map(|directory| (directory.to_path_buf(), directory.to_path_buf()))
                }
            });
        if let Some((display_path, open_path)) = location {
            self.model
                .announce(format!("Recovery location: {}", display_path.display()));
            self.model.revealed_recovery_location = Some((origin, display_path));
            match self.recovery_location_opener.open(&open_path) {
                Ok(()) => self.model.picker_failure = None,
                Err(message) => self.model.picker_failure = Some(message),
            }
        }
        Task::none()
    }

    pub(crate) fn batch6_recovery_location_tick(&mut self) -> Task<Msg> {
        match self.recovery_location_opener.pump() {
            Some((generation, outcome)) => Task::done(Msg::RecoveryLocationFinished {
                generation,
                outcome,
            }),
            None => Task::none(),
        }
    }

    pub(crate) fn batch6_recovery_location_finished(
        &mut self,
        generation: u64,
        outcome: enzui_iced_core::portal::PortalOutcome<()>,
    ) -> Task<Msg> {
        if generation != self.recovery_location_opener.generation() {
            return Task::none();
        }
        use enzui_iced_core::portal::PortalOutcome;
        match outcome {
            PortalOutcome::Completed(()) => {
                self.model.announce("Opened recovery location.");
            }
            PortalOutcome::Cancelled(_) => {}
            PortalOutcome::Unavailable
            | PortalOutcome::Unsupported
            | PortalOutcome::NotDeclared
            | PortalOutcome::Denied
            | PortalOutcome::ParentUnavailable
            | PortalOutcome::Failed => {
                self.model.picker_failure =
                    Some("The desktop could not open the recovery location.".into());
            }
        }
        Task::none()
    }

    pub(crate) fn batch6_start_empty_state(
        &mut self,
        origin: crate::plumbing::persistence::StateOrigin,
    ) -> Task<Msg> {
        self.start_empty_state(origin)
    }
}
