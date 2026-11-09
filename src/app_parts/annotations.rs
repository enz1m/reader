//! Annotation persistence and projection.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn save_position(&mut self) {
        let Some(path) = self.model.file_path.clone() else {
            return;
        };
        let key = persistence::canonical_key(&path.to_string_lossy());
        if let Some(rec) = self.model.app_state.files.get(&key).cloned() {
            let title = rec.title.clone();
            let author = rec.author.clone();
            let format = rec.format.clone();
            self.persist_progress(&path, title, author, &format);
        }
    }

    pub(crate) fn save_annotations(&mut self) {
        if self.model.file_path.is_none() {
            return;
        }
        // Marked, not written: `flush_persistence` owns the write, and every document swap
        // flushes first, so the sidecar always lands on its own path.
        self.annotation_state_revision = self.annotation_state_revision.saturating_add(1);
        self.annotations_dirty = true;
    }

    pub(crate) fn record_add_annotation(&mut self, id: u64) {
        if let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|a| a.id == id)
            .cloned()
        {
            self.model
                .undo_stack
                .record(UndoableAction::AddAnnotation { annotation });
        }
    }

    pub(crate) fn request_annotations_for_document(&mut self, path: &Path) -> Task<Msg> {
        self.annotation_recovery_generation = self.annotation_recovery_generation.wrapping_add(1);
        self.annotation_state_revision = 0;
        self.model.annotations_loading = true;
        self.model.annotations_unavailable = false;
        self.model.annotations =
            std::sync::Arc::new(persistence::empty_annotations(&path.to_string_lossy()));
        self.annotation_store = None;

        let generation = self.annotation_recovery_generation;
        let base_revision = self.annotation_state_revision;
        let source_path = path.to_string_lossy().into_owned();
        if self.load_controller.is_loading() {
            if self.active_load_origin != Some(persistence::StateOrigin::Library) {
                self.load_controller.invalidate();
            }
            self.pending_load = Some(PendingLoad::Annotations(path.to_path_buf()));
            return Task::none();
        }
        self.pending_load = None;
        let resolver = self.xdg_resolver.clone();
        let data_resolver = self.app_data_resolver.clone();
        let annotations_root = self.annotations_root.clone();
        let probe_observer = self.annotation_probe_observer.clone();
        let worker_source_path = source_path.clone();
        self.active_load_origin = Some(persistence::StateOrigin::Annotations {
            source_path: source_path.clone(),
        });
        // Annotation reload dispatched through the shared bounded owner.
        let task = self
            .load_controller
            .begin(&self.blocking_dispatcher, move || {
                persistence::reload_recovery_with_observer(
                    persistence::StateOrigin::Annotations {
                        source_path: worker_source_path,
                    },
                    &resolver,
                    &data_resolver,
                    annotations_root.as_deref(),
                    probe_observer.as_ref(),
                )
            });
        match task {
            Ok(task) => task.map(move |settlement| Msg::AnnotationsRestored {
                generation,
                base_revision,
                settlement,
            }),
            Err(error) => Task::done(Msg::AnnotationsRestored {
                generation,
                base_revision,
                settlement: persistence::BlockingLoadSettlement::WorkerFailed(error),
            }),
        }
    }

    pub(crate) fn apply_annotation_load_settlement(
        &mut self,
        generation: u64,
        base_revision: u64,
        settlement: persistence::BlockingLoadSettlement,
    ) -> Task<Msg> {
        if !self.load_controller.is_loading() {
            self.active_load_origin = None;
        }
        let task = match settlement {
            persistence::BlockingLoadSettlement::Applied(reload) => {
                self.apply_annotations_restore(generation, base_revision, reload)
            }
            persistence::BlockingLoadSettlement::DiscardedStale => Task::none(),
            persistence::BlockingLoadSettlement::WorkerFailed(error) => {
                eprintln!("reader-app annotation restore worker failed: {error}");
                if generation != self.annotation_recovery_generation {
                    Task::none()
                } else {
                    let source_path = self
                        .model
                        .file_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let reload = persistence::recovery_worker_failure_with_data(
                        persistence::StateOrigin::Annotations { source_path },
                        &self.xdg_resolver,
                        &self.app_data_resolver,
                        self.annotations_root.as_deref(),
                    );
                    self.apply_annotations_restore(generation, base_revision, reload)
                }
            }
        };
        self.resume_pending_load(task)
    }

    pub(crate) fn resume_pending_load(&mut self, task: Task<Msg>) -> Task<Msg> {
        if self.load_controller.is_loading() {
            return task;
        }
        let Some(pending) = self.pending_load.take() else {
            return task;
        };
        let next = match pending {
            PendingLoad::Annotations(path) => self.request_annotations_for_document(&path),
            PendingLoad::Recovery(origin) => self.update(Msg::RetryStateRecovery(origin)),
        };
        Task::batch([task, next])
    }

    pub(crate) fn apply_annotations_restore(
        &mut self,
        generation: u64,
        base_revision: u64,
        reload: persistence::RecoveryReload,
    ) -> Task<Msg> {
        if generation != self.annotation_recovery_generation {
            return Task::none();
        }
        let persistence::RecoveryReload::Annotations {
            source_path,
            source_key,
            destination,
            store,
            outcome,
        } = reload
        else {
            return Task::none();
        };
        let Some(current) = self
            .model
            .file_path
            .clone()
            .filter(|path| path == Path::new(&source_path))
        else {
            return Task::none();
        };
        if base_revision != self.annotation_state_revision {
            return Task::none();
        }
        self.model.annotations_loading = false;
        self.annotation_store = destination.map(|destination| AnnotationCustody {
            source_path: current,
            source_key: source_key.clone(),
            destination,
            store,
        });
        let unavailable = outcome.is_unavailable();
        let (annotations, recovery) = outcome.into_parts();
        self.model.annotations_unavailable = unavailable || recovery.is_some();
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
        let bookmark_count = annotations.bookmarks.len();
        self.model.annotations = std::sync::Arc::new(annotations);
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
        Task::none()
    }

    #[cfg(test)]
    pub(crate) fn load_annotations_for_document(&mut self, path: &Path) {
        self.annotation_recovery_generation = self.annotation_recovery_generation.wrapping_add(1);
        self.annotation_state_revision = 0;
        self.model.annotations_loading = true;
        self.model.annotations_unavailable = false;
        let source_path = path.to_string_lossy().into_owned();
        let opened = persistence::open_annotations_authority_with_observer(
            &source_path,
            &self.xdg_resolver,
            &self.app_data_resolver,
            self.annotations_root.as_deref(),
            self.annotation_probe_observer.as_ref(),
        );
        let generation = self.annotation_recovery_generation;
        // Drive the restore's follow-up work so the helper returns with the annotations
        // installed, as the runtime would.
        let restore = self.apply_annotations_restore(
            generation,
            self.annotation_state_revision,
            persistence::RecoveryReload::Annotations {
                source_key: persistence::canonical_key(&source_path),
                destination: opened
                    .authority
                    .as_ref()
                    .map(|authority| authority.destination.clone()),
                source_path,
                store: opened.store,
                outcome: opened.load,
            },
        );
        let mut pending: std::collections::VecDeque<Msg> = crate::app::drain_task(restore).into();
        let mut budget = 32usize;
        while let Some(message) = pending.pop_front() {
            budget = budget
                .checked_sub(1)
                .expect("the annotation restore chain exceeded its message budget");
            let task = self.update(message);
            pending.extend(crate::app::drain_task(task));
        }
    }

    pub(crate) fn request_pdf_navigation(&mut self) -> Task<Msg> {
        let Some(path) = self.model.file_path.clone() else {
            return Task::none();
        };
        if self.model.format != Format::Pdf {
            return Task::none();
        }
        self.model.pdf_navigation_generation = self.model.pdf_navigation_generation.wrapping_add(1);
        let generation = self.model.pdf_navigation_generation;
        self.model.pdf_navigation_loading = true;
        self.model.pdf_navigation_error = None;
        self.model.pdf_outline_error = None;
        let password = self.model.pdf_password.clone();
        // PDF outline/navigation read dispatched through the shared
        // bounded owner (latest-wins gated by generation).
        self.dispatch_blocking(move || plumbing::pdf_navigation::load(&path, password.as_deref()))
            .map(move |result| Msg::PdfNavigationLoaded {
                generation,
                outcome: result
                    .unwrap_or_else(|error| Err(format!("PDF navigation worker failed: {error}"))),
            })
    }

    pub(crate) fn request_native_pdf_annotations(&mut self) -> Task<Msg> {
        let Some(path) = self.model.file_path.clone() else {
            return Task::none();
        };
        if self.model.format != Format::Pdf {
            return Task::none();
        }
        self.model.pdf_annotation_generation = self.model.pdf_annotation_generation.wrapping_add(1);
        let generation = self.model.pdf_annotation_generation;
        // Native PDF annotation read dispatched through the shared
        // bounded owner (latest-wins gated by generation).
        self.dispatch_blocking(move || plumbing::pdf_ops::read_document_annotations(&path))
            .map(move |result| Msg::NativePdfAnnotationsLoaded {
                generation,
                outcome: result
                    .unwrap_or_else(|error| Err(format!("PDF annotation worker failed: {error}"))),
            })
    }

    pub(crate) fn reconcile_native_pdf_annotations(
        &mut self,
        snapshot: plumbing::pdf_ops::PdfAnnotationSnapshot,
    ) {
        let reconciliation = plumbing::pdf_annotations::reconcile(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            &snapshot,
            &self.model.pdf_text,
        );
        self.model.pdf_annotation_states = reconciliation.states;
        self.model.pdf_annotation_snapshot = Some(snapshot);
        self.rebuild_pdf_annotation_projection();
        if reconciliation.changed {
            self.save_annotations();
        }
    }

    pub(crate) fn invalidate_pdf_annotation_page(&mut self, page: usize) {
        self.model.raster_generation = self.model.raster_generation.wrapping_add(1);
        self.model.rasterising.clear();
        // Stale rather than removed, so the old raster stays until its replacement lands.
        self.model.pages.mark_stale(page);
        self.model.raster_failed.remove(&page);
        self.model.thumbnail_generation = self.model.thumbnail_generation.wrapping_add(1);
        self.model.thumbnail_rasterising.clear();
        self.model.thumbnail_rasters.mark_stale(page);
        self.model.thumbnail_failed.remove(&page);
    }

    pub(crate) fn rebuild_pdf_annotation_projection(&mut self) {
        let mut pages: HashMap<usize, Vec<model::PdfPageMark>> = HashMap::new();
        for annotation in &self.model.annotations.annotations {
            if !self
                .model
                .pdf_annotation_states
                .get(&annotation.id)
                .is_some_and(|state| state.is_projectable())
            {
                continue;
            }
            let persistence::AnnotationPosition::PdfRect { page, x, y, w, h } =
                persistence::AnnotationPosition::parse(&annotation.position, annotation.page)
            else {
                continue;
            };
            if !(w > 0.0 && h > 0.0) || self.model.page_extents.get(page).is_none() {
                continue;
            }
            let is_comment = annotation.kind == "note";
            let note = if is_comment {
                &annotation.text
            } else {
                &annotation.note
            };
            pages.entry(page).or_default().push(model::PdfPageMark {
                id: annotation.id,
                rect: [x, y, w, h],
                note: (!note.trim().is_empty()).then(|| note.clone()),
                tints_text: is_comment,
                color: plumbing::pdf_ops::highlight_rgb(&annotation.color),
            });
        }
        self.model.pdf_overlay_annotations = pages
            .into_iter()
            .map(|(page, annotations)| (page, std::sync::Arc::new(annotations)))
            .collect();
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn persistence_pending(&self) -> bool {
        self.progress_dirty || self.annotations_dirty
    }
}
