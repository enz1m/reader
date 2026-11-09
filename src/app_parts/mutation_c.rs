//! Annotation commits and native history.

use super::*;

use iced::Task;

impl App {
    // Only admitted mutations or NoChange may close the composer; queued and refused edits
    // preserve its draft.
    pub(crate) fn edit_pdf_annotation_comment(
        &mut self,
        id: u64,
        text: &str,
    ) -> EditCommentOutcome {
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == id)
            .cloned()
        else {
            return EditCommentOutcome::NoChange;
        };
        let Some(binding) = annotation.pdf_binding.clone() else {
            return EditCommentOutcome::NonNative;
        };
        let previous = if annotation.kind == "highlight" {
            annotation.note.clone()
        } else {
            annotation.text.clone()
        };
        if previous == text {
            return EditCommentOutcome::NoChange;
        }
        let Some(path) = self.model.file_path.clone() else {
            return EditCommentOutcome::Failed;
        };
        if !self.native_binding_is_current(&annotation) {
            self.model
                .refuse("This PDF annotation is not safe to edit.");
            return EditCommentOutcome::Failed;
        }
        let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref() else {
            self.model
                .refuse("Native PDF annotations are still being verified.");
            return EditCommentOutcome::Failed;
        };
        let expected_document_fingerprint = snapshot.document_fingerprint.clone();
        let (expected_rect, expected_subtype) =
            native_annotation_rect_and_subtype(snapshot, &binding.native_id, annotation.page);
        let intent = PdfMutationIntent::EditContents(PdfEditContentsIntent {
            path,
            expected_document_fingerprint,
            expected_page_fingerprint: binding.page_fingerprint.clone(),
            expected_native_fingerprint: binding.native_fingerprint.clone(),
            page: annotation.page,
            native_id: binding.native_id.clone(),
            color: annotation.color.clone(),
            sidecar_id: id,
            new_contents: text.to_string(),
            expected_rect,
            expected_subtype,
        });
        let (task, result) = self.admit_pdf_mutation(intent);
        EditCommentOutcome::Native { task, result }
    }

    pub(crate) fn native_binding_is_current(&self, annotation: &persistence::Annotation) -> bool {
        let Some(binding) = annotation.pdf_binding.as_ref() else {
            return false;
        };
        let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref() else {
            return false;
        };
        if self
            .model
            .pdf_annotation_states
            .get(&annotation.id)
            .is_some_and(|state| {
                matches!(
                    state,
                    model::PdfAnnotationState::Orphaned | model::PdfAnnotationState::Conflict
                )
            })
        {
            return false;
        }
        let Some(sidecar_rect) =
            plumbing::pdf_annotations::absolute_rect(annotation, &snapshot.pages)
        else {
            return false;
        };
        snapshot
            .pages
            .get(annotation.page)
            .is_some_and(|page| page.fingerprint == binding.page_fingerprint)
            && snapshot.annotations.iter().any(|native| {
                native.page == annotation.page
                    && native.native_id == binding.native_id
                    && native.fingerprint == binding.native_fingerprint
                    && native.rect.is_some_and(|native_rect| {
                        plumbing::pdf_annotations::rects_match(sidecar_rect, native_rect)
                    })
            })
    }

    // Admit one `Cycle color` mutation off the reducer. For a sidecar-only
    // highlight (no native binding, e.g. an imported EPUB CFI) the colour
    // change lands synchronously in the sidecar; only the native branch
    // hands off to the shared bounded worker.
    pub(crate) fn cycle_pdf_annotation_color(&mut self, id: u64) -> (Task<Msg>, AdmissionResult) {
        const COLORS: [&str; 6] = ["yellow", "orange", "red", "magenta", "aqua", "lime"];
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == id && annotation.kind == "highlight")
            .cloned()
        else {
            return (Task::none(), AdmissionResult::Refused);
        };
        let color = COLORS
            .iter()
            .position(|color| *color == annotation.color)
            .map(|index| COLORS[(index + 1) % COLORS.len()])
            .unwrap_or(COLORS[0]);
        let Some(binding) = annotation.pdf_binding.as_ref() else {
            // Sidecar-only highlight (no native peer): the recolour is a
            // pure sidecar mutation and reduces synchronously.
            persistence::update_annotation_color(
                std::sync::Arc::make_mut(&mut self.model.annotations),
                id,
                color,
            );
            self.model
                .undo_stack
                .record(UndoableAction::UpdateAnnotationColor {
                    id,
                    previous: annotation.color.clone(),
                    new: color.to_string(),
                });
            self.rebuild_pdf_annotation_projection();
            self.save_annotations();
            self.model
                .announce(format!("Highlight color changed to {color}"));
            return (Task::none(), AdmissionResult::Admitted);
        };
        let Some(path) = self.model.file_path.clone() else {
            return (Task::none(), AdmissionResult::Refused);
        };
        if !self.native_binding_is_current(&annotation) {
            self.model
                .refuse("This PDF annotation is not safe to recolor.");
            return (Task::none(), AdmissionResult::Refused);
        }
        let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref() else {
            self.model
                .refuse("Native PDF annotations are still being verified.");
            return (Task::none(), AdmissionResult::Refused);
        };
        let contents = snapshot
            .annotations
            .iter()
            .find(|native| native.native_id == binding.native_id)
            .map(|native| native.contents.clone())
            .unwrap_or_else(|| annotation.note.clone());
        let (expected_rect, expected_subtype) =
            native_annotation_rect_and_subtype(snapshot, &binding.native_id, annotation.page);
        let intent = PdfMutationIntent::UpdateColor(PdfUpdateColorIntent {
            path,
            expected_document_fingerprint: snapshot.document_fingerprint.clone(),
            expected_page_fingerprint: binding.page_fingerprint.clone(),
            expected_native_fingerprint: binding.native_fingerprint.clone(),
            page: annotation.page,
            native_id: binding.native_id.clone(),
            sidecar_id: id,
            contents,
            new_color: color.to_string(),
            expected_rect,
            expected_subtype,
        });
        self.admit_pdf_mutation(intent)
    }

    // Native deletion changes sidecar/history only at its confirmed terminal; sidecar-only
    // deletion is synchronous.
    pub(crate) fn remove_annotation(&mut self, id: u64) -> (Task<Msg>, AdmissionResult) {
        let Some(annotation) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|annotation| annotation.id == id)
            .cloned()
        else {
            return (Task::none(), AdmissionResult::Refused);
        };
        if let Some(binding) = annotation.pdf_binding.as_ref() {
            let Some(path) = self.model.file_path.clone() else {
                return (Task::none(), AdmissionResult::Refused);
            };
            if !self.native_binding_is_current(&annotation) {
                self.model
                    .refuse("This PDF annotation is not safe to delete.");
                return (Task::none(), AdmissionResult::Refused);
            }
            let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref() else {
                self.model
                    .refuse("Native PDF annotations are still being verified.");
                return (Task::none(), AdmissionResult::Refused);
            };
            let intent = PdfMutationIntent::Remove(PdfRemoveIntent {
                path,
                expected_document_fingerprint: snapshot.document_fingerprint.clone(),
                expected_page_fingerprint: binding.page_fingerprint.clone(),
                expected_native_fingerprint: binding.native_fingerprint.clone(),
                page: annotation.page,
                native_id: binding.native_id.clone(),
                sidecar_id: id,
            });
            return self.admit_pdf_mutation(intent);
        }
        let epub_chapter =
            match persistence::AnnotationPosition::parse(&annotation.position, annotation.page) {
                persistence::AnnotationPosition::EpubCfi { chapter, .. }
                    if annotation.kind == "highlight" =>
                {
                    Some(chapter)
                }
                _ => None,
            };
        if persistence::remove_annotation(std::sync::Arc::make_mut(&mut self.model.annotations), id)
        {
            self.model
                .undo_stack
                .record(UndoableAction::RemoveAnnotation {
                    annotation: annotation.clone(),
                });
            self.model.pdf_annotation_states.remove(&id);
            if self.model.selected_annotation == Some(id) {
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
        (Task::none(), AdmissionResult::Admitted)
    }

    // Native history changes settle at the mutation terminal; callers may apply sidecar
    // history only for NonNative.
    pub(crate) fn apply_native_history(
        &mut self,
        action: &UndoableAction,
        forward: bool,
    ) -> (Task<Msg>, HistoryDecision) {
        let annotation = match action {
            UndoableAction::AddAnnotation { annotation }
            | UndoableAction::RemoveAnnotation { annotation } => annotation.clone(),
            UndoableAction::UpdateAnnotationText { id, .. }
            | UndoableAction::UpdateAnnotationNote { id, .. }
            | UndoableAction::UpdateAnnotationColor { id, .. } => {
                let Some(annotation) = self
                    .model
                    .annotations
                    .annotations
                    .iter()
                    .find(|annotation| annotation.id == *id)
                    .cloned()
                else {
                    return (Task::none(), HistoryDecision::NonNative);
                };
                annotation
            }
            _ => return (Task::none(), HistoryDecision::NonNative),
        };
        let Some(binding) = annotation.pdf_binding.as_ref() else {
            return (Task::none(), HistoryDecision::NonNative);
        };
        let Some(path) = self.model.file_path.clone() else {
            return (Task::none(), HistoryDecision::Refused);
        };
        let Some(snapshot) = self.model.pdf_annotation_snapshot.as_ref().cloned() else {
            self.model
                .refuse("Native PDF annotations are still being verified.");
            return (Task::none(), HistoryDecision::Refused);
        };
        if snapshot
            .pages
            .get(annotation.page)
            .map(|page| page.fingerprint.as_str())
            != Some(binding.page_fingerprint.as_str())
        {
            self.model
                .refuse("The PDF page changed; undo was not applied to uncertain content.");
            return (Task::none(), HistoryDecision::Refused);
        }
        let native_must_exist = matches!(
            (action, forward),
            (UndoableAction::AddAnnotation { .. }, false)
                | (UndoableAction::RemoveAnnotation { .. }, true)
                | (UndoableAction::UpdateAnnotationText { .. }, _)
                | (UndoableAction::UpdateAnnotationNote { .. }, _)
                | (UndoableAction::UpdateAnnotationColor { .. }, _)
        );
        if native_must_exist && !self.native_binding_is_current(&annotation) {
            self.model
                .refuse("The native PDF annotation changed; undo was not applied.");
            return (Task::none(), HistoryDecision::Refused);
        }

        let sidecar_id = annotation.id;
        let op = match (action, forward) {
            (UndoableAction::AddAnnotation { .. }, false)
            | (UndoableAction::RemoveAnnotation { .. }, true) => {
                plumbing::pdf_ops::PdfMutationOp::Delete {
                    page: annotation.page,
                    native_id: binding.native_id.clone(),
                    expected_page_fingerprint: binding.page_fingerprint.clone(),
                    expected_native_fingerprint: binding.native_fingerprint.clone(),
                }
            }
            (UndoableAction::AddAnnotation { .. }, true)
            | (UndoableAction::RemoveAnnotation { .. }, false) => {
                let Some(rect) =
                    plumbing::pdf_annotations::absolute_rect(&annotation, &snapshot.pages)
                else {
                    self.model
                        .refuse("The PDF annotation geometry is unavailable.");
                    return (Task::none(), HistoryDecision::Refused);
                };
                let contents = if annotation.kind == "highlight" {
                    annotation.note.clone()
                } else {
                    annotation.text.clone()
                };
                plumbing::pdf_ops::PdfMutationOp::Create {
                    page: annotation.page,
                    expected_page_fingerprint: binding.page_fingerprint.clone(),
                    kind: plumbing::pdf_annotations::kind_for(&annotation),
                    rect,
                    quads: annotation
                        .quads
                        .iter()
                        .map(|[x0, y0, x1, y1]| {
                            let points = |v: i32| v as f32 / 100.0;
                            plumbing::pdf_ops::PdfRect::new(
                                points(*x0),
                                points(*y0),
                                points(*x1),
                                points(*y1),
                            )
                        })
                        .collect(),
                    contents,
                    color: annotation.color.clone(),
                    native_id: binding.native_id.clone(),
                }
            }
            (UndoableAction::UpdateAnnotationText { previous, new, .. }, _) => {
                plumbing::pdf_ops::PdfMutationOp::UpdateContents {
                    page: annotation.page,
                    native_id: binding.native_id.clone(),
                    expected_page_fingerprint: binding.page_fingerprint.clone(),
                    expected_native_fingerprint: binding.native_fingerprint.clone(),
                    color: annotation.color.clone(),
                    new_contents: if forward {
                        new.clone()
                    } else {
                        previous.clone()
                    },
                }
            }
            (UndoableAction::UpdateAnnotationNote { previous, new, .. }, _) => {
                plumbing::pdf_ops::PdfMutationOp::UpdateContents {
                    page: annotation.page,
                    native_id: binding.native_id.clone(),
                    expected_page_fingerprint: binding.page_fingerprint.clone(),
                    expected_native_fingerprint: binding.native_fingerprint.clone(),
                    color: annotation.color.clone(),
                    new_contents: if forward {
                        new.clone()
                    } else {
                        previous.clone()
                    },
                }
            }
            (UndoableAction::UpdateAnnotationColor { previous, new, .. }, _) => {
                plumbing::pdf_ops::PdfMutationOp::UpdateColor {
                    page: annotation.page,
                    native_id: binding.native_id.clone(),
                    expected_page_fingerprint: binding.page_fingerprint.clone(),
                    expected_native_fingerprint: binding.native_fingerprint.clone(),
                    contents: annotation.note.clone(),
                    new_color: if forward {
                        new.clone()
                    } else {
                        previous.clone()
                    },
                }
            }
            _ => return (Task::none(), HistoryDecision::NonNative),
        };
        let (expected_rect, expected_subtype) =
            native_annotation_rect_and_subtype(&snapshot, &binding.native_id, annotation.page);
        let intent = PdfMutationIntent::HistoryReplay {
            path,
            expected_document_fingerprint: snapshot.document_fingerprint.clone(),
            op,
            sidecar_id,
            action: Box::new(action.clone()),
            forward,
            expected_subtype,
            expected_rect,
        };
        let (task, result) = self.admit_pdf_mutation(intent);
        let decision = match result {
            AdmissionResult::Admitted => HistoryDecision::NativeAdmitted,
            AdmissionResult::Queued | AdmissionResult::Refused => {
                // History intents never coalesce; either path means the
                // sidecar undo stack must stay intact.
                HistoryDecision::Refused
            }
        };
        (task, decision)
    }
}
