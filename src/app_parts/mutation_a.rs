//! Rotation, selection, and mutation admission.

use super::*;

use iced::Task;

impl App {
    // Per-page rotation is independent of global view rotation; history records absolute
    // angles.
    pub(crate) fn rotate_page(&mut self, id: u64, clockwise: bool) {
        let Some(index) = self.model.arrangement.position(id) else {
            return;
        };
        let Some(entry) = self.model.arrangement.get(index) else {
            return;
        };
        let label = entry.label.clone();
        let previous = entry.rotation;
        let Some(new) = self.model.arrangement.rotate(id, clockwise) else {
            return;
        };
        self.model
            .undo_stack
            .record(UndoableAction::RotatePage { id, previous, new });
        self.model.announce(format!("{label}: {}", new.status()));
    }

    // Empty glyph ranges must not enable annotation actions with degenerate geometry.
    pub(crate) fn set_pdf_selection(&mut self, page: usize, range: std::ops::Range<usize>) {
        use plumbing::pdf_ops::PdfRect;
        let Some(page_text) = self.model.pdf_text.get(page) else {
            self.model.pdf_selection = None;
            return;
        };
        let glyphs = page_text.selectable_glyphs();
        let range = range.start.min(glyphs.glyphs().len())..range.end.min(glyphs.glyphs().len());
        let text = glyphs.text(range.clone());
        if range.is_empty() || text.trim().is_empty() {
            self.model.pdf_selection = None;
            return;
        }
        // The glyph boxes are top-left points; PDF user space is bottom-left.
        let height = page_text.height_pts;
        let quads: Vec<PdfRect> = glyphs
            .range_rects(range.clone())
            .into_iter()
            .map(|[x, y, w, h]| PdfRect::new(x, height - (y + h), x + w, height - y))
            .collect();
        let Some(rect) = quads.iter().copied().reduce(|a, b| {
            PdfRect::new(
                a.x0.min(b.x0),
                a.y0.min(b.y0),
                a.x1.max(b.x1),
                a.y1.max(b.y1),
            )
        }) else {
            self.model.pdf_selection = None;
            return;
        };
        self.model.pdf_selection = Some(model::PdfSelection {
            page,
            range,
            rect,
            quads,
            text,
        });
    }

    pub(crate) fn open_pdf_comment_editor(&mut self) -> Task<Msg> {
        if self.model.format != Format::Pdf {
            self.model.refuse("PDF annotations need a PDF.");
            return Task::none();
        }
        let Some(selection) = self.model.pdf_selection.clone() else {
            self.model.refuse(crate::actions::SELECT_TEXT_FIRST);
            return Task::none();
        };
        self.model.pending_pdf_comment = Some(selection.clone());
        self.model.note_editor = Some(enzui_iced_widgets::form_dialog::FormPresenter::new(
            crate::view::note_editor::note_form_model(""),
        ));
        self.model.note_editor_editing = None;
        self.model.note_editor_page = selection.page;
        self.model.note_editor_position = None;
        crate::overlay_focus::open(&mut self.model, model::Overlay::NoteEditor)
    }

    // A terminal may arrive before writer exit, so admission also waits for the scope-exit
    // flag.
    pub(crate) fn pdf_mutation_writer_still_running(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        if let Some(flag) = self.pdf_mutation_writer_running.as_ref() {
            if flag.load(Ordering::Acquire) {
                return true;
            }
            self.pdf_mutation_writer_running = None;
        }
        false
    }

    // Reference the current writer's handoff slot at terminal time, or `None` when no
    // writer dispatch is in flight. The returned `Arc` is the reducer's
    // [`Self::pdf_mutation_writer_authority_slot`].
    pub(crate) fn claim_writer_authority(
        &mut self,
    ) -> Option<
        std::sync::Arc<std::sync::Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>>,
    > {
        self.pdf_mutation_writer_authority_slot.as_ref().cloned()
    }

    // Move a released writer's unclaimed authority onto the live
    // [`UncertainPublication::lost_authority`]. Runs before every uncertainty gate so a
    // late handoff from a cancelled task's closure lands before the next admit or close.
    pub(crate) fn finalize_writer_authority_handoff(&mut self) {
        if self.pdf_mutation_writer_still_running() {
            return;
        }
        let Some(slot_arc) = self.pdf_mutation_writer_authority_slot.as_ref().cloned() else {
            return;
        };
        self.pdf_mutation_writer_authority_slot = None;
        // Only peeks `is_some()`; the writer has released occupancy, so nothing contends.
        let has_authority = slot_arc
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        if !has_authority {
            return;
        }
        if let Some(record) = self.pdf_mutation_uncertain.as_mut()
            && record.lost_authority.is_none()
        {
            record.lost_authority = Some(slot_arc);
        }
        // With no uncertain record, the authority drops with its last `Arc` and its
        // descriptors close.
    }

