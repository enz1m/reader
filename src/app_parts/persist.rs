//! Progress persistence, availability, and blocking tasks.

use super::classify::refresh_projection;
use super::*;

use iced::Task;

impl App {
    pub(crate) fn persist_progress(
        &mut self,
        path: &Path,
        title: String,
        author: String,
        format: &str,
    ) {
        let key = persistence::canonical_key(&path.to_string_lossy());

        // Bank this session's elapsed time and reset the clock so it is not counted twice.
        let prev_time = self
            .model
            .app_state
            .files
            .get(&key)
            .and_then(|r| r.reading_time_secs)
            .unwrap_or(0);
        let now = persistence::now_unix_secs();
        let reading_time_secs =
            persistence::accumulate_reading_time(prev_time, self.model.session_start_secs, now);
        if self.model.session_start_secs != 0 {
            self.model.session_start_secs = now;
        }

        let record = ProgressRecord {
            page: self.model.current_position(),
            total: self.model.page_count,
            epub_font_size: Some(self.model.epub_font_size),
            epub_line_spacing: Some(self.model.epub_line_spacing),
            epub_theme: Some(self.model.reading_theme.id().to_string()),
            epub_view_mode: Some(self.model.epub_view_mode.id().to_string()),
            epub_anchor: self.current_epub_progress_anchor(),
            last_opened: persistence::current_timestamp(),
            title,
            author,
            format: format.to_string(),
            reading_time_secs: Some(reading_time_secs),
            ..Default::default()
        };
        // Persistence is deferred, but navigation must see the latest position.
        persistence::set_progress(
            std::sync::Arc::make_mut(&mut self.model.app_state),
            &key,
            record,
        );
        self.library_state_revision = self.library_state_revision.saturating_add(1);
        self.library_removed_during_restore.remove(&key);
        self.progress_dirty = true;
    }

    // Request an availability scan. Bumps the generation and cancels the prior
    // chain; coalesces into one pending request while a chain is active, so a
    // navigation burst admits at most one active plus one pending scan.
    pub(crate) fn request_availability_scan(&mut self) -> Task<Msg> {
        self.availability.generation = self.availability.generation.wrapping_add(1);
        if let Some(previous) = self.availability.cancel.take() {
            previous.cancel(cancellation::CancelCause::Cancelled);
        }
        if self.availability.active {
            self.availability.pending = true;
            return Task::none();
        }
        self.begin_availability_chain()
    }

    // Begin a fresh chain at batch 0, pinning the current library ordering. An
    // empty library retires ownership synchronously instead of leaving a chain
    // that can never complete.
    pub(crate) fn begin_availability_chain(&mut self) -> Task<Msg> {
        self.availability.pending = false;
        if self.model.library.is_empty() {
            self.availability.active = false;
            self.availability.cancel = None;
            return Task::none();
        }
        self.availability.active = true;
        self.availability.revision = self.model.order_revision;
        let token = cancellation::CancelToken::new();
        self.availability.cancel = Some(token.share());
        self.dispatch_availability_batch(self.availability.generation, 0, token)
    }

