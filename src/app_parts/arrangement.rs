//! Metadata, pages, merge, and arrangement publication.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn save_metadata(&mut self, confirmed: plumbing::pdf_ops::MetadataDateRemoval) {
        if self.model.metadata.has_invalid_date_edit() {
            self.model
                .refuse("Correct the invalid PDF date before saving metadata.");
            return;
        }
        let requested = self.model.metadata.date_removals_requested();
        if !requested.is_empty()
            && ((requested.creation && !confirmed.creation)
                || (requested.modification && !confirmed.modification))
        {
            self.ask(model::PendingPageAction::RemoveMetadataDates(requested));
            return;
        }
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        match plumbing::pdf_ops::write_metadata_with_date_removal(
            &path,
            &self.model.metadata,
            confirmed,
        ) {
            Ok(reopened) => {
                self.model.metadata_baseline = reopened.clone();
                self.model.metadata = reopened;
                self.model.metadata_save_failure = None;
                self.model.error = None;
                self.model.announce("Metadata saved");
            }
            Err(error) => {
                let failure = crate::user_copy::PdfWriteFailure::new(
                    crate::user_copy::PdfWriteOperation::SaveMetadata,
                    error,
                );
                if std::env::var("ENZUI_DIAGNOSTICS").as_deref() == Ok("1") {
                    eprintln!(
                        "[READER_PDF_WRITE] operation={:?} diagnostic={:?}",
                        crate::user_copy::PdfWriteOperation::SaveMetadata,
                        failure.diagnostic()
                    );
                }
                let summary = failure.user_summary();
                self.model.error = Some(summary.clone());
                self.model.metadata_save_failure = Some(model::MetadataSaveFailure {
                    path,
                    confirmed_removal: confirmed,
                    message: summary,
                });
            }
        }
    }

    pub(crate) fn report_pdf_write_failure(
        &mut self,
        operation: crate::user_copy::PdfWriteOperation,
        error: plumbing::pdf_ops::PdfWriteError,
    ) {
        let failure = crate::user_copy::PdfWriteFailure::new(operation, error);
        if std::env::var("ENZUI_DIAGNOSTICS").as_deref() == Ok("1") {
            eprintln!(
                "[READER_PDF_WRITE] operation={operation:?} diagnostic={:?}",
                failure.diagnostic()
            );
        }
        self.model.error = Some(failure.user_summary());
    }

    // The document's page-tree length for arranger purposes.
    //
    // `total_page_count`, not `pages.len()`, which is capped at the eager-render limit
    // and would drop later pages on reorder.
    pub(crate) fn page_tree_len(&self) -> usize {
        self.model.total_page_count.max(self.model.page_count)
    }

    // Present a confirm dialog for `action`.
    //
    // A fresh [`ConfirmState`] every time, so reopening never inherits the previous
    // answer.
    pub(crate) fn ask(&mut self, action: model::PendingPageAction) {
        self.model.page_confirm_state = enzui_iced_core::confirm::ConfirmState::new(None);
        self.model.pending_page_action = Some(action);
    }

    // Fold a confirm-dialog exit.
    //
    // Declining clears the pending action and runs no mutation branch, so the document
    // is unchanged by construction.
    pub(crate) fn on_page_confirm(
        &mut self,
        event: enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent,
    ) -> Task<Msg> {
        use enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent;
        let ConfirmDialogEvent::Resolved(cause) = event else {
            // Checkbox / focus events do not end the presentation.
            return Task::none();
        };
        let Some(action) = self.model.pending_page_action.take() else {
            return Task::none();
        };
        let state = std::mem::replace(
            &mut self.model.page_confirm_state,
            enzui_iced_core::confirm::ConfirmState::new(None),
        );
        if !state.snapshot(cause).is_confirmed() {
            // Declined (Cancel, Escape or scrim).
            self.model.announce("Cancelled");
            return Task::none();
        }
        match action {
            model::PendingPageAction::DeleteAnnotation(id) => self.remove_annotation(id).0,
            model::PendingPageAction::DeletePage(id) => {
                self.delete_page(id);
                Task::none()
            }
            model::PendingPageAction::ExtractPage(id) => {
                self.extract_page(id);
                Task::none()
            }
            model::PendingPageAction::DeletePages(ids) => {
                for id in ids {
                    self.delete_page(id);
                }
                Task::none()
            }
            model::PendingPageAction::ExtractPages(ids) => {
                self.extract_pages(&ids);
                Task::none()
            }
            model::PendingPageAction::ClearMetadata => {
                self.clear_metadata();
                Task::none()
            }
            model::PendingPageAction::RemoveMetadataDates(removal) => {
                self.save_metadata(removal);
                Task::none()
            }
            model::PendingPageAction::SplitPdf => {
                self.split_pdf();
                Task::none()
            }
        }
    }

    // Retain all output custody on partial failure so close remains vetoed until
    // acknowledgement.
    pub(crate) fn split_pdf(&mut self) {
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        let mut debt = plumbing::pdf_ops::ExtractPublicationDebt::default();
        match plumbing::pdf_ops::split_pdf_with_custody(&path, &mut debt) {
            Ok(written) => {
                self.model
                    .announce(format!("Split into {} files", written.len()));
            }
            Err(error) => {
                self.install_extract_publication_debt(
                    debt,
                    crate::user_copy::PdfWriteOperation::Split,
                    &error,
                );
                self.report_pdf_write_failure(crate::user_copy::PdfWriteOperation::Split, error)
            }
        }
    }

    pub(crate) fn delete_page(&mut self, id: u64) {
        if self.model.arrangement.len() <= 1 {
            self.model
                .refuse("A PDF must keep at least one page; nothing was deleted.");
            return;
        }
        let Some((index, entry)) = self.model.arrangement.remove(id) else {
            return;
        };
        let label = entry.label.clone();
        self.model
            .undo_stack
            .record(UndoableAction::DeletePage { index, entry });
        self.model.announce(format!("Deleted {label}"));
    }

    // Merged pages have no source page in the open document; refuse rather than extracting a
    // different page.
    pub(crate) fn extract_page(&mut self, id: u64) {
        let Some(index) = self.model.arrangement.position(id) else {
            return;
        };
        let Some(entry) = self.model.arrangement.get(index) else {
            return;
        };
        let crate::pages::PageSource::Original(source_index) = entry.source else {
            self.model
                .refuse("Only pages from this document can be extracted yet.");
            return;
        };
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        let mut debt = plumbing::pdf_ops::ExtractPublicationDebt::default();
        match plumbing::pdf_ops::extract_page_no_clobber_with_custody(
            &path,
            source_index,
            None,
            &mut debt,
        ) {
            Ok(published) => {
                self.model.announce(format!(
                    "Extracted page to {}",
                    crate::user_copy::path_name(&published)
                ));
            }
            Err(error) => {
                self.install_extract_publication_debt(
                    debt,
                    crate::user_copy::PdfWriteOperation::Extract,
                    &error,
                );
                self.report_pdf_write_failure(crate::user_copy::PdfWriteOperation::Extract, error)
            }
        }
    }

    // Extract the arranger pages `ids` into one new PDF beside this one.
    // Every page must still live in the open document, as for
    // [`Self::extract_page`].
    pub(crate) fn extract_pages(&mut self, ids: &[u64]) {
        let mut indices = Vec::with_capacity(ids.len());
        for id in ids {
            let source = self
                .model
                .arrangement
                .position(*id)
                .and_then(|index| self.model.arrangement.get(index))
                .map(|entry| entry.source.clone());
            match source {
                Some(crate::pages::PageSource::Original(index)) => indices.push(index),
                Some(_) => {
                    self.model
                        .refuse("Only pages from this document can be extracted yet.");
                    return;
                }
                None => {}
            }
        }
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        if indices.is_empty() {
            return;
        }
        let mut debt = plumbing::pdf_ops::ExtractPublicationDebt::default();
        match plumbing::pdf_ops::extract_pages_no_clobber_with_custody(&path, &indices, &mut debt) {
            Ok(published) => {
                self.model.announce(format!(
                    "Extracted {} pages to {}",
                    indices.len(),
                    crate::user_copy::path_name(&published)
                ));
            }
            Err(error) => {
                self.install_extract_publication_debt(
                    debt,
                    crate::user_copy::PdfWriteOperation::Extract,
                    &error,
                );
                self.report_pdf_write_failure(crate::user_copy::PdfWriteOperation::Extract, error)
            }
        }
    }

    // Move a failing extract's or split's [`plumbing::pdf_ops::ExtractPublicationDebt`]
    // onto [`Self::extract_publication_debt`] and install a summary naming the retained
    // count and custody kind without any pathname. Called only on failure.
    pub(crate) fn install_extract_publication_debt(
        &mut self,
        debt: plumbing::pdf_ops::ExtractPublicationDebt,
        operation: crate::user_copy::PdfWriteOperation,
        _error: &plumbing::pdf_ops::PdfWriteError,
    ) {
        let completed_count = debt.completed.len();
        let uncertain_kind = match debt.uncertain.as_ref() {
            Some(plumbing::pdf_ops::ArrangementUncertainAuthority::Durable(_)) => {
                plumbing::pdf_ops::ExtractUncertainKind::Durable
            }
            Some(plumbing::pdf_ops::ArrangementUncertainAuthority::Lost { staged, .. }) => {
                plumbing::pdf_ops::ExtractUncertainKind::Lost {
                    staged: plumbing::pdf_ops::staged_bytes_description(staged),
                }
            }
            None => plumbing::pdf_ops::ExtractUncertainKind::NotPublished,
        };
        let op_label = match operation {
            crate::user_copy::PdfWriteOperation::Split => "split",
            _ => "extract",
        };
        let uncertain_label = match &uncertain_kind {
            plumbing::pdf_ops::ExtractUncertainKind::Durable => "durability uncertain",
            plumbing::pdf_ops::ExtractUncertainKind::Lost { .. } => "identity lost",
            plumbing::pdf_ops::ExtractUncertainKind::NotPublished => "no destination published",
        };
        // No pathnames: the raw diagnostic contains the failing candidate's path.
        let message = format!(
            "An {op_label} publication left {completed_count} retained output(s) and one failing output ({uncertain_label}); no pathname is exposed. Dismiss abandons the retained handles without cleaning up the on-disk files.",
        );
        self.model.extract_uncertain_publication = Some(model::ExtractUncertainPublication {
            completed_count,
            uncertain_kind,
            message,
        });
        self.extract_publication_debt = Some(debt);
    }

    pub(crate) fn clear_metadata(&mut self) {
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        self.model.metadata_save_failure = None;
        match plumbing::pdf_ops::clear_metadata(&path) {
            Ok(()) => {
                self.model.metadata = plumbing::pdf_ops::PdfMetadata::default();
                self.model.metadata_baseline = plumbing::pdf_ops::PdfMetadata::default();
                self.model.error = None;
                self.model.announce("Metadata cleared");
            }
            Err(error) if error.was_published() => {
                self.model.metadata = plumbing::pdf_ops::PdfMetadata::default();
                self.report_pdf_write_failure(
                    crate::user_copy::PdfWriteOperation::ClearMetadata,
                    error,
                );
            }
            Err(error) => self.report_pdf_write_failure(
                crate::user_copy::PdfWriteOperation::ClearMetadata,
                error,
            ),
        }
    }

    pub(crate) fn merge_pdf(&mut self) {
        let raw = self.model.merge_path.trim().to_string();
        if raw.is_empty() {
            self.model.refuse("Choose a PDF to append.");
            return;
        }
        let path = PathBuf::from(&raw);
        let name = crate::user_copy::path_name(&path);
        if !path.is_file() {
            self.model.refuse(format!("File missing: {name}"));
            return;
        }
        // One pathname read yields both the page count and the identity, so a
        // substitution between two reads cannot pair one file's count with another's
        // fingerprint.
        let snapshot = match plumbing::pdf_ops::read_document_annotations(&path) {
            Ok(snapshot) => snapshot,
            Err(diagnostic) => {
                eprintln!(
                    "reader-app merged PDF identity failure: path={path:?} diagnostic={diagnostic:?}"
                );
                self.model
                    .refuse(format!("Could not verify {name} before adding it."));
                return;
            }
        };
        let count = snapshot.pages.len();
        if count == 0 {
            self.model.refuse(format!("{name} has no pages."));
            return;
        }
        let identity = snapshot.document_fingerprint;
        let source = path.to_string_lossy().into_owned();
        // Refuse a duplicate merge source before retention: `publish_arrangement` would
        // collapse ordered duplicates and the intent is ambiguous.
        let already_merged = self.model.arrangement.entries().iter().any(|entry| {
            matches!(
                &entry.source,
                crate::pages::PageSource::Merged { document_fingerprint, .. }
                    if document_fingerprint == &identity
            )
        });
        if already_merged {
            self.model.refuse(format!(
                "{name} was already merged; each source may appear once."
            ));
            return;
        }
        let entries = self
            .model
            .arrangement
            .append_merged(&source, count, &identity);
        self.model.undo_stack.record(UndoableAction::MergePages {
            file: name.clone(),
            entries,
        });
        self.model
            .announce(format!("Merged {count} pages from {name}"));
        self.model.merge_path.clear();
    }

    pub(crate) fn begin_arrangement_publication(&mut self, destination: PathBuf) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        let Some(source) = self.model.file_path.clone() else {
            self.model
                .refuse("No source PDF is open; the draft was retained.");
            return Task::none();
        };
        if self.model.arrangement.is_empty() {
            self.model
                .refuse("A PDF must keep at least one page; the draft was retained.");
            return Task::none();
        }
        let Some(expected_fingerprint) = self
            .model
            .pdf_annotation_snapshot
            .as_ref()
            .map(|snapshot| snapshot.document_fingerprint.clone())
        else {
            self.model
                .refuse("The source PDF identity is still being verified; the draft was retained.");
            return Task::none();
        };

        let generation = self.model.arrangement_publish_generation.wrapping_add(1);
        self.model.arrangement_publish_generation = generation;
        let arrangement = self.model.arrangement.clone();
        self.model.arrangement_publication = Some(model::PageArrangementPublication {
            generation,
            source: source.clone(),
            destination: destination.clone(),
            arrangement: arrangement.clone(),
        });
        self.model.arrangement_publish_failure = None;
        self.model.error = None;

        // Handoff slot for a Save As `create_new` that ends `PublishedButNotDurable` or
        // `Lost`. In-place publication never fills it, but it is always installed so
        // `finish_arrangement_publication` need not branch on the destination.
        let custody_slot: crate::plumbing::pdf_ops::ArrangementUncertainSlot =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        self.arrangement_publication_slot = Some(std::sync::Arc::clone(&custody_slot));

        let task_source = source.clone();
        let task_destination = destination.clone();
        let task_arrangement = arrangement.clone();
        let worker_slot = std::sync::Arc::clone(&custody_slot);
        self.dispatch_blocking(move || {
            plumbing::pdf_ops::publish_arrangement_with_custody(
                &task_source,
                &expected_fingerprint,
                task_arrangement.entries(),
                &task_destination,
                Some(worker_slot),
            )
        })
        .map(move |worker_result| Msg::PageArrangementPublished {
            generation,
            source: source.clone(),
            destination: destination.clone(),
            arrangement: arrangement.clone(),
            result: worker_result.unwrap_or_else(|error| {
                Err(format!("arranger publication worker failed: {error}").into())
            }),
        })
    }

    pub(crate) fn finish_arrangement_publication(
        &mut self,
        generation: u64,
        source: PathBuf,
        destination: PathBuf,
        arrangement: crate::pages::PageArrangement,
        result: Result<plumbing::pdf_ops::ArrangementPublication, plumbing::pdf_ops::PdfWriteError>,
    ) -> Task<Msg> {
        // Drain the slot first so a handed-off authority is available to both branches;
        // dropping it stops a stale completion from writing into it.
        let handed_off_authority = self
            .arrangement_publication_slot
            .take()
            .and_then(|slot| slot.lock().ok().and_then(|mut guard| guard.take()));
        let is_current = self
            .model
            .arrangement_publication
            .as_ref()
            .is_some_and(|pending| {
                pending.generation == generation
                    && pending.source == source
                    && pending.destination == destination
                    && pending.arrangement == arrangement
                    && self.model.file_path.as_ref() == Some(&source)
                    && self.model.arrangement == arrangement
            });
        if !is_current {
            if self
                .model
                .arrangement_publication
                .as_ref()
                .is_some_and(|pending| pending.generation == generation)
            {
                self.model.arrangement_publication = None;
            }
            // Cancellation after accepted work is uncertainty: the destination may carry
            // bytes the model no longer names, so install a debt and veto close.
            if let Ok(publication) = &result {
                let destination_label = crate::user_copy::path_name(&destination);
                let message = format!(
                    "An arranged PDF ({} page(s)) was published to {} after the source document was closed. Verify the file before opening it.",
                    publication.page_count, destination_label,
                );
                self.model.error = Some(message.clone());
                self.model.arrangement_uncertain_publication =
                    Some(model::ArrangementUncertainPublication {
                        destination: destination.clone(),
                        page_count: publication.page_count,
                        message,
                        custody_kind: crate::plumbing::pdf_ops::ArrangementUncertainKind::Durable,
                    });
            }
            return Task::none();
        }
        self.model.arrangement_publication = None;

        let publication = match result {
            Ok(publication) => publication,
            Err(error) => {
                eprintln!(
                    "reader-app arranged PDF publication failure: source={source:?} destination={destination:?} diagnostic={error:?}"
                );
                let destination_label = crate::user_copy::path_name(&destination);
                let message = if error.was_published() && destination == source {
                    format!(
                        "An in-place arrangement of {destination_label} reached the disk, but durability or reopen verification failed. The reversible draft is retained; reopen the source before retrying."
                    )
                } else if error.was_published() {
                    format!(
                        "The arranged bytes reached {}, but durability or reopen verification failed. The source is unchanged and the reversible draft is retained; verify the destination before retrying.",
                        destination.display(),
                    )
                } else {
                    format!(
                        "Could not publish the arranged PDF to {destination_label}. No destination was published; the source and reversible draft are retained."
                    )
                };
                self.model.arrangement_publish_failure = Some(message.clone());
                self.model.error = Some(message);
                // Retain the handed-off authority and install the typed summary, so the
                // banner names the custody variant and close-drain vetoes.
                if error.was_published()
                    && let Some(authority) = handed_off_authority
                {
                    let custody_kind = match &authority {
                        crate::plumbing::pdf_ops::ArrangementUncertainAuthority::Durable(_) => {
                            crate::plumbing::pdf_ops::ArrangementUncertainKind::Durable
                        }
                        crate::plumbing::pdf_ops::ArrangementUncertainAuthority::Lost {
                            staged,
                            ..
                        } => crate::plumbing::pdf_ops::ArrangementUncertainKind::Lost {
                            staged: crate::plumbing::pdf_ops::staged_bytes_description(staged),
                        },
                    };
                    self.arrangement_uncertain_authority = Some(authority);
                    self.model.arrangement_uncertain_publication =
                        Some(model::ArrangementUncertainPublication {
                            destination: destination.clone(),
                            page_count: 0,
                            message: self
                                .model
                                .arrangement_publish_failure
                                .clone()
                                .unwrap_or_default(),
                            custody_kind,
                        });
                }
                return Task::none();
            }
        };

        self.remap_annotations_after_publication(&arrangement, &publication.annotation_snapshot);
        self.save_position();
        self.model.file_path = Some(destination.clone());
        self.save_annotations();
        self.model.arrangement_publish_failure = None;
        self.model.announce(format!(
            "Saved {} pages to {}",
            publication.page_count,
            crate::user_copy::path_name(&destination)
        ));
        self.open(destination)
    }

    pub(crate) fn remap_annotations_after_publication(
        &mut self,
        arrangement: &crate::pages::PageArrangement,
        snapshot: &plumbing::pdf_ops::PdfAnnotationSnapshot,
    ) {
        let page_map = arrangement
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(published, entry)| match entry.source {
                crate::pages::PageSource::Original(original) => Some((original, published)),
                crate::pages::PageSource::Merged { .. } => None,
            })
            .collect::<std::collections::HashMap<_, _>>();
        let annotations = std::sync::Arc::make_mut(&mut self.model.annotations);
        annotations.annotations.retain_mut(|annotation| {
            let Some(&published_page) = page_map.get(&annotation.page) else {
                return false;
            };
            annotation.page = published_page;
            if let persistence::AnnotationPosition::PdfRect { x, y, w, h, .. } =
                persistence::AnnotationPosition::parse(&annotation.position, published_page)
            {
                annotation.position = persistence::AnnotationPosition::PdfRect {
                    page: published_page,
                    x,
                    y,
                    w,
                    h,
                }
                .to_stored();
            }
            if let Some(binding) = annotation.pdf_binding.as_mut() {
                binding.document_fingerprint = snapshot.document_fingerprint.clone();
                if let Some(page) = snapshot.pages.get(published_page) {
                    binding.page_fingerprint = page.fingerprint.clone();
                }
                if let Some(native) = snapshot.annotations.iter().find(|native| {
                    native.page == published_page && native.native_id == binding.native_id
                }) {
                    binding.native_fingerprint = native.fingerprint.clone();
                }
            }
            true
        });
        annotations.bookmarks.retain_mut(|bookmark| {
            let Some(&published_page) = page_map.get(&bookmark.page) else {
                return false;
            };
            bookmark.page = published_page;
            true
        });
    }
}

