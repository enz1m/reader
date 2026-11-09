//! Persistence tasks, recovery, and close drain.

use super::classify::refresh_projection;
use super::*;

use iced::Task;

impl App {
    pub(crate) fn persistence_task(&self, pending: PendingPersistence) -> Task<Msg> {
        let queue = self.persistence_queue.clone();
        let resolver = self.xdg_resolver.clone();
        let ticket = pending.ticket.clone();
        // Serialisation and atomic writes run on the bounded owner; per-destination FIFO
        // order stays in `PersistenceQueue`.
        self.dispatch_blocking(move || Self::execute_persistence(&queue, &resolver, &pending))
            .map(move |result| Msg::PersistenceFinished {
                destination: ticket.destination().to_string(),
                generation: ticket.generation(),
                outcome: result.unwrap_or_else(|error| {
                    persistence::PersistenceOutcome::Error(format!(
                        "persistence worker failed: {error}"
                    ))
                }),
            })
    }

    pub(crate) fn execute_persistence(
        queue: &persistence::PersistenceQueue,
        _resolver: &XdgResolver,
        pending: &PendingPersistence,
    ) -> persistence::PersistenceOutcome {
        queue.execute(&pending.ticket, || match &pending.snapshot {
            PersistenceSnapshot::Library { state, store } => store
                .save_blocking(state)
                .map_err(|error| error.diagnostic().to_string()),
            PersistenceSnapshot::Annotations { file, store, .. } => store
                .save_blocking(file)
                .map_err(|error| error.diagnostic().to_string()),
        })
    }

    // Only the current generation can settle debt; superseded writes cannot erase a failure.
    pub(crate) fn record_persistence_completion(
        &mut self,
        destination: &str,
        generation: u64,
        outcome: persistence::PersistenceOutcome,
    ) {
        use persistence::PersistenceOutcome as Outcome;
        let matches_current = self
            .persistence_debt
            .get(destination)
            .is_some_and(|pending| pending.ticket.generation() == generation);
        let saved_bookmarks = if matches_current && matches!(&outcome, Outcome::Saved) {
            self.persistence_debt
                .get(destination)
                .and_then(|pending| match &pending.snapshot {
                    PersistenceSnapshot::Annotations {
                        source_path, file, ..
                    } => Some((
                        persistence::canonical_key(source_path),
                        file.bookmarks.len(),
                    )),
                    PersistenceSnapshot::Library { .. } => None,
                })
        } else {
            None
        };
        match outcome {
            Outcome::Saved if matches_current => {
                self.persistence_debt.remove(destination);
            }
            Outcome::Error(diagnostic) if matches_current => {
                let summary =
                    enzui_iced_core::persist::PersistError::new(diagnostic.clone()).user_summary();
                // The raw diagnostic goes only to the structured log, with destination and
                // generation for correlation.
                eprintln!(
                    "reader-app persistence failure: destination={destination:?} \
                 generation={generation} diagnostic={diagnostic:?}"
                );
                if let Some(entry) = self.persistence_debt.get_mut(destination) {
                    entry.status = DebtStatus::Failed {
                        summary,
                        diagnostic,
                    };
                }
            }
            // Superseded is quiet by contract; stale Saved or Error results land here too.
            _ => {}
        }
        if let Some((path, count)) = saved_bookmarks
            && self.model.bookmark_counts.insert(path.clone(), count) != Some(count)
        {
            self.model
                .apply_bookmark_counts(&std::collections::HashMap::from([(path, count)]));
        }
        self.recompute_persistence_alert();
    }

    // Re-issue every failed destination under a fresh ticket generation, writing the
    // exact retained snapshot. Library and annotation destinations dispatch as
    // independent tasks.
    pub(crate) fn retry_persistence(&mut self) -> Task<Msg> {
        let mut prepared: Vec<PendingPersistence> = Vec::new();
        for entry in self.persistence_debt.values_mut() {
            if matches!(entry.status, DebtStatus::Failed { .. }) {
                entry.ticket = self
                    .persistence_queue
                    .register(entry.ticket.destination().to_string());
                entry.status = DebtStatus::InFlight;
                prepared.push(entry.clone());
            }
        }
        self.recompute_persistence_alert();
        Task::batch(
            prepared
                .into_iter()
                .map(|pending| self.persistence_task(pending)),
        )
    }