    // True while the last dispatched Retry blocking closure has not
    // signalled scope exit via its `Drop`-guard.
    pub(crate) fn pdf_mutation_retry_still_running(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        if let Some(flag) = self.pdf_mutation_retry_running.as_ref() {
            if flag.load(Ordering::Acquire) {
                return true;
            }
            self.pdf_mutation_retry_running = None;
        }
        false
    }

    // Drop the dispatch-side `Arc` to the Retry authority slot once the closure has
    // released occupancy. The record keeps its own reference; with no record the
    // retained descriptors close.
    pub(crate) fn finalize_retry_authority_handoff(&mut self) {
        if self.pdf_mutation_retry_still_running() {
            return;
        }
        self.pdf_mutation_retry_authority_slot = None;
    }

    // Only Admitted permits clearing the caller draft or selection; Queued must preserve it.
    pub(crate) fn admit_pdf_mutation(
        &mut self,
        intent: PdfMutationIntent,
    ) -> (Task<Msg>, AdmissionResult) {
        // Claim a late authority handoff before the uncertainty gate judges this admit.
        self.finalize_writer_authority_handoff();
        self.finalize_retry_authority_handoff();
        // The reconciliation record blocks every native PDF mutation whatever the banner
        // state, so a mutation cannot race the pending recheck of the exact file.
        if self.pdf_mutation_uncertain.is_some() {
            self.model.refuse(RECONCILIATION_BLOCKED_ADMIT_SUMMARY);
            return (Task::none(), AdmissionResult::Refused);
        }
        // On `Cancelled` / `Shutdown` / `Panicked` the previous closure may still be
        // writing; refuse until its `Drop`-guard clears the running flag. A running Retry
        // holds the retained authority and blocks the same way.
        if self.pdf_mutation_writer_still_running() {
            self.model.refuse(WRITER_OCCUPANCY_BLOCKED_ADMIT_SUMMARY);
            return (Task::none(), AdmissionResult::Refused);
        }
        if self.pdf_mutation_retry_still_running() {
            self.model.refuse(RETRY_OCCUPANCY_BLOCKED_ADMIT_SUMMARY);
            return (Task::none(), AdmissionResult::Refused);
        }
        if self.pdf_mutation_active.is_some() {
            // History intents never queue; a live active plus a queued
            // history admission would corrupt the undo stack ordering.
            if intent.refuses_coalescing() {
                return (Task::none(), AdmissionResult::Refused);
            }
            // One busy-retry plus one pending slot; refuse rather than overwrite the
            // queued operation.
            if self.pdf_mutation_pending.is_some() {
                return (Task::none(), AdmissionResult::Refused);
            }
            let document_generation = self.model.pdf_annotation_generation.max(1);
            self.pdf_mutation_pending = Some(PdfMutationPending {
                document_generation,
                intent,
            });
            return (Task::none(), AdmissionResult::Queued);
        }
        let Some(op_token) = self.mint_pdf_op_token() else {
            return (Task::none(), AdmissionResult::Refused);
        };
        if self.model.pdf_annotation_generation == 0 {
            self.model.pdf_annotation_generation = 1;
        }
        let document_generation = self.model.pdf_annotation_generation;
        (
            self.dispatch_pdf_mutation(intent, op_token, document_generation),
            AdmissionResult::Admitted,
        )
    }

    // Advance [`Self::pdf_mutation_sequence`] and return the fresh nonzero op token, or
    // `None` on exhaustion, which forces the caller to refuse before any worker work.
    pub(crate) fn mint_pdf_op_token(&mut self) -> Option<u64> {
        let next = self.pdf_mutation_sequence.wrapping_add(1);
        if next == 0 {
            return None;
        }
        self.pdf_mutation_sequence = next;
        Some(next)
    }

    pub(crate) fn dispatch_pdf_mutation(
        &mut self,
        intent: PdfMutationIntent,
        op_token: u64,
        document_generation: u64,
    ) -> Task<Msg> {
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, Mutex};

