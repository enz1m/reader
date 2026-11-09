use super::*;

use iced::Task;

impl App {
    fn open_dropped_document(
        &mut self,
        receipt: enzui_iced_widgets::file_drop::FileDrop,
        copies: Option<enzui_iced_core::drop_files::CopiedFiles>,
    ) -> Task<Msg> {
        let path = copies.as_ref().map_or_else(
            || receipt.paths()[0].clone(),
            |copies| copies.paths()[0].clone(),
        );
        let task = self.update(Msg::OpenPath(path.clone()));
        if self.model.loading && self.model.file_path.as_ref() == Some(&path) {
            self.native_drop = Some((self.model.document_load_attempt, path, receipt, copies));
        }
        task
    }

    /// Processes an application message and returns its scheduled effects.
    ///
    /// After close commitment, only the main-window cleanup notification is accepted.
    pub fn update(&mut self, message: Msg) -> Task<Msg> {
        // The crash ring must observe even messages rejected after close commitment.
        enzui_iced_core::crash_report::observe(&message);
        if matches!(self.close_state, CloseState::Committed { .. })
            && !matches!(message, Msg::MainWindowClosed)
        {
            return Task::none();
        }
        // Accessibility operations keep their publishing window when drained by any message.
        let a11y_pending =
            enzui_iced_widgets::a11y_actions::apply_pending_accessibility_actions::<Msg>();
        let task = self.reduce(message);
        if let Some((attempt, path, _, _)) = &self.native_drop {
            if *attempt != self.model.document_load_attempt
                || self.model.file_path.as_ref() != Some(path)
            {
                self.native_drop = None;
            } else if !self.model.loading
                && let Some((_, _, receipt, copies)) = self.native_drop.take()
                && self.model.error.is_none()
                && self.model.overlay.is_none()
            {
                if let Some(copies) = copies {
                    copies.preserve();
                }
                receipt.finish();
            }
        }
        let _ = self
            .model
            .toasts
            .sync_expiry(&mut self.model.toast_expiry, std::time::Instant::now());
        if self.model.screen == Screen::Library {
            self.ensure_projection_fresh();
        }
        Task::batch([
            a11y_pending,
            task,
            self.ensure_page_rasters(),
            self.ensure_pdf_thumbnails(),
            self.ensure_epub_geometry(),
        ])
    }