impl App {
    // Open `Protect with Password…`, or `Remove Password…` when the open
    // PDF was unlocked with one.
    pub(crate) fn open_pdf_password_form(&mut self) -> Task<Msg> {
        if self.model.format != Format::Pdf || self.model.file_path.is_none() {
            return Task::none();
        }
        let purpose = if self.model.pdf_password.is_some() {
            model::PasswordPurpose::Remove
        } else {
            model::PasswordPurpose::Protect
        };
        self.model.pdf_password_form = Some(model::PdfPasswordForm {
            purpose,
            presenter: crate::view::pdf_password_dialog::present(purpose),
            error: None,
        });
        crate::overlay_focus::open(&mut self.model, model::Overlay::PdfPassword)
    }

    pub(crate) fn pdf_password_form_event(
        &mut self,
        event: enzui_iced_widgets::form_dialog::FormDialogEvent,
    ) -> Task<Msg> {
        use enzui_iced_widgets::form_dialog::FormOutcome;
        let Some(form) = self.model.pdf_password_form.as_mut() else {
            return Task::none();
        };
        let (outcome, task) = form.presenter.update::<Msg>(event, None);
        match outcome {
            Some(FormOutcome::Submitted(submission)) => {
                let purpose = form.purpose;
                let text = |id| submission.text(&id).unwrap_or_default().to_owned();
                let password = text(crate::view::pdf_password_dialog::password_field());
                let confirm = text(crate::view::pdf_password_dialog::confirm_field());
                Task::batch([task, self.apply_pdf_password(purpose, password, confirm)])
            }
            Some(FormOutcome::Dismissed(_)) => {
                self.model.pdf_password_form = None;
                Task::batch([
                    task,
                    crate::overlay_focus::dismiss(
                        &mut self.model,
                        enzui_iced_core::modal::DismissCause::CancelAction,
                    ),
                ])
            }
            None => task,
        }
    }