        let request = intent.to_request();
        self.pdf_mutation_active = Some(PdfMutationInFlight {
            op_token,
            document_generation,
            intent: intent.clone(),
        });
        // The guard is built before dispatch, so a refused submission drops it and clears
        // the flag. Inside the closure it is the first local, so it drops last: any
        // `hand_off(slot, authority)` is observable before `running` reads `false`.
        let flag = Arc::new(AtomicBool::new(true));
        let authority_slot: Arc<Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>> =
            Arc::new(Mutex::new(None));
        self.pdf_mutation_writer_running = Some(Arc::clone(&flag));
        self.pdf_mutation_writer_authority_slot = Some(Arc::clone(&authority_slot));
        let occupancy = BlockingOccupancyGuard(flag);
        let dispatch_intent = intent.clone();
        let terminal_intent = intent;
        // Test builds carry the thread-local PDF save faults to the worker and back.
        #[cfg(test)]
        let (carried, carried_back) = {
            let carried = Arc::new(Mutex::new(Some(plumbing::pdf_ops::save_fault::carry())));
            (Arc::clone(&carried), carried)
        };
        self.dispatch_blocking(move || {
            let _guard = occupancy;
            #[cfg(test)]
            if let Some(state) = carried.lock().expect("fault carry").take() {
                state.install();
            }
            let outcome = plumbing::pdf_ops::apply_annotation_mutation(request, authority_slot);
            #[cfg(test)]
            {
                *carried.lock().expect("fault carry") =
                    Some(plumbing::pdf_ops::save_fault::carry());
            }
            outcome
        })
        .map(move |result| {
            #[cfg(test)]
            if let Some(state) = carried_back.lock().expect("fault carry").take() {
                state.install();
            }
            result
        })
        .map(move |result| match result {
            Ok(outcome) => Msg::PdfMutationCompleted {
                op_token,
                document_generation,
                intent: Box::new(terminal_intent.clone()),
                outcome: Ok(outcome),
            },
            Err(BlockingTaskError::Busy) => Msg::PdfMutationRefusedBusy {
                op_token,
                document_generation,
                intent: Box::new(dispatch_intent.clone()),
            },
            Err(terminal) => Msg::PdfMutationCompleted {
                op_token,
                document_generation,
                intent: Box::new(terminal_intent.clone()),
                outcome: Err(terminal),
            },
        })
    }

    // Accept sidecar/history changes only when operation token, document generation, path and
    // stable intent identity agree.
    pub(crate) fn finish_pdf_mutation(
        &mut self,
        op_token: u64,
        document_generation: u64,
        intent: PdfMutationIntent,
        outcome: Result<plumbing::pdf_ops::PdfMutationOutcome, BlockingTaskError>,
    ) -> Task<Msg> {
        // A stale terminal usually arrives with a stale pending, so re-validate here.
        self.discard_stale_pending_slots();
        let matches_current = self.pdf_mutation_active.as_ref().is_some_and(|active| {
            active.op_token == op_token
                && active.document_generation == document_generation
                && active.intent.same_stable_identity(&intent)
        });
        if !matches_current {
            return Task::none();
        }
        self.pdf_mutation_active = None;
        let path_matches = self.model.file_path.as_deref() == Some(intent.path())
            && self.model.format == Format::Pdf
            && document_generation == self.model.pdf_annotation_generation;
        if !path_matches {
            return self.release_retained_pdf_mutation();
        }
        let apply_task = match outcome {
            Ok(outcome) => self.apply_pdf_mutation_outcome(intent, op_token, outcome),
            Err(terminal) => {
                // The closure crossed the blocking boundary, so whether it published is
                // unknowable; record uncertainty debt for the exact op, path and
                // generation instead of dropping the mutation.
                self.record_pdf_mutation_terminal_uncertainty(op_token, &intent, terminal);
                Task::none()
            }
        };
        let follow = self.release_retained_pdf_mutation();
        Task::batch([apply_task, follow])
    }

    // Convert a post-dispatch `BlockingTaskError` into typed
    // [`UncertainPublication`] debt, block pending / new writes and
    // close on it. `Busy` is pre-wire only and never reaches this
    // path.
    pub(crate) fn record_pdf_mutation_terminal_uncertainty(
        &mut self,
        op_token: u64,
        intent: &PdfMutationIntent,
        terminal: BlockingTaskError,
    ) {
        let page = intent.page();
        let native_id = intent.native_id();
        // No reopen or post-fingerprint is known, so `sidecar_projected` is false and the
        // record cannot classify as Applied until a Retry or later write fingerprints the
        // file.
        self.install_pdf_mutation_uncertain(UncertainPublication {
            op_token,
            page,
            native_id,
            expected_pre_document_fingerprint: intent.expected_pre_document_fingerprint(),
            expected_post_document_fingerprint: None,
            intended: intent.intended_reconciliation(None),
            banner_visible: true,
            summary: pdf_worker_terminal_message(terminal),
            retry_sequence: self.pdf_mutation_uncertain_retry_sequence,
            retry_active: false,
            lost_authority: None,
        });
        // Claim the handoff now in case the closure already exited.
        self.finalize_writer_authority_handoff();
    }

    // Drain the older busy-retry slot first, then the newer pending slot. A slot whose
    // `document_generation` or `intent.path()` no longer matches the open PDF is
    // discarded. Blocked while uncertainty is unresolved.
    pub(crate) fn release_retained_pdf_mutation(&mut self) -> Task<Msg> {
        // Same handoff claim as `admit_pdf_mutation`, before the occupancy gates.
        self.finalize_writer_authority_handoff();
        self.finalize_retry_authority_handoff();
        if self.pdf_mutation_active.is_some()
            || self.pdf_mutation_uncertain.is_some()
            || self.pdf_mutation_writer_still_running()
            || self.pdf_mutation_retry_still_running()
        {
            return Task::none();
        }
        self.discard_stale_pending_slots();
        let next = match self.pdf_mutation_busy_retry.take() {
            Some(next) => next,
            None => match self.pdf_mutation_pending.take() {
                Some(next) => next,
                None => return Task::none(),
            },
        };
        let Some(op_token) = self.mint_pdf_op_token() else {
            // Sequence exhausted: restore the intent to its slot to preserve order.
            self.stash_pending_slot(next, /*prefer_busy_retry=*/ true);
            return Task::none();
        };
        if self.model.pdf_annotation_generation == 0 {
            self.model.pdf_annotation_generation = 1;
        }
        let document_generation = self.model.pdf_annotation_generation;
        self.dispatch_pdf_mutation(next.intent, op_token, document_generation)
    }

    // Discard busy-retry and pending slots whose `document_generation` or
    // `intent.path()` no longer matches the open PDF.
    pub(crate) fn discard_stale_pending_slots(&mut self) {
        let current_gen = self.model.pdf_annotation_generation;
        let current_path = self.model.file_path.clone();
        let current_format = self.model.format;
        let still_current = |pending: &PdfMutationPending| -> bool {
            current_format == Format::Pdf
                && pending.document_generation == current_gen
                && current_path.as_deref() == Some(pending.intent.path())
        };
        if let Some(pending) = self.pdf_mutation_busy_retry.as_ref()
            && !still_current(pending)
        {
            self.pdf_mutation_busy_retry = None;
        }
        if let Some(pending) = self.pdf_mutation_pending.as_ref()
            && !still_current(pending)
        {
            self.pdf_mutation_pending = None;
        }
    }

    pub(crate) fn stash_pending_slot(&mut self, next: PdfMutationPending, prefer_busy_retry: bool) {
        if prefer_busy_retry && self.pdf_mutation_busy_retry.is_none() {
            self.pdf_mutation_busy_retry = Some(next);
        } else if self.pdf_mutation_pending.is_none() {
            self.pdf_mutation_pending = Some(next);
        } else {
            self.pdf_mutation_busy_retry = Some(next);
        }
    }

    // Native history replays at its matching terminal; sidecar-only history replays
    // immediately.
    pub(crate) fn replay_history(&mut self, forward: bool) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        let empty = if forward {
            self.model.undo_stack.redo_empty()
        } else {
            self.model.undo_stack.undo_empty()
        };
        if empty {
            self.model.announce(if forward {
                undo::NOTHING_TO_REDO
            } else {
                undo::NOTHING_TO_UNDO
            });
            return Task::none();
        }
        let action = if forward {
            self.model.undo_stack.peek_redo_action().cloned()
        } else {
            self.model.undo_stack.peek_undo_action().cloned()
        };
        let (native_task, decision) = action
            .as_ref()
            .map(|action| self.apply_native_history(action, forward))
            .unwrap_or((Task::none(), HistoryDecision::NonNative));
        match decision {
            HistoryDecision::NativeAdmitted | HistoryDecision::Refused => {
                // The worker terminal owns the stack, or admission was refused.
                return native_task;
            }
            HistoryDecision::NonNative => {}
        }
        let label = if forward {
            self.model.undo_stack.redo(undo::UndoTarget::new(
                std::sync::Arc::make_mut(&mut self.model.annotations),
                &mut self.model.arrangement,
            ))
        } else {
            self.model.undo_stack.undo(undo::UndoTarget::new(
                std::sync::Arc::make_mut(&mut self.model.annotations),
                &mut self.model.arrangement,
            ))
        };
        if let Some(label) = label {
            self.refresh_all_epub_annotation_projections();
            self.save_annotations();
            self.model.announce(format!(
                "{} {label}",
                if forward { "Redid" } else { "Undid" }
            ));
        }
        Task::none()
    }

    // Install one uncertain-publication debt slot and mirror its summary onto
    // [`Model::pdf_mutation_uncertain_summary`] for the banner. All assignments of
    // `self.pdf_mutation_uncertain` go through here.
    pub(crate) fn install_pdf_mutation_uncertain(&mut self, debt: UncertainPublication) {
        // Dismiss hides the banner through `banner_visible` while the record stays.
        self.model.pdf_mutation_uncertain_summary =
            debt.banner_visible.then(|| debt.summary.to_string());
        self.pdf_mutation_uncertain = Some(debt);
    }

    pub(crate) fn clear_pdf_mutation_uncertain(&mut self) -> Option<UncertainPublication> {
        self.model.pdf_mutation_uncertain_summary = None;
        self.pdf_mutation_uncertain.take()
    }

    // Retry performs bounded I/O outside the custody lock and restores authority before
    // releasing its running flag.
    pub(crate) fn retry_pdf_mutation_uncertain(&mut self) -> Task<Msg> {
        // Claim a late handoff before the uncertainty gate judges this Retry.
        self.finalize_writer_authority_handoff();
        self.finalize_retry_authority_handoff();
        // Retry is serialized against the writer and against itself.
        if self.pdf_mutation_writer_still_running() || self.pdf_mutation_retry_still_running() {
            return Task::none();
        }
        let Some(debt_view) = self
            .pdf_mutation_uncertain
            .as_ref()
            .filter(|d| !d.retry_active)
        else {
            return Task::none();
        };
        let Some(authority_arc) = debt_view.lost_authority.as_ref().cloned() else {
            // Without retained authority Retry cannot read authoritatively; only Dismiss
            // resolves the record.
            self.model.refuse(UNCERTAIN_RETRY_FAILED_SUMMARY);
            return Task::none();
        };
        // `checked_add`: a wrapped zero would collide with a stale retry acknowledgement.
        let Some(next_retry) = self.pdf_mutation_uncertain_retry_sequence.checked_add(1) else {
            self.model.refuse(UNCERTAIN_RETRY_EXHAUSTED_SUMMARY);
            return Task::none();
        };
        self.pdf_mutation_uncertain_retry_sequence = next_retry;
        let (
            op_token,
            expected_pre_document_fingerprint,
            expected_post_document_fingerprint,
            intended,
            page,
            native_id,
        ) = {
            let debt = self
                .pdf_mutation_uncertain
                .as_mut()
                .expect("debt present per guard above");
            debt.retry_sequence = next_retry;
            debt.retry_active = true;
            (
                debt.op_token,
                debt.expected_pre_document_fingerprint.clone(),
                debt.expected_post_document_fingerprint.clone(),
                debt.intended.clone(),
                debt.page,
                debt.native_id.clone(),
            )
        };
        let retry_sequence = next_retry;

        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let flag = Arc::new(AtomicBool::new(true));
        self.pdf_mutation_retry_running = Some(Arc::clone(&flag));
        self.pdf_mutation_retry_authority_slot = Some(Arc::clone(&authority_arc));
        // Built before dispatch, so a refused submission drops it and clears the flag
        // without the authority leaving the slot.
        let occupancy = BlockingOccupancyGuard(flag);

        self.dispatch_blocking(move || {
            use enzui_iced_core::user_file::DocumentAuthority;
            struct AuthorityRestoreGuard {
                slot: Arc<std::sync::Mutex<Option<DocumentAuthority>>>,
                holding: Option<DocumentAuthority>,
            }
            impl Drop for AuthorityRestoreGuard {
                fn drop(&mut self) {
                    if let Some(auth) = self.holding.take()
                        && let Ok(mut guard) = self.slot.lock()
                        && guard.is_none()
                    {
                        *guard = Some(auth);
                    }
                }
            }
            // First local, so it drops last: the `AuthorityRestoreGuard` restores the
            // authority before the flag reads `false`.
            let _guard = occupancy;
            let taken = match authority_arc.lock() {
                Ok(mut guard) => guard.take(),
                Err(_) => None,
            };
            let Some(authority) = taken else {
                return PdfReconciliationClass::StillUncertain;
            };
            let restore = AuthorityRestoreGuard {
                slot: Arc::clone(&authority_arc),
                holding: Some(authority),
            };
            let class = classify_reconciliation_via_authority(
                restore.holding.as_ref().expect("authority present"),
                &expected_pre_document_fingerprint,
                expected_post_document_fingerprint.as_deref(),
                &intended,
                page,
                &native_id,
            );
            drop(restore);
            class
        })
        .map(move |result| Msg::PdfMutationUncertainRetried {
            op_token,
            retry_sequence,
            outcome: result,
        })
    }
}