    pub(crate) fn apply_startup_library_restore(
        &mut self,
        generation: u64,
        base_revision: u64,
        open_path: Option<PathBuf>,
        reload: persistence::RecoveryReload,
    ) -> Task<Msg> {
        if generation != self.library_recovery_generation {
            return Task::none();
        }
        let persistence::RecoveryReload::Library { store, outcome } = reload else {
            return Task::none();
        };
        self.library_store = store;
        let unavailable = outcome.is_unavailable();
        let (mut state, recovery) = outcome.into_parts();
        if self.library_state_revision != base_revision {
            for path in &self.library_removed_during_restore {
                state.files.remove(path);
            }
            for (path, record) in &self.model.app_state.files {
                state.files.insert(path.clone(), record.clone());
            }
        }
        self.library_removed_during_restore.clear();
        self.model.app_state = std::sync::Arc::new(state);
        self.model.library_loading = false;
        self.model
            .set_library_persistence_unavailable(unavailable, self.persist_enabled);
        self.model
            .recovery_authorities
            .retain(|authority| authority.origin != persistence::StateOrigin::Library);
        if let Some(recovery) = recovery {
            self.model.recovery_authorities.push(recovery);
        }
        refresh_projection(&mut self.model);
        let thumbnails = self.thumbnail_tasks();
        let open = self
            .pending_launch_document
            .take()
            .or(open_path)
            .map_or_else(Task::none, |path| self.open(path));
        Task::batch([thumbnails, self.request_availability_scan(), open])
    }

    pub(crate) fn advance_recovery_generation(&mut self, origin: &persistence::StateOrigin) -> u64 {
        let generation = match origin {
            persistence::StateOrigin::Library => &mut self.library_recovery_generation,
            persistence::StateOrigin::Annotations { .. } => {
                &mut self.annotation_recovery_generation
            }
        };
        *generation = (*generation).wrapping_add(1);
        *generation
    }

    pub(crate) fn recovery_generation_for(&self, reload: &persistence::RecoveryReload) -> u64 {
        match reload {
            persistence::RecoveryReload::Library { .. } => self.library_recovery_generation,
            persistence::RecoveryReload::Annotations { .. } => self.annotation_recovery_generation,
        }
    }