    fn apply_pdf_password(
        &mut self,
        purpose: model::PasswordPurpose,
        password: String,
        confirm: String,
    ) -> Task<Msg> {
        use crate::view::pdf_password_dialog::MIN_PASSWORD_CHARS;
        let refused = match purpose {
            model::PasswordPurpose::Protect if password.chars().count() < MIN_PASSWORD_CHARS => {
                Some(format!("Use at least {MIN_PASSWORD_CHARS} characters."))
            }
            model::PasswordPurpose::Protect if password != confirm => {
                Some("The passwords don't match.".to_string())
            }
            _ => None,
        };
        let result = match (refused, self.model.file_path.clone()) {
            (Some(reason), _) => Err(reason),
            (None, None) => Err("No PDF is open.".to_string()),
            (None, Some(path)) => match purpose {
                model::PasswordPurpose::Protect => {
                    plumbing::pdf_ops::set_password(&path, &password)
                }
                model::PasswordPurpose::Remove => {
                    plumbing::pdf_ops::remove_password(&path, &password)
                }
            }
            .map_err(|error| error.to_string()),
        };
        match result {
            Ok(()) => {
                // Pages rendered from here on need the new password, or none.
                self.model.pdf_password = match purpose {
                    model::PasswordPurpose::Protect => Some(password),
                    model::PasswordPurpose::Remove => None,
                };
                self.model.pdf_password_form = None;
                self.model.announce(match purpose {
                    model::PasswordPurpose::Protect => "The PDF now asks for its password",
                    model::PasswordPurpose::Remove => "The PDF no longer has a password",
                });
                crate::overlay_focus::dismiss(
                    &mut self.model,
                    enzui_iced_core::modal::DismissCause::DefaultAction,
                )
            }
            Err(reason) => {
                self.model.pdf_password_form = Some(model::PdfPasswordForm {
                    purpose,
                    presenter: crate::view::pdf_password_dialog::present(purpose),
                    error: Some(reason),
                });
                Task::none()
            }
        }
    }
}
