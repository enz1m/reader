//! Uncertain retry and outcome application.

use super::*;

use iced::Task;

impl App {
    // Terminal handler for the Retry blocking dispatch. The retained authority returns to
    // the debt whatever the classification; only `Applied` / `NotApplied` on the exact
    // `(op_token, retry_sequence)` clear the debt.
    pub(crate) fn finish_pdf_mutation_uncertain_retry(
        &mut self,
        op_token: u64,
        retry_sequence: u64,
        outcome: Result<PdfReconciliationClass, BlockingTaskError>,
    ) -> Task<Msg> {
        // If the closure is still running its `Drop`-restore, this is a no-op and a later
        // turn drops the reducer-side `Arc`.
        self.finalize_retry_authority_handoff();

        let matches_current = self
            .pdf_mutation_uncertain
            .as_ref()
            .is_some_and(|d| d.op_token == op_token && d.retry_sequence == retry_sequence);
        if !matches_current {
            // Stale: the closure's restore guard already returned the shared authority.
            return Task::none();
        }
        if let Some(debt) = self.pdf_mutation_uncertain.as_mut() {
            debt.retry_active = false;
        }
        let debt = self
            .pdf_mutation_uncertain
            .as_ref()
            .cloned()
            .expect("debt present per matches_current");
        match outcome {
            Ok(PdfReconciliationClass::Applied) => {
                let reread = self.apply_reconciled_classification(&debt, false);
                self.clear_pdf_mutation_uncertain();
                self.model.announce(UNCERTAIN_RETRY_CLEARED_SUMMARY);
                return reread;
            }
            Ok(PdfReconciliationClass::NotApplied) => {
                let reread = self.apply_reconciled_classification(&debt, true);
                self.clear_pdf_mutation_uncertain();
                self.model.announce(UNCERTAIN_RETRY_NOT_APPLIED_SUMMARY);
                return reread;
            }
            Ok(PdfReconciliationClass::StillUncertain) => {
                self.model.refuse(UNCERTAIN_RETRY_MISMATCH_SUMMARY);
            }
            Err(_) => {
                self.model.refuse(UNCERTAIN_RETRY_FAILED_SUMMARY);
            }
        }
        Task::none()
    }

    // A create projected before failed publication needs sidecar rollback; reread native
    // annotations after reconciliation.
    pub(crate) fn apply_reconciled_classification(
        &mut self,
        debt: &UncertainPublication,
        not_applied: bool,
    ) -> Task<Msg> {
        if not_applied
            && matches!(
                debt.intended,
                support::PdfReconciliationIntent::Present {
                    expected_native_fingerprint: None,
                }
            )
        {
            let annotations = std::sync::Arc::make_mut(&mut self.model.annotations);
            let before = annotations.annotations.len();
            annotations.annotations.retain(|annotation| {
                annotation
                    .pdf_binding
                    .as_ref()
                    .is_none_or(|binding| binding.native_id != debt.native_id)
            });
            if annotations.annotations.len() != before {
                self.rebuild_pdf_annotation_projection();
                self.save_annotations();
            }
        }
        self.invalidate_pdf_annotation_page(debt.page);
        self.request_native_pdf_annotations()
    }