    pub(crate) fn apply_recovery_reload(
        &mut self,
        reload: persistence::RecoveryReload,
    ) -> Task<Msg> {
        match reload {
            persistence::RecoveryReload::Library { store, outcome } => {
                self.library_store = store;
                let unavailable = outcome.is_unavailable();
                let (state, recovery) = outcome.into_parts();
                self.model.app_state = std::sync::Arc::new(state);
                self.model.library_loading = false;
                self.model
                    .set_library_persistence_unavailable(unavailable, self.persist_enabled);
                self.library_removed_during_restore.clear();
                self.model
                    .recovery_authorities
                    .retain(|authority| authority.origin != persistence::StateOrigin::Library);
                if let Some(recovery) = recovery {
                    self.model.recovery_authorities.push(recovery);
                }
                refresh_projection(&mut self.model);
            }
            persistence::RecoveryReload::Annotations {
                source_path,
                source_key,
                destination,
                store,
                outcome,
            } => {
                let Some(current) = self
                    .model
                    .file_path
                    .clone()
                    .filter(|path| path == Path::new(&source_path))
                else {
                    return Task::none();
                };
                self.annotation_store = destination.map(|destination| AnnotationCustody {
                    source_path: current,
                    source_key: source_key.clone(),
                    destination,
                    store,
                });
                let unavailable = outcome.is_unavailable();
                let (annotations, recovery) = outcome.into_parts();
                self.model.annotations = std::sync::Arc::new(annotations);
                self.model.annotations_loading = false;
                self.model.annotations_unavailable = unavailable || recovery.is_some();
                self.annotation_state_revision = 0;
                self.model.recovery_authorities.retain(|authority| {
                    !matches!(
                        &authority.origin,
                        persistence::StateOrigin::Annotations { source_path: authority_source }
                            if Path::new(authority_source) == Path::new(&source_path)
                    )
                });
                if let Some(recovery) = recovery {
                    self.model.recovery_authorities.push(recovery);
                }
                let bookmark_count = self.model.annotations.bookmarks.len();
                if self
                    .model
                    .bookmark_counts
                    .insert(source_key.clone(), bookmark_count)
                    != Some(bookmark_count)
                {
                    self.model
                        .apply_bookmark_counts(&std::collections::HashMap::from([(
                            source_key,
                            bookmark_count,
                        )]));
                }
                if self.model.format == Format::Epub {
                    if let Some(epub) = self.model.epub.as_mut() {
                        epub.chapter_paragraphs.clear();
                    }
                    self.ensure_chapter_paragraphs();
                }
                if let Some(snapshot) = self.model.pdf_annotation_snapshot.clone() {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
            }
        }
        self.model.revealed_recovery_location = None;
        Task::none()
    }

    pub(crate) fn start_empty_state(&mut self, origin: persistence::StateOrigin) -> Task<Msg> {
        if self.load_controller.is_loading() && self.active_load_origin.as_ref() == Some(&origin) {
            self.load_controller.invalidate();
        }
        self.advance_recovery_generation(&origin);
        let Some(authority) = self
            .model
            .recovery_authorities
            .iter()
            .find(|authority| authority.origin == origin)
            .cloned()
        else {
            return Task::none();
        };
        if let Some(refusal) = authority.start_empty_refusal() {
            self.model.refuse(refusal);
            return Task::none();
        }
        match &origin {
            persistence::StateOrigin::Library => {
                self.library_store.authorize_replacement();
                self.model.app_state = std::sync::Arc::new(persistence::empty_app_state());
                self.model.library_loading = false;
                self.library_removed_during_restore.clear();
                self.progress_dirty = true;
                refresh_projection(&mut self.model);
            }
            persistence::StateOrigin::Annotations { source_path } => {
                let Some(custody) = self
                    .annotation_store
                    .as_ref()
                    .filter(|custody| custody.holds_origin(source_path))
                else {
                    return Task::none();
                };
                custody.store.authorize_replacement();
                self.model.annotations =
                    std::sync::Arc::new(persistence::empty_annotations(source_path));
                self.model.annotations_loading = false;
                self.model.annotations_unavailable = false;
                if let Some(snapshot) = self.model.pdf_annotation_snapshot.clone() {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
                self.annotations_dirty = true;
            }
        }
        self.model
            .recovery_authorities
            .retain(|authority| authority.origin != origin);
        self.model.revealed_recovery_location = None;
        self.flush_persistence()
    }

    // Refresh `model.persistence_alert` from the current debt map. Set to
    // `Some(bounded_safe_summary)` when at least one destination is in
    // [`DebtStatus::Failed`]; `None` otherwise.
    pub(crate) fn recompute_persistence_alert(&mut self) {
        let failed_summary = self
            .persistence_debt
            .values()
            .find_map(|entry| match &entry.status {
                DebtStatus::Failed { summary, .. } => Some(summary.clone()),
                DebtStatus::InFlight => None,
            });
        if self.model.persistence_alert != failed_summary {
            self.model.persistence_alert_hidden = false;
        }
        self.model.persistence_alert = failed_summary;
    }

    // Whether the debt map contains any `Failed` entry: the shell raises the persistence
    // banner and close vetoes on it.
    pub(crate) fn has_persistence_failure(&self) -> bool {
        self.persistence_debt
            .values()
            .any(|entry| matches!(entry.status, DebtStatus::Failed { .. }))
    }

    #[cfg(test)]
    pub(crate) fn persistence_failure_diagnostics(&self) -> Vec<String> {
        self.persistence_debt
            .values()
            .filter_map(|entry| match &entry.status {
                DebtStatus::Failed { diagnostic, .. } => Some(diagnostic.clone()),
                DebtStatus::InFlight => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn drain_persistence_debt_for_test(&mut self) -> Task<Msg> {
        let inflight: Vec<PendingPersistence> = self
            .persistence_debt
            .values()
            .filter(|p| matches!(p.status, DebtStatus::InFlight))
            .cloned()
            .collect();
        let mut tasks = Vec::new();
        for pending in inflight {
            let outcome =
                Self::execute_persistence(&self.persistence_queue, &self.xdg_resolver, &pending);
            tasks.push(self.update(Msg::PersistenceFinished {
                destination: pending.ticket.destination().to_string(),
                generation: pending.ticket.generation(),
                outcome,
            }));
        }
        Task::batch(tasks)
    }

    pub(crate) fn begin_close_drain(&mut self, window: Option<iced::window::Id>) -> Task<Msg> {
        match &mut self.close_state {
            CloseState::Draining {
                window: current, ..
            } => {
                if current.is_none() && window.is_some() {
                    *current = window;
                }
                return Task::none();
            }
            CloseState::Vetoed { .. } => {
                return Task::none();
            }
            CloseState::Committed { .. } => return Task::none(),
            CloseState::Idle => {}
        }
        self.save_position();
        self.cancel_expendable_close();
        let persistence = self.flush_persistence();
        let deadline = std::time::Instant::now() + CLOSE_DRAIN_BUDGET;
        self.close_state = CloseState::Draining { window, deadline };
        let close = self.check_close_drain_completion();
        Task::batch([persistence, close])
    }

    pub(crate) fn retry_close(&mut self) -> Task<Msg> {
        let CloseState::Vetoed { window, .. } = self.close_state.clone() else {
            return Task::none();
        };
        let deadline = std::time::Instant::now() + CLOSE_DRAIN_BUDGET;
        self.close_state = CloseState::Draining { window, deadline };
        let retry = self.retry_persistence();
        let recheck = self.check_close_drain_completion();
        Task::batch([retry, recheck])
    }

    pub(crate) fn cancel_expendable_close(&mut self) {
        self.picker
            .cancel(enzui_iced_core::portal::CancelReason::ParentClosed);
        if let Some(pick) = self.pending_pick.take() {
            pick.cancel
                .cancel(enzui_iced_core::portal::CancelReason::ParentClosed);
        }
        if let Some(token) = self.availability.cancel.take() {
            // Cancellation does not release capacity until the admitted closure exits.
            token.cancel(cancellation::CancelCause::Cancelled);
        }
        self.availability.active = false;
        self.availability.pending = false;
        self.preview_pending.clear();
        self.busy_retry_pending = false;
    }

    pub(crate) fn check_close_drain_completion(&mut self) -> Task<Msg> {
        let CloseState::Draining { window, deadline } = self.close_state else {
            return Task::none();
        };
        // Claim a late authority handoff before the uncertainty gate decides.
        self.finalize_writer_authority_handoff();
        self.finalize_retry_authority_handoff();
        let now = std::time::Instant::now();
        let pool_live = self.blocking_dispatcher.snapshot().live();
        // Accepted or retained PDF mutations and uncertain publications block a clean
        // close; none may be discarded or presented as unchanged.
        let writer_still_running = self.pdf_mutation_writer_still_running();
        let retry_still_running = self.pdf_mutation_retry_still_running();
        // Retained arrangement and metadata debts survive document swaps and block a clean
        // close the same way.
        let drain_clean = pool_live == 0
            && !self.has_persistence_failure()
            && !self.progress_dirty
            && !self.annotations_dirty
            && self.persistence_debt.is_empty()
            && self.pdf_mutation_active.is_none()
            && self.pdf_mutation_busy_retry.is_none()
            && self.pdf_mutation_pending.is_none()
            && self.pdf_mutation_uncertain.is_none()
            && self.model.arrangement_publication.is_none()
            && self.model.arrangement_publish_failure.is_none()
            && self.model.arrangement_uncertain_publication.is_none()
            && self.arrangement_uncertain_authority.is_none()
            && self.arrangement_publication_slot.is_none()
            && self.extract_publication_debt.is_none()
            && self.model.extract_uncertain_publication.is_none()
            && self.model.metadata_save_failure.is_none()
            && !writer_still_running
            && !retry_still_running;
        if drain_clean {
            let target = window.or(self.main_window);
            self.close_state = CloseState::Committed { window: target };
            #[cfg(feature = "center")]
            self.shutdown_center();
            return match target {
                Some(id) => iced::window::close(id),
                None => Task::none(),
            };
        }
        if now >= deadline {
            let summary = self.compose_close_veto_summary();
            // The shell shows the bounded summary; per-destination diagnostics stay in the
            // structured log. The window id is retained for `Msg::RetryClose`.
            self.model.error = Some(summary.clone());
            self.model.persistence_alert_hidden = false;
            self.close_state = CloseState::Vetoed { window, summary };
        }
        Task::none()
    }

    // Bounded visible summary for a vetoed close, by precedence: persistence failure,
    // uncertainty, in-flight or queued PDF mutation, then pool occupancy.
    pub(crate) fn compose_close_veto_summary(&self) -> String {
        if self.has_persistence_failure() {
            return "Saving did not finish before the close deadline.".to_owned();
        }
        if let Some(debt) = self.pdf_mutation_uncertain.as_ref() {
            // Retry rechecks the file; Dismiss hides the banner but keeps the record;
            // doing nothing keeps the window open.
            return format!(
                "{} Retry to recheck the exact file, or Dismiss to hide the warning; either way the exact file will be rechecked before another native write. Leave the warning in place to keep the window open.",
                debt.summary
            );
        }
        if let Some(debt) = self.model.arrangement_uncertain_publication.as_ref() {
            return debt.message.clone();
        }
        if let Some(debt) = self.model.extract_uncertain_publication.as_ref() {
            return debt.message.clone();
        }
        if let Some(message) = self.model.arrangement_publish_failure.as_ref() {
            return message.clone();
        }
        if let Some(failure) = self.model.metadata_save_failure.as_ref() {
            return failure.message.clone();
        }
        if self
            .pdf_mutation_writer_running
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::Acquire))
        {
            return WRITER_OCCUPANCY_BLOCKED_ADMIT_SUMMARY.to_owned();
        }
        if self
            .pdf_mutation_retry_running
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::Acquire))
        {
            return RETRY_OCCUPANCY_BLOCKED_ADMIT_SUMMARY.to_owned();
        }
        if self.pdf_mutation_active.is_some()
            || self.pdf_mutation_busy_retry.is_some()
            || self.pdf_mutation_pending.is_some()
        {
            return "A PDF annotation mutation was still running when the close deadline expired."
                .to_owned();
        }
        let pool_live = self.blocking_dispatcher.snapshot().live();
        if pool_live > 0 {
            return format!(
                "{pool_live} background operation(s) were still running when \
             the close deadline expired."
            );
        }
        "A pending save did not finish before the close deadline.".to_owned()
    }
}