    pub(crate) fn reduce(&mut self, message: Msg) -> Task<Msg> {
        // Navigation invalidates the prose action surface's rectangle, so close it before
        // the destination applies.
        if matches!(
            &message,
            Msg::NextChapter
                | Msg::PrevChapter
                | Msg::GoToChapter(_)
                | Msg::NextPage
                | Msg::PrevPage
                | Msg::GoToPage(_)
                | Msg::GoToBookmark { .. }
                | Msg::GoToAnnotation(_)
                | Msg::ToggleEpubViewMode
        ) {
            self.cancel_position_restore();
        }
        if matches!(
            &message,
            Msg::OpenPath(_)
                | Msg::GoLibrary
                | Msg::DocumentLoaded { .. }
                | Msg::NextChapter
                | Msg::PrevChapter
                | Msg::GoToChapter(_)
                | Msg::NextPage
                | Msg::PrevPage
                | Msg::GoToPage(_)
                | Msg::GoToBookmark { .. }
                | Msg::ToggleEpubViewMode
                | Msg::FontSizeUp
                | Msg::FontSizeDown
                | Msg::LineSpacingUp
                | Msg::LineSpacingDown
                | Msg::SetReadingTheme(_)
                | Msg::WindowResized(_)
                | Msg::GoToAnnotation(_)
        ) {
            self.model.active_selection = None;
            self.model.active_footnote = None;
            // Controlled clear, so the painted tint disappears with the surface.
            self.bump_selection_clear();
        }
        match message {
            Msg::InstanceWake => self.forwarded_launch(),
            // Shared-theme events drive only the chrome; the reading palette is untouched.
            Msg::Theme(event) => self.batch1_theme(*event),
            Msg::Toast(event) => self.batch1_toast(event),
            Msg::A11yWake(receipt) => self.batch1_a11y_wake(receipt),
            Msg::FilesDropped(receipt) => {
                self.native_drop = None;
                if receipt.paths().len() != 1 || self.model.loading || self.model.overlay.is_some()
                {
                    self.model.error = Some("Drop one document at a time.".into());
                    return Task::none();
                }
                if receipt.is_temporary() {
                    let Some(storage) = self
                        .xdg_resolver
                        .resolve(
                            enzui_iced_core::persist::PersistenceKind::State,
                            "reader-app",
                            "imports",
                        )
                        .ok()
                        .flatten()
                    else {
                        return Task::none();
                    };
                    let paths = receipt.paths().to_vec();
                    let attempt = self.model.document_load_attempt;
                    return self
                        .dispatch_blocking(move || {
                            enzui_iced_core::drop_files::copy_files(
                                &paths,
                                Some(&storage),
                                512 * 1024 * 1024,
                                &enzui_iced_core::cancellation::CancelToken::new(),
                            )
                            .map_err(|error| error.to_string())
                        })
                        .map(move |result| Msg::FilesRetained {
                            attempt,
                            receipt: receipt.clone(),
                            copies: result
                                .map_err(|error| error.to_string())
                                .and_then(std::convert::identity),
                        });
                }
                self.open_dropped_document(receipt, None)
            }
            Msg::FilesRetained {
                attempt,
                receipt,
                copies,
            } => {
                if attempt != self.model.document_load_attempt
                    || self.model.loading
                    || self.model.overlay.is_some()
                {
                    return Task::none();
                }
                match copies {
                    Ok(copies) => self.open_dropped_document(receipt, Some(copies)),
                    Err(error) => {
                        self.model.error = Some(error);
                        Task::none()
                    }
                }
            }
            Msg::OpenPath(path) => self.batch1_open_path(path),
            #[cfg(feature = "center")]
            Msg::Center(event) => self.center_event(event),
            Msg::OpenDialog => self.batch1_open_dialog(),
            Msg::MainWindowReady(id) => self.batch1_main_window_ready(id),
            Msg::MainWindowClosed => self.batch1_main_window_closed(),
            Msg::PickerParentResolved {
                generation,
                target,
                window,
                payload,
            } => self.batch1_picker_parent_resolved(generation, target, window, payload),
            Msg::PickResolved {
                generation,
                target,
                window,
                outcome,
            } => self.batch1_pick_resolved(generation, target, window, outcome),
            Msg::PortalTick => self.batch1_portal_tick(),
            Msg::GoLibrary => self.batch1_go_library(),
            Msg::LibraryQueryChanged(q) => self.batch1_library_query_changed(q),
            Msg::LibrarySortMenu(event) => self.batch1_library_sort_menu(event),
            Msg::LibraryScrolled {
                offset,
                viewport_height,
            } => self.batch1_library_scrolled(offset, viewport_height),
            Msg::DocumentLoaded {
                attempt,
                path,
                format,
                outcome,
            } => self.batch1_document_loaded(attempt, path, format, outcome),
            Msg::AnnotationsRestored {
                generation,
                base_revision,
                settlement,
            } => self.batch1_annotations_restored(generation, base_revision, settlement),
            Msg::PdfNavigationLoaded {
                generation,
                outcome,
            } => self.batch1_pdf_navigation_loaded(generation, outcome),
            Msg::NativePdfAnnotationsLoaded {
                generation,
                outcome,
            } => self.batch1_native_pdf_annotations_loaded(generation, outcome),
            Msg::NextChapter => self.batch1_next_chapter(),
            Msg::PrevChapter => self.batch1_prev_chapter(),
            Msg::GoToChapter(idx) => self.batch1_go_to_chapter(idx),
            Msg::GoToPage(target) => self.batch1_go_to_page(target),
            Msg::BeginPositionRestore { generation } => {
                self.batch1_begin_position_restore(generation)
            }
            Msg::PagesScrolled {
                generation,
                offset,
                viewport_height,
                viewport_width,
            } => self.batch1_pages_scrolled(generation, offset, viewport_height, viewport_width),
            Msg::PagesRasterised {
                generation,
                requested,
                pages,
                text,
            } => self.batch1_pages_rasterised(generation, requested, pages, text),
            Msg::PageRasterAllocated {
                generation,
                page,
                allocation,
            } => {
                if generation == self.model.raster_generation
                    && self.model.pages.contains(page.index)
                {
                    let protect = self
                        .visible_band()
                        .unwrap_or(self.model.current_page..=self.model.current_page);
                    let mut page = *page;
                    page.allocation = allocation;
                    self.model.pages.insert(page, protect);
                }
                Task::none()
            }
            Msg::PdfThumbnailsScrolled {
                generation,
                offset,
                viewport_height,
            } => self.batch2_pdf_thumbnails_scrolled(generation, offset, viewport_height),
            Msg::PdfThumbnailsRendered {
                generation,
                requested,
                pages,
            } => self.batch2_pdf_thumbnails_rendered(generation, requested, pages),
            // The focus gate lives here because `listen_with` cannot capture the model;
            // `Msg::NextPage` keeps its single clamp contract. `document_focused` is false
            // under an overlay or off the reading surface.
            Msg::DocumentKeyCommand(command) => self.batch2_document_key_command(command),
            // Modal state is unavailable to the stateless event classifier.
            Msg::ShellKeyCommand(command) => self.batch2_shell_key_command(command),
            Msg::PageEntryEdited(text) => {
                // Digits only; the field is a page number.
                self.model.page_entry =
                    Some(text.chars().filter(char::is_ascii_digit).take(9).collect());
                Task::none()
            }
            Msg::PageEntrySubmitted => {
                let target = self
                    .model
                    .page_entry
                    .take()
                    .and_then(|text| text.parse::<usize>().ok())
                    .filter(|page| *page >= 1);
                match target {
                    Some(page) => {
                        let last = self.model.page_count.max(1);
                        self.update(Msg::GoToPage(page.min(last) - 1))
                    }
                    None => Task::none(),
                }
            }
            Msg::NextPage => self.batch2_next_page(),
            Msg::PrevPage => self.batch2_prev_page(),
            Msg::FontSizeUp => self.batch2_font_size_up(),
            Msg::FontSizeDown => self.batch2_font_size_down(),
            Msg::LineSpacingUp => self.batch2_line_spacing_up(),
            Msg::LineSpacingDown => self.batch2_line_spacing_down(),
            Msg::SetReadingTheme(t) => self.batch2_set_reading_theme(t),
            Msg::ToggleReadingThemeMenu => self.batch2_toggle_reading_theme_menu(),
            Msg::CloseReadingThemeMenu => self.batch2_close_reading_theme_menu(),
            Msg::FootnoteClicked(target_id) => self.batch2_footnote_clicked(target_id),
            Msg::DismissFootnote => self.batch2_dismiss_footnote(),
            Msg::CopiedSelection(text) => self.batch2_copied_selection(text),
            Msg::SelectionCopied(result) => self.batch2_selection_copied(result),
            Msg::SelectionChanged(sel) => self.batch2_selection_changed(sel),
            Msg::HighlightSelection => self.batch2_highlight_selection(),
            Msg::AddNoteToSelection => self.batch2_add_note_to_selection(),
            Msg::CopySelection => self.batch2_copy_selection(),
            Msg::DismissSelectionActions => self.batch2_dismiss_selection_actions(),
            Msg::ToggleSidebar => self.batch2_toggle_sidebar(),
            Msg::DismissSidebar => self.batch2_dismiss_sidebar(),
            Msg::WindowResized(size) => self.batch2_window_resized(size),
            Msg::EpubGeometryMeasured(key, geometry) => {
                if self.model.epub_geometry_pending == Some(key) {
                    self.model.epub_geometry_pending = None;
                }
                if let Some(geometry) = geometry {
                    self.model.epub_geometry = Some(geometry);
                }
                Task::none()
            }
            Msg::EpubPageBoxMeasured(height) => {
                if height.is_finite()
                    && self
                        .model
                        .epub_page_box
                        .is_none_or(|known| (known - height).abs() >= 0.5)
                {
                    self.model.epub_page_box = Some(height);
                    self.repaginate();
                }
                Task::none()
            }
            Msg::EpubPageWheel(notches) => {
                if !notches.is_finite() {
                    return Task::none();
                }
                // A reversal starts over, so a page never turns against the wheel.
                if self.model.epub_page_wheel * notches < 0.0 {
                    self.model.epub_page_wheel = 0.0;
                }
                self.model.epub_page_wheel += notches;
                if self.model.epub_page_wheel <= -1.0 {
                    self.model.epub_page_wheel = 0.0;
                    return self.update(Msg::NextPage);
                }
                if self.model.epub_page_wheel >= 1.0 {
                    self.model.epub_page_wheel = 0.0;
                    return self.update(Msg::PrevPage);
                }
                Task::none()
            }
            Msg::EpubScrolled {
                generation,
                fraction,
                offset,
                viewport_height,
            } => {
                if generation == self.model.position_restore_generation
                    && offset.is_finite()
                    && viewport_height.is_finite()
                {
                    self.model.epub_scroll_offset = offset.max(0.0);
                    self.model.epub_viewport_height = viewport_height.max(0.0);
                }
                self.batch3_epub_scrolled(generation, fraction)
            }
            Msg::DismissError => self.batch3_dismiss_error(),
            Msg::DismissPickerFailure => self.batch3_dismiss_picker_failure(),
            Msg::HidePersistenceAlert => {
                self.model.persistence_alert_hidden = true;
                Task::none()
            }
            Msg::CloseWindow(window) => self.batch3_close_window(window),
            Msg::RetryClose => self.batch3_retry_close(),
            Msg::CloseDrainTick => self.batch3_close_drain_tick(),
            Msg::RasterBandRefusedBusy {
                generation,
                indices,
            } => self.batch3_raster_band_refused_busy(generation, indices),
            Msg::PdfThumbnailsRefusedBusy {
                generation,
                indices,
            } => self.batch3_pdf_thumbnails_refused_busy(generation, indices),
            Msg::SearchRefusedBusy { generation, format } => {
                self.batch3_search_refused_busy(generation, format)
            }
            Msg::RecentThumbnailRefusedBusy { path, source } => {
                self.batch3_recent_thumbnail_refused_busy(path, source)
            }
            Msg::BusyRetryTick => self.batch3_busy_retry_tick(),
            Msg::Window(command) => self.batch3_window(command),
            Msg::GoToAnnotation(id) => self.batch3_go_to_annotation(id),
            Msg::RemoveAnnotation(id) => self.batch3_remove_annotation(id),
            Msg::CyclePdfAnnotationColor(id) => self.batch3_cycle_pdf_annotation_color(id),
            Msg::PdfMutationCompleted {
                op_token,
                document_generation,
                intent,
                outcome,
            } => {
                self.batch3_pdf_mutation_completed(op_token, document_generation, *intent, outcome)
            }
            Msg::PdfMutationRefusedBusy {
                op_token,
                document_generation,
                intent,
            } => self.batch3_pdf_mutation_refused_busy(op_token, document_generation, *intent),
            Msg::RetryPdfMutationUncertain => self.batch3_retry_pdf_mutation_uncertain(),
            Msg::PdfMutationUncertainRetried {
                op_token,
                retry_sequence,
                outcome,
            } => self.batch3_pdf_mutation_uncertain_retried(op_token, retry_sequence, outcome),
            Msg::DismissPdfMutationUncertainWarning => {
                self.batch3_dismiss_pdf_mutation_uncertain_warning()
            }
            Msg::DismissArrangementUncertainPublication => {
                self.batch3_dismiss_arrangement_uncertain_publication()
            }
            Msg::DismissExtractUncertainPublication => {
                self.batch3_dismiss_extract_uncertain_publication()
            }
            // The `Ctrl+D` page-free form resolves the page here, where the model is.
            Msg::ToggleCurrentBookmark => self.batch3_toggle_current_bookmark(),
            Msg::ToggleBookmark(page) => self.batch3_toggle_bookmark(page),
            Msg::GoToBookmark {
                format,
                document_key,
                position,
            } => self.batch3_go_to_bookmark(format, document_key, position),
            Msg::RemoveBookmark(page) => self.batch4_remove_bookmark(page),

            Msg::SelectSidebarTab(tab) => self.batch4_select_sidebar_tab(tab),
            Msg::ToggleAnnotationTools(expanded) => self.batch4_toggle_annotation_tools(expanded),
            Msg::AnnotationFilterChanged(text) => self.batch4_annotation_filter_changed(text),
            Msg::CopyCitation(id) => self.batch4_copy_citation(id),
            Msg::CitationCopied(result) => self.batch4_citation_copied(result),
            Msg::BeginEditAnnotation(id) => self.batch4_begin_edit_annotation(id),
            Msg::EditAnnotationChanged(text) => self.batch4_edit_annotation_changed(text),
            Msg::CommitEditAnnotation => self.batch4_commit_edit_annotation(),
            Msg::CancelEditAnnotation => self.batch4_cancel_edit_annotation(),
            Msg::CopyAnnotationsJson => self.batch4_copy_annotations_json(),
            Msg::AnnotationsJsonCopied(result) => self.batch4_annotations_json_copied(result),
            Msg::CopyAnnotationsMarkdown => self.batch4_copy_annotations_markdown(),
            Msg::AnnotationsMarkdownCopied(result) => {
                self.batch4_annotations_markdown_copied(result)
            }

            Msg::OpenSearch => self.batch4_open_search(),
            Msg::SearchQueryChanged(q) => self.batch4_search_query_changed(q),
            Msg::SearchTick => self.batch4_search_tick(),
            Msg::SearchCompleted { generation, hits } => {
                self.batch4_search_completed(generation, hits)
            }
            Msg::PdfSearchCompleted { generation, hits } => {
                self.batch4_pdf_search_completed(generation, hits)
            }
            Msg::SearchResultActivated(index) => self.batch4_search_result_activated(index),
            Msg::NextMatch => self.batch4_next_match(),
            Msg::PrevMatch => self.batch4_prev_match(),
            Msg::FocusNext => self.batch4_focus_next(),
            Msg::FocusPrevious => self.batch5_focus_previous(),

            Msg::Undo => self.batch5_undo(),
            Msg::Redo => self.batch5_redo(),

            Msg::OpenPageArranger => self.batch5_open_page_arranger(),
            Msg::ClosePageArranger => self.batch5_close_page_arranger(),
            Msg::CloseOverlay => self.batch5_close_overlay(),
            Msg::ShellDialogEvent(event) => self.batch5_shell_dialog_event(event),
            Msg::MovePageUp(id) => self.batch5_move_page_up(id),
            Msg::MovePageDown(id) => self.batch5_move_page_down(id),
            Msg::PageDropped(request) => self.batch5_page_dropped(request),
            Msg::OpenPage(id) => self.batch5_open_page(id),
            Msg::RequestDeletePage(id) => self.batch5_request_delete_page(id),
            Msg::RequestExtractPage(id) => self.batch5_request_extract_page(id),
            Msg::OpenPdfPasswordForm => self.open_pdf_password_form(),
            Msg::OpenSignDialog
            | Msg::SignNameChanged(_)
            | Msg::SignFontSelected(_)
            | Msg::ChooseSigningCertificate
            | Msg::ClearSigningCertificate
            | Msg::SignPasswordChanged(_)
            | Msg::ToggleSignPasswordReveal
            | Msg::CancelSigning
            | Msg::PlaceSignature
            | Msg::SignaturePointer { .. }
            | Msg::SignatureClicked(_)
            | Msg::SignedCopyWritten(_) => self.reduce_signing(message),
            Msg::PdfPasswordForm(event) => self.pdf_password_form_event(event),
            Msg::ArrangerSelectionChanged(keys) => {
                let entries = self.model.arrangement.entries();
                self.model.arranger_selection = keys
                    .iter()
                    .filter_map(|key| entries.iter().find(|entry| &entry.key() == key))
                    .map(|entry| entry.id)
                    .collect();
                Task::none()
            }
            Msg::ArrangerScrolled {
                offset,
                viewport_height,
            } => {
                if offset.is_finite() && viewport_height.is_finite() {
                    self.model.arranger_scroll_offset = offset.max(0.0);
                    self.model.arranger_viewport_height = viewport_height.max(0.0);
                }
                Task::none()
            }
            Msg::RotatePages { ids, clockwise } => {
                if self.model.arrangement_publication.is_none() {
                    for id in ids {
                        self.rotate_page(id, clockwise);
                    }
                }
                Task::none()
            }
            Msg::RequestDeletePages(ids) => self.request_delete_pages(ids),
            Msg::RequestExtractPages(ids) => {
                if self.model.arrangement_publication.is_none() && !ids.is_empty() {
                    self.ask(model::PendingPageAction::ExtractPages(ids));
                }
                Task::none()
            }
            Msg::ApplyPageArrangement => self.batch5_apply_page_arrangement(),
            Msg::SavePageArrangementAs => self.batch5_save_page_arrangement_as(),
            Msg::PageArrangementPublished {
                generation,
                source,
                destination,
                arrangement,
                result,
            } => self.batch5_page_arrangement_published(
                generation,
                source,
                destination,
                arrangement,
                result,
            ),
            Msg::RequestMergePdf => self.batch5_request_merge_pdf(),
            Msg::MergePathChanged(text) => self.batch5_merge_path_changed(text),
            Msg::MergePdf => self.batch5_merge_pdf(),
            Msg::PageConfirmEvent(event) => self.batch5_page_confirm_event(event),

            Msg::OpenMetadata => self.batch5_open_metadata(),
            Msg::OpenProperties => self.batch5_open_properties(),
            Msg::MetadataFieldChanged(field, value) => {
                self.batch5_metadata_field_changed(field, value)
            }
            Msg::MetadataDatePicker(field, event) => self.batch5_metadata_date_picker(field, event),
            Msg::MetadataTime(field, event) => self.batch5_metadata_time(field, event),
            Msg::ClearMetadataDate(field) => {
                self.model.edit_metadata(field, String::new());
                self.model.reset_metadata_date_ui();
                Task::none()
            }
            Msg::SaveMetadata => self.batch5_save_metadata(),
            Msg::RetryMetadataSave => self.batch5_retry_metadata_save(),
            Msg::RequestClearMetadata => self.batch6_request_clear_metadata(),

            Msg::ZoomIn => self.batch6_zoom_in(),
            Msg::ZoomOut => self.batch6_zoom_out(),
            Msg::ZoomFitWidth => self.batch6_zoom_fit_width(),
            Msg::ZoomFitPage => self.batch6_zoom_fit_page(),
            Msg::RotateClockwise => self.batch6_rotate_clockwise(),
            Msg::ToggleSpread => self.batch6_toggle_spread(),
            Msg::TogglePageViewMode => self.toggle_page_view_mode(),
            Msg::RotatePageLeft(id) => self.batch6_rotate_page_left(id),
            Msg::RotatePageRight(id) => self.batch6_rotate_page_right(id),
            Msg::RequestSplitPdf => self.batch6_request_split_pdf(),
            Msg::ToggleEpubViewMode => self.batch6_toggle_epub_view_mode(),

            Msg::PasswordRequired(path) => self.batch6_password_required(path),
            Msg::PasswordChanged(text) => self.batch6_password_changed(text),
            Msg::TogglePasswordReveal => self.batch6_toggle_password_reveal(),
            Msg::SubmitPassword => self.batch6_submit_password(),
            Msg::PasswordUnlockResult {
                attempt,
                path,
                outcome,
            } => self.batch6_password_unlock_result(attempt, path, outcome),
            Msg::CancelPassword => self.batch6_cancel_password(),
            Msg::FlushPersistence => self.batch6_flush_persistence(),
            Msg::PersistenceFinished {
                destination,
                generation,
                outcome,
            } => self.batch6_persistence_finished(destination, generation, outcome),
            Msg::RetryPersistence => self.batch6_retry_persistence(),
            Msg::RetryStateRecovery(origin) => self.batch6_retry_state_recovery(origin),
            Msg::StartupLibraryRestored {
                generation,
                base_revision,
                open_path,
                settlement,
            } => self.batch6_startup_library_restored(
                generation,
                base_revision,
                open_path,
                settlement,
            ),
            Msg::StateRecoveryReloaded {
                generation,
                base_revision,
                origin,
                settlement,
            } => self.batch6_state_recovery_reloaded(generation, base_revision, origin, settlement),
            Msg::RevealRecoveryLocation(origin) => self.batch6_reveal_recovery_location(origin),
            Msg::RecoveryLocationTick => self.batch6_recovery_location_tick(),
            Msg::RecoveryLocationFinished {
                generation,
                outcome,
            } => self.batch6_recovery_location_finished(generation, outcome),
            Msg::StartEmptyState(origin) => self.batch6_start_empty_state(origin),

            Msg::OpenNoteEditor => self.batch7_open_note_editor(),
            Msg::OpenNoteEditorForEdit(id) => self.batch7_open_note_editor_for_edit(id),
            Msg::NoteEditorAction(action) => self.batch7_note_editor_action(action),
            Msg::CommitNoteEditor => self.batch7_commit_note_editor(),
            Msg::AddPdfComment => self.batch7_add_pdf_comment(),
            Msg::AddPdfHighlight => self.batch7_add_pdf_highlight(),
            Msg::PdfTextSelected {
                page,
                range,
                finished,
            } => self.batch7_pdf_text_selected(page, range, finished),
            Msg::CopyPdfSelection => self.batch7_copy_pdf_selection(),
            Msg::FindSelection => self.batch7_find_selection(),
            Msg::DismissPdfSelectionActions => {
                self.model.pdf_selection_actions = false;
                Task::none()
            }
            Msg::ClearPdfSelection => self.batch7_clear_pdf_selection(),
            Msg::FindSimilar(id) => self.batch7_find_similar(id),
            Msg::RemoveFromRecents(path) => self.batch7_remove_from_recents(path),

            Msg::RecentThumbnailRendered { path, thumb } => {
                self.batch7_recent_thumbnail_rendered(path, thumb)
            }

            Msg::AvailabilityBatchDone {
                generation,
                next_index,
                total,
                outcome,
            } => self.batch7_availability_batch_done(generation, next_index, total, outcome),

            Msg::OpenContextMenu(target, at) => self.batch7_open_context_menu(target, at),
            Msg::ContextMenuEvent(generation, event) => {
                self.batch7_context_menu_event(generation, event)
            }

            Msg::OpenShortcuts => self.batch7_open_shortcuts(),
            Msg::CloseShortcuts => self.batch7_close_shortcuts(),

            Msg::ToggleToolbarOverflow => self.batch7_toggle_toolbar_overflow(),
            Msg::CloseToolbarOverflow => {
                self.model.toolbar_overflow_open = false;
                Task::none()
            }

            Msg::Noop => self.batch7_noop(),
        }
    }
}