    pub(crate) fn apply_pdf_mutation_outcome(
        &mut self,
        intent: PdfMutationIntent,
        op_token: u64,
        outcome: plumbing::pdf_ops::PdfMutationOutcome,
    ) -> Task<Msg> {
        use plumbing::pdf_ops::{PdfMutationCustody, PdfMutationRefusal};
        match outcome.custody {
            PdfMutationCustody::Applied => {
                // Commit the sidecar entry before reconciling the post-write snapshot,
                // or the new native annotation imports as foreign and duplicates.
                self.commit_intent_sidecar(&intent, op_token, &outcome, false);
                if let Some(snapshot) = outcome.snapshot.clone() {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
                self.invalidate_pdf_annotation_page(outcome.page);
                Task::none()
            }
            PdfMutationCustody::Uncertain {
                adopted_via_reopen,
                lost_custody,
                ..
            } => {
                // As for Applied: commit an adopted write before reconciling
                // the snapshot that already contains it.
                if adopted_via_reopen {
                    self.commit_intent_sidecar(&intent, op_token, &outcome, true);
                    self.invalidate_pdf_annotation_page(outcome.page);
                }
                if let Some(snapshot) = outcome.snapshot.clone() {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
                let lost_authority = if lost_custody {
                    self.claim_writer_authority()
                } else {
                    None
                };
                self.install_pdf_mutation_uncertain(UncertainPublication {
                    op_token,
                    page: outcome.page,
                    native_id: outcome.native_id.clone(),
                    expected_pre_document_fingerprint: intent.expected_pre_document_fingerprint(),
                    expected_post_document_fingerprint: outcome
                        .refreshed
                        .as_ref()
                        .map(|r| r.document_fingerprint.clone()),
                    intended: intent.intended_reconciliation(outcome.refreshed.as_ref()),
                    banner_visible: true,
                    summary: UNCERTAIN_PUBLICATION_SUMMARY,
                    retry_sequence: 0,
                    retry_active: false,
                    lost_authority,
                });
                Task::none()
            }
            PdfMutationCustody::Refused { reason } => {
                if let Some(snapshot) = outcome.snapshot.clone() {
                    self.reconcile_native_pdf_annotations(snapshot);
                }
                match &reason {
                    PdfMutationRefusal::PageIdentityDrifted { .. }
                    | PdfMutationRefusal::DocumentIdentityDrifted => {
                        self.model.refuse(reason.user_summary());
                    }
                    other => {
                        self.model.refuse(other.user_summary());
                    }
                }
                Task::none()
            }
        }
    }

    pub(crate) fn commit_intent_sidecar(
        &mut self,
        intent: &PdfMutationIntent,
        op_token: u64,
        outcome: &plumbing::pdf_ops::PdfMutationOutcome,
        uncertain: bool,
    ) {
        match intent {
            PdfMutationIntent::Create(create) => {
                self.commit_created_annotation(create, outcome);
                let suffix = if uncertain {
                    " (publication uncertain)"
                } else {
                    ""
                };
                self.model.announce(match create.kind {
                    plumbing::pdf_ops::PdfAnnotationKind::Highlight => {
                        format!(
                            "Highlight written to page {}{suffix}",
                            create.selection.page + 1
                        )
                    }
                    plumbing::pdf_ops::PdfAnnotationKind::Comment => format!(
                        "Comment added on page {}{suffix}",
                        create.selection.page + 1
                    ),
                });
            }
            PdfMutationIntent::EditContents(edit) => {
                self.commit_edited_contents(edit, outcome);
            }
            PdfMutationIntent::UpdateColor(update) => {
                self.commit_color_update(update, outcome);
            }
            PdfMutationIntent::Remove(remove) => {
                self.commit_native_removal(remove);
            }
            PdfMutationIntent::HistoryReplay {
                action, forward, ..
            } => {
                self.commit_history_replay(intent, op_token, action, *forward, outcome);
            }
        }
    }

    // Replay sidecar history only after confirmed PDF mutation; stack drift records
    // uncertainty rather than replaying another action.
    pub(crate) fn commit_history_replay(
        &mut self,
        intent: &PdfMutationIntent,
        op_token: u64,
        action: &UndoableAction,
        forward: bool,
        outcome: &plumbing::pdf_ops::PdfMutationOutcome,
    ) {
        let stack_matches = if forward {
            self.model.undo_stack.peek_redo_action() == Some(action)
        } else {
            self.model.undo_stack.peek_undo_action() == Some(action)
        };
        if !stack_matches {
            // The stack drifted, so the sidecar cannot follow; `sidecar_projected = false`
            // lets a matching reopen re-project.
            self.install_pdf_mutation_uncertain(UncertainPublication {
                op_token,
                page: outcome.page,
                native_id: outcome.native_id.clone(),
                expected_pre_document_fingerprint: intent.expected_pre_document_fingerprint(),
                expected_post_document_fingerprint: outcome
                    .refreshed
                    .as_ref()
                    .map(|r| r.document_fingerprint.clone()),
                intended: intent.intended_reconciliation(outcome.refreshed.as_ref()),
                banner_visible: true,
                summary: HISTORY_STACK_DRIFT_SUMMARY,
                retry_sequence: 0,
                retry_active: false,
                lost_authority: None,
            });
            return;
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
    }

    pub(crate) fn commit_created_annotation(
        &mut self,
        create: &PdfCreateIntent,
        outcome: &plumbing::pdf_ops::PdfMutationOutcome,
    ) {
        use plumbing::pdf_ops::PdfAnnotationKind;
        let (annotation_kind, text, stored_color, style, note) = match create.kind {
            PdfAnnotationKind::Highlight => (
                "highlight",
                create.selection.text.as_str(),
                create.color.as_str(),
                "highlight",
                create.authored_contents.as_str(),
            ),
            PdfAnnotationKind::Comment => ("note", create.authored_contents.as_str(), "", "", ""),
        };
        let id = persistence::add_annotation_full(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            create.selection.page,
            annotation_kind,
            text,
            stored_color,
            create.position.clone(),
            style,
            note,
        );
        if let Some(refreshed) = outcome.refreshed.as_ref()
            && let Some(annotation) = std::sync::Arc::make_mut(&mut self.model.annotations)
                .annotations
                .iter_mut()
                .find(|annotation| annotation.id == id)
        {
            annotation.quads = create
                .selection
                .quads
                .iter()
                .map(|q| {
                    let hundredths = |v: f32| (v * 100.0).round() as i32;
                    [
                        hundredths(q.x0),
                        hundredths(q.y0),
                        hundredths(q.x1),
                        hundredths(q.y1),
                    ]
                })
                .collect();
            annotation.pdf_binding = Some(persistence::PdfAnnotationBinding {
                native_id: create.native_id.clone(),
                document_fingerprint: refreshed.document_fingerprint.clone(),
                page_fingerprint: refreshed.page_fingerprint.clone(),
                native_fingerprint: refreshed.native_fingerprint.clone(),
                text_fingerprint: if create.kind == PdfAnnotationKind::Highlight {
                    plumbing::pdf_ops::text_fingerprint(&create.selection.text)
                } else {
                    String::new()
                },
            });
        }
        self.model
            .pdf_annotation_states
            .insert(id, model::PdfAnnotationState::Verified);
        self.model.selected_annotation = Some(id);
        self.rebuild_pdf_annotation_projection();
        self.record_add_annotation(id);
        self.model.pdf_selection = None;
        self.save_annotations();
    }

    pub(crate) fn commit_edited_contents(
        &mut self,
        edit: &PdfEditContentsIntent,
        outcome: &plumbing::pdf_ops::PdfMutationOutcome,
    ) {
        let file = std::sync::Arc::make_mut(&mut self.model.annotations);
        let Some(annotation) = file
            .annotations
            .iter_mut()
            .find(|annotation| annotation.id == edit.sidecar_id)
            .cloned()
        else {
            return;
        };
        let previous = if annotation.kind == "highlight" {
            annotation.note.clone()
        } else {
            annotation.text.clone()
        };
        if annotation.kind == "highlight" {
            persistence::update_annotation_note(file, edit.sidecar_id, &edit.new_contents);
            self.model
                .undo_stack
                .record(UndoableAction::UpdateAnnotationNote {
                    id: edit.sidecar_id,
                    previous,
                    new: edit.new_contents.clone(),
                });
        } else {
            persistence::update_annotation_text(file, edit.sidecar_id, &edit.new_contents);
            self.model
                .undo_stack
                .record(UndoableAction::UpdateAnnotationText {
                    id: edit.sidecar_id,
                    previous,
                    new: edit.new_contents.clone(),
                });
        }
        if let Some(refreshed) = outcome.refreshed.as_ref()
            && let Some(annotation) = std::sync::Arc::make_mut(&mut self.model.annotations)
                .annotations
                .iter_mut()
                .find(|annotation| annotation.id == edit.sidecar_id)
            && let Some(binding) = annotation.pdf_binding.as_mut()
        {
            binding.document_fingerprint = refreshed.document_fingerprint.clone();
            binding.page_fingerprint = refreshed.page_fingerprint.clone();
            binding.native_fingerprint = refreshed.native_fingerprint.clone();
        }
        self.rebuild_pdf_annotation_projection();
        self.save_annotations();
    }

    pub(crate) fn commit_color_update(
        &mut self,
        update: &PdfUpdateColorIntent,
        outcome: &plumbing::pdf_ops::PdfMutationOutcome,
    ) {
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == update.sidecar_id)
            .cloned()
        else {
            return;
        };
        persistence::update_annotation_color(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            update.sidecar_id,
            &update.new_color,
        );
        self.model
            .undo_stack
            .record(UndoableAction::UpdateAnnotationColor {
                id: update.sidecar_id,
                previous: annotation.color.clone(),
                new: update.new_color.clone(),
            });
        if let Some(refreshed) = outcome.refreshed.as_ref()
            && let Some(annotation) = std::sync::Arc::make_mut(&mut self.model.annotations)
                .annotations
                .iter_mut()
                .find(|annotation| annotation.id == update.sidecar_id)
            && let Some(binding) = annotation.pdf_binding.as_mut()
        {
            binding.document_fingerprint = refreshed.document_fingerprint.clone();
            binding.page_fingerprint = refreshed.page_fingerprint.clone();
            binding.native_fingerprint = refreshed.native_fingerprint.clone();
        }
        self.rebuild_pdf_annotation_projection();
        self.save_annotations();
        self.model
            .announce(format!("Highlight color changed to {}", update.new_color));
    }

    pub(crate) fn commit_native_removal(&mut self, remove: &PdfRemoveIntent) {
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == remove.sidecar_id)
            .cloned()
        else {
            return;
        };
        let epub_chapter =
            match persistence::AnnotationPosition::parse(&annotation.position, annotation.page) {
                persistence::AnnotationPosition::EpubCfi { chapter, .. }
                    if annotation.kind == "highlight" =>
                {
                    Some(chapter)
                }
                _ => None,
            };
        if let Some(snapshot) = self.model.pdf_annotation_snapshot.as_mut() {
            snapshot
                .annotations
                .retain(|native| native.native_id != remove.native_id);
        }
        if persistence::remove_annotation(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            remove.sidecar_id,
        ) {
            self.model
                .undo_stack
                .record(UndoableAction::RemoveAnnotation {
                    annotation: annotation.clone(),
                });
            self.model.pdf_annotation_states.remove(&remove.sidecar_id);
            if self.model.selected_annotation == Some(remove.sidecar_id) {
                self.model.selected_annotation = None;
            }
            self.rebuild_pdf_annotation_projection();
            if let Some(chapter) = epub_chapter {
                self.refresh_epub_annotation_projection(chapter);
            }
            self.save_annotations();
            self.model
                .announce("Annotation deleted — Undo is available");
        }
    }

    // Only Admitted permits clearing the caller draft or selection; Queued must preserve it.
    pub(crate) fn create_pdf_annotation(
        &mut self,
        kind: plumbing::pdf_ops::PdfAnnotationKind,
        selection: model::PdfSelection,
        authored_comment: &str,
        color: &str,
    ) -> (Task<Msg>, AdmissionResult) {
        let Some(path) = self.model.file_path.clone() else {
            return (Task::none(), AdmissionResult::Refused);
        };
        if self.model.format != Format::Pdf {
            self.model.refuse("PDF annotations need a PDF.");
            return (Task::none(), AdmissionResult::Refused);
        }
        let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref() else {
            self.model
                .refuse("Native PDF annotations are still being verified.");
            return (Task::none(), AdmissionResult::Refused);
        };
        let expected_document_fingerprint = snapshot.document_fingerprint.clone();
        let Some(page_identity) = snapshot.pages.get(selection.page).cloned() else {
            self.model
                .refuse("The selected PDF page is no longer available.");
            return (Task::none(), AdmissionResult::Refused);
        };
        let Some(position) = plumbing::pdf_annotations::normalized_position(
            selection.page,
            selection.rect,
            &page_identity,
        ) else {
            self.model
                .refuse("The selected PDF text has invalid geometry.");
            return (Task::none(), AdmissionResult::Refused);
        };
        let next_id = self.model.annotations.next_id.max(1);
        let native_id = format!("enzui-{}-{next_id}", expected_document_fingerprint);
        let intent = PdfMutationIntent::Create(PdfCreateIntent {
            path,
            expected_document_fingerprint,
            expected_page_fingerprint: page_identity.fingerprint,
            selection,
            position,
            kind,
            color: color.to_string(),
            authored_contents: authored_comment.to_string(),
            native_id,
        });
        self.admit_pdf_mutation(intent)
    }
}