    // Mint one fresh submission generation for the shared dispatcher.
    //
    // Independent of app-level generation counters; it attributes Busy, Cancelled or
    // Panicked observations to one submission.
    pub(crate) fn next_blocking_generation(&self) -> u64 {
        self.blocking_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1)
    }

    // Submit one synchronous closure to the shared bounded owner.
    //
    // Synchronous submission: wrapping in `async` would keep the work on the executor.
    // The per-submission generation keeps one settlement from answering another.
    fn submit_blocking_task<T, F>(&self, work: F) -> enzui_iced_core::blocking_task::BlockingTask<T>
    where
        T: Send + 'static,
        F: FnOnce(enzui_iced_core::blocking_task::BlockingTaskContext) -> T + Send + 'static,
    {
        let generation = self.next_blocking_generation();
        self.blocking_dispatcher.submit(generation, work)
    }

    // Submit a context-free closure and forward the typed [`BlockingTaskError`]
    // unchanged, so callers can tell a retryable `Busy` from `Cancelled` / `Shutdown` /
    // `Panicked`.
    pub(crate) fn dispatch_blocking<T, F>(&self, work: F) -> Task<Result<T, BlockingTaskError>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let task = self.submit_blocking_task(move |_context| work());
        Task::future(async move { task.await.into_result() })
    }

    // Check cancellation between books; worker failure retires retryably without skipping
    // unprocessed paths.
    pub(crate) fn dispatch_availability_batch(
        &self,
        generation: u64,
        start: usize,
        token: cancellation::CancelToken,
    ) -> Task<Msg> {
        let total = self.model.library.len();
        let end = start.saturating_add(AVAILABILITY_BATCH).min(total);
        let paths: Vec<(String, Option<usize>)> = self.model.library[start..end]
            .iter()
            .map(|book| {
                (
                    book.path.clone(),
                    self.model.bookmark_counts.get(&book.path).copied(),
                )
            })
            .collect();
        let backend = std::sync::Arc::clone(&self.metadata);
        let resolver = self.xdg_resolver.clone();
        let data_resolver = self.app_data_resolver.clone();
        let annotations_root = self.annotations_root.clone();
        // Both cancellation signals gate each iteration, so shutdown stops the scan
        // between entries.
        let task = self.submit_blocking_task(move |context| {
            let mut results = Vec::with_capacity(paths.len());
            for (path, previous_bookmark_count) in paths {
                if token.is_terminal() || context.is_cancelled() {
                    break;
                }
                let exists = backend.exists(&path);
                let bookmark_count = persistence::annotation_bookmark_count_with_data(
                    &path,
                    &resolver,
                    &data_resolver,
                    annotations_root.as_deref(),
                );
                results.push((path, exists, previous_bookmark_count, bookmark_count));
            }
            results
        });
        Task::future(async move {
            let outcome = task.await.into_result().ok();
            Msg::AvailabilityBatchDone {
                generation,
                next_index: end,
                total,
                outcome,
            }
        })
    }

    // Reject superseded batches and restart on library-order drift; worker failure leaves a
    // retryable scan.
    pub(crate) fn apply_availability_batch(
        &mut self,
        generation: u64,
        next_index: usize,
        total: usize,
        outcome: Option<Vec<crate::message::AvailabilityResult>>,
    ) -> Task<Msg> {
        if generation != self.availability.generation {
            self.availability.active = false;
            return self.begin_pending_or_idle();
        }
        let Some(results) = outcome else {
            // Worker failure: retire, keep cached status, and drop the coalesced request so
            // a failing spawn does not spin.
            self.availability.active = false;
            self.availability.pending = false;
            return Task::none();
        };
        if self.model.order_revision != self.availability.revision {
            // The order changed between batches; restart so every current key
            // is covered and none is skipped or double-counted.
            return self.begin_availability_chain();
        }
        let mut availability_updates = std::collections::HashMap::new();
        let mut bookmark_updates = std::collections::HashMap::new();
        for (path, exists, previous_bookmark_count, bookmark_count) in results {
            if self.model.app_state.files.contains_key(&path) {
                if self.model.availability.insert(path.clone(), exists) != Some(exists) {
                    availability_updates.insert(path.clone(), exists);
                }
                if let Some(bookmark_count) = bookmark_count
                    && self.model.bookmark_counts.get(&path).copied() == previous_bookmark_count
                    && self
                        .model
                        .bookmark_counts
                        .insert(path.clone(), bookmark_count)
                        != Some(bookmark_count)
                {
                    bookmark_updates.insert(path, bookmark_count);
                }
            }
        }
        if !availability_updates.is_empty() {
            self.model.apply_availability_flags(&availability_updates);
        }
        if !bookmark_updates.is_empty() {
            self.model.apply_bookmark_counts(&bookmark_updates);
        }
        if next_index < total {
            let token = match self.availability.cancel.as_ref() {
                Some(token) if !token.is_terminal() => token.share(),
                _ => {
                    self.availability.active = false;
                    return self.begin_pending_or_idle();
                }
            };
            return self.dispatch_availability_batch(generation, next_index, token);
        }
        self.availability.active = false;
        self.begin_pending_or_idle()
    }

    pub(crate) fn begin_pending_or_idle(&mut self) -> Task<Msg> {
        if self.availability.pending {
            self.begin_availability_chain()
        } else {
            Task::none()
        }
    }

    pub(crate) fn ensure_projection_fresh(&mut self) {
        if self.projection_dirty {
            self.rebuild_projection_now();
        }
    }

    pub(crate) fn rebuild_projection_now(&mut self) {
        refresh_projection(&mut self.model);
        self.projection_dirty = false;
    }

    // Snapshot everything dirty and dispatch serialisation and atomic writes off-thread;
    // derived views are only marked stale. Snapshots are O(1) `Arc` clones, and
    // [`persistence::PersistenceQueue`] orders writes so the last scheduled snapshot wins.
    pub(crate) fn flush_persistence(&mut self) -> Task<Msg> {
        let pending = self.prepare_persistence();
        Task::batch(
            pending
                .into_iter()
                .map(|pending| self.persistence_task(pending)),
        )
    }

    // The latest destination snapshot supersedes previous failed debt because it is the state
    // that must reach disk.
    pub(crate) fn prepare_persistence(&mut self) -> Vec<PendingPersistence> {
        let mut prepared = Vec::new();
        if self.progress_dirty
            && !self.model.library_loading
            && !self.model.library_persistence_unavailable
        {
            self.progress_dirty = false;
            let blocked = self
                .model
                .recovery_authorities
                .iter()
                .any(|authority| authority.origin == persistence::StateOrigin::Library);
            if self.persist_enabled && !blocked {
                let snapshot = PersistenceSnapshot::Library {
                    state: std::sync::Arc::clone(&self.model.app_state),
                    store: std::sync::Arc::clone(&self.library_store),
                };
                let ticket = self
                    .persistence_queue
                    .register(persistence::LIBRARY_STATE_DESTINATION);
                let pending = PendingPersistence {
                    ticket,
                    snapshot,
                    status: DebtStatus::InFlight,
                };
                self.persistence_debt
                    .insert(pending.ticket.destination().to_string(), pending.clone());
                prepared.push(pending);
            }
            // Mark the derived views stale rather than rebuilding here: a
            // routine progress flush must not sort/clone the whole store. The
            // rebuild is deferred to when the library is next shown.
            self.projection_dirty = true;
        }
        if self.annotations_dirty
            && !self.model.annotations_loading
            && !self.model.annotations_unavailable
        {
            self.annotations_dirty = false;
            let current_source = self.model.file_path.clone();
            let blocked = current_source.as_deref().is_some_and(|current| {
                let current = persistence::canonical_key(&current.to_string_lossy());
                self.model.recovery_authorities.iter().any(|authority| {
                    matches!(
                        &authority.origin,
                        persistence::StateOrigin::Annotations { source_path }
                            if persistence::canonical_key(source_path) == current
                    )
                })
            });
            if self.persist_enabled
                && !blocked
                && let Some(path) = current_source.as_deref()
            {
                let Some(custody) = self
                    .annotation_store
                    .as_ref()
                    .filter(|custody| custody.holds(path))
                    .cloned()
                else {
                    return prepared;
                };
                let ticket = self.persistence_queue.register(custody.queue_destination());
                let snapshot = PersistenceSnapshot::Annotations {
                    source_path: path.to_string_lossy().into_owned(),
                    file: std::sync::Arc::clone(&self.model.annotations),
                    store: custody.store,
                };
                let pending = PendingPersistence {
                    ticket,
                    snapshot,
                    status: DebtStatus::InFlight,
                };
                self.persistence_debt
                    .insert(pending.ticket.destination().to_string(), pending.clone());
                prepared.push(pending);
            }
        }
        // A replaced failure is no longer announced.
        self.recompute_persistence_alert();
        prepared
    }
}
