//! App support types and free functions.

use super::classify::probe_exists;
#[path = "support/types.rs"]
mod types;

use super::*;
pub(crate) use types::{
    AdmissionResult, AnnotationCustody, AvailabilityScan, BUSY_RETRY_INTERVAL,
    BlockingOccupancyGuard, CLOSE_DRAIN_BUDGET, CLOSE_DRAIN_TICK, DebtStatus, EditCommentOutcome,
    FsMetadataBackend, HistoryDecision, MetadataBackend, PERSIST_FLUSH_INTERVAL,
    PREVIEW_INFLIGHT_CAP, PdfEditContentsIntent, PdfMutationInFlight, PdfMutationPending,
    PdfReconciliationClass, PdfReconciliationIntent, PdfRemoveIntent, PdfUpdateColorIntent,
    PendingDocumentLoad, PendingLoad, PendingPersistence, PendingPick, PersistenceSnapshot,
    UncertainPublication, export_result_to_outcome, overlay_restore_id, reading_theme_for_mode,
    restore_focus_task,
};
pub use types::{CloseState, PdfCreateIntent, PdfMutationIntent};

// Free-function classifier used by the reducer helper and the
// Retry blocking closure. Kept out of `impl App` so the closure
// (which cannot hold `&self`) can call it directly.
pub(crate) fn classify_reconciliation_from_bytes(
    expected_pre_document_fingerprint: &str,
    expected_post_document_fingerprint: Option<&str>,
    intended: &PdfReconciliationIntent,
    page: usize,
    native_id: &str,
    previous_bytes: Option<&[u8]>,
    staged_or_current_bytes: Option<&[u8]>,
) -> PdfReconciliationClass {
    let previous_snapshot = previous_bytes.and_then(plumbing::pdf_ops::snapshot_from_bytes);
    let staged_snapshot = staged_or_current_bytes.and_then(plumbing::pdf_ops::snapshot_from_bytes);
    let previous_present = previous_bytes.is_some();
    let staged_present = staged_or_current_bytes.is_some();
    let previous_matches_pre = previous_snapshot.as_ref().is_some_and(|snapshot| {
        !expected_pre_document_fingerprint.is_empty()
            && snapshot.document_fingerprint == expected_pre_document_fingerprint
    });
    let intent_predicate_holds_on =
        |snapshot: &plumbing::pdf_ops::PdfAnnotationSnapshot| match intended {
            PdfReconciliationIntent::Present {
                expected_native_fingerprint,
            } => snapshot.annotations.iter().any(|native| {
                native.page == page
                    && native.native_id == native_id
                    && expected_native_fingerprint
                        .as_ref()
                        .is_none_or(|fp| &native.fingerprint == fp)
            }),
            PdfReconciliationIntent::Absent => !snapshot
                .annotations
                .iter()
                .any(|native| native.page == page && native.native_id == native_id),
        };
    let staged_matches_post = staged_snapshot.as_ref().is_some_and(|snapshot| {
        expected_post_document_fingerprint.is_some_and(|fp| snapshot.document_fingerprint == fp)
            && intent_predicate_holds_on(snapshot)
    });
    // Only exactly one matching retained observation classifies; everything else stays
    // StillUncertain.
    match (
        previous_present,
        staged_present,
        previous_matches_pre,
        staged_matches_post,
    ) {
        (true, false, true, _) => PdfReconciliationClass::NotApplied,
        (false, true, _, true) => PdfReconciliationClass::Applied,
        _ => PdfReconciliationClass::StillUncertain,
    }
}

// Authority reader for the Retry closure, bounded by
// [`plumbing::pdf_ops::PDF_MUTATION_MAX_BYTES`]. Any read error or over-cap read yields
// [`PdfReconciliationClass::StillUncertain`], so I/O failure never clears debt.
pub(crate) fn classify_reconciliation_via_authority(
    authority: &enzui_iced_core::user_file::DocumentAuthority,
    expected_pre_document_fingerprint: &str,
    expected_post_document_fingerprint: Option<&str>,
    intended: &PdfReconciliationIntent,
    page: usize,
    native_id: &str,
) -> PdfReconciliationClass {
    use enzui_iced_core::user_file::{DocumentSnapshot, HandleRead};
    match authority.read_bounded(plumbing::pdf_ops::PDF_MUTATION_MAX_BYTES) {
        Ok(DocumentSnapshot::Current {
            bytes,
            overflow: false,
            ..
        }) => classify_reconciliation_from_bytes(
            expected_pre_document_fingerprint,
            expected_post_document_fingerprint,
            intended,
            page,
            native_id,
            None,
            Some(&bytes),
        ),
        Ok(DocumentSnapshot::Current { overflow: true, .. }) => {
            PdfReconciliationClass::StillUncertain
        }
        Ok(DocumentSnapshot::Lost) => {
            let Some(reconciliation) =
                authority.lost_snapshots(plumbing::pdf_ops::PDF_MUTATION_MAX_BYTES)
            else {
                return PdfReconciliationClass::StillUncertain;
            };
            let previous_bytes = match reconciliation.previous() {
                Some(HandleRead::Read {
                    bytes,
                    overflow: false,
                    ..
                }) => Some(bytes.as_slice()),
                _ => None,
            };
            let staged_bytes = match reconciliation.staged() {
                Some(HandleRead::Read {
                    bytes,
                    overflow: false,
                    ..
                }) => Some(bytes.as_slice()),
                _ => None,
            };
            classify_reconciliation_from_bytes(
                expected_pre_document_fingerprint,
                expected_post_document_fingerprint,
                intended,
                page,
                native_id,
                previous_bytes,
                staged_bytes,
            )
        }
        Err(_) => PdfReconciliationClass::StillUncertain,
    }
}

impl PdfMutationIntent {
    pub(crate) fn path(&self) -> &std::path::Path {
        match self {
            PdfMutationIntent::Create(create) => &create.path,
            PdfMutationIntent::EditContents(edit) => &edit.path,
            PdfMutationIntent::UpdateColor(update) => &update.path,
            PdfMutationIntent::Remove(remove) => &remove.path,
            PdfMutationIntent::HistoryReplay { path, .. } => path,
        }
    }

    pub(crate) fn page(&self) -> usize {
        match self {
            PdfMutationIntent::Create(create) => create.selection.page,
            PdfMutationIntent::EditContents(edit) => edit.page,
            PdfMutationIntent::UpdateColor(update) => update.page,
            PdfMutationIntent::Remove(remove) => remove.page,
            PdfMutationIntent::HistoryReplay { op, .. } => op.page(),
        }
    }

    pub(crate) fn native_id(&self) -> String {
        match self {
            PdfMutationIntent::Create(create) => create.native_id.clone(),
            PdfMutationIntent::EditContents(edit) => edit.native_id.clone(),
            PdfMutationIntent::UpdateColor(update) => update.native_id.clone(),
            PdfMutationIntent::Remove(remove) => remove.native_id.clone(),
            PdfMutationIntent::HistoryReplay { op, .. } => op.native_id().to_string(),
        }
    }

    pub(crate) fn to_request(&self) -> plumbing::pdf_ops::PdfMutationRequest {
        use plumbing::pdf_ops::{PdfMutationOp, PdfMutationRequest};
        let op = match self {
            PdfMutationIntent::Create(create) => PdfMutationOp::Create {
                page: create.selection.page,
                expected_page_fingerprint: create.expected_page_fingerprint.clone(),
                kind: create.kind,
                rect: create.selection.rect,
                quads: create.selection.quads.clone(),
                contents: create.authored_contents.clone(),
                color: create.color.clone(),
                native_id: create.native_id.clone(),
            },
            PdfMutationIntent::EditContents(edit) => PdfMutationOp::UpdateContents {
                page: edit.page,
                native_id: edit.native_id.clone(),
                expected_page_fingerprint: edit.expected_page_fingerprint.clone(),
                expected_native_fingerprint: edit.expected_native_fingerprint.clone(),
                color: edit.color.clone(),
                new_contents: edit.new_contents.clone(),
            },
            PdfMutationIntent::UpdateColor(update) => PdfMutationOp::UpdateColor {
                page: update.page,
                native_id: update.native_id.clone(),
                expected_page_fingerprint: update.expected_page_fingerprint.clone(),
                expected_native_fingerprint: update.expected_native_fingerprint.clone(),
                contents: update.contents.clone(),
                new_color: update.new_color.clone(),
            },
            PdfMutationIntent::Remove(remove) => PdfMutationOp::Delete {
                page: remove.page,
                native_id: remove.native_id.clone(),
                expected_page_fingerprint: remove.expected_page_fingerprint.clone(),
                expected_native_fingerprint: remove.expected_native_fingerprint.clone(),
            },
            PdfMutationIntent::HistoryReplay { op, .. } => op.clone(),
        };
        PdfMutationRequest {
            path: self.path().to_path_buf(),
            expected_document_fingerprint: match self {
                PdfMutationIntent::Create(create) => create.expected_document_fingerprint.clone(),
                PdfMutationIntent::EditContents(edit) => edit.expected_document_fingerprint.clone(),
                PdfMutationIntent::UpdateColor(update) => {
                    update.expected_document_fingerprint.clone()
                }
                PdfMutationIntent::Remove(remove) => remove.expected_document_fingerprint.clone(),
                PdfMutationIntent::HistoryReplay {
                    expected_document_fingerprint,
                    ..
                } => expected_document_fingerprint.clone(),
            },
            op,
        }
    }

    // Whether `self` names the same PDF and target annotation identity as `other`; with
    // the op token this rejects coincidental token collisions across a document swap.
    pub(crate) fn same_stable_identity(&self, other: &PdfMutationIntent) -> bool {
        if self.path() != other.path() {
            return false;
        }
        match (self, other) {
            (PdfMutationIntent::Create(a), PdfMutationIntent::Create(b)) => {
                a.native_id == b.native_id && a.selection.page == b.selection.page
            }
            (PdfMutationIntent::EditContents(a), PdfMutationIntent::EditContents(b)) => {
                a.native_id == b.native_id && a.page == b.page && a.sidecar_id == b.sidecar_id
            }
            (PdfMutationIntent::UpdateColor(a), PdfMutationIntent::UpdateColor(b)) => {
                a.native_id == b.native_id && a.page == b.page && a.sidecar_id == b.sidecar_id
            }
            (PdfMutationIntent::Remove(a), PdfMutationIntent::Remove(b)) => {
                a.native_id == b.native_id && a.page == b.page && a.sidecar_id == b.sidecar_id
            }
            (
                PdfMutationIntent::HistoryReplay {
                    sidecar_id: a_id,
                    forward: a_forward,
                    op: a_op,
                    ..
                },
                PdfMutationIntent::HistoryReplay {
                    sidecar_id: b_id,
                    forward: b_forward,
                    op: b_op,
                    ..
                },
            ) => {
                a_id == b_id
                    && a_forward == b_forward
                    && plumbing_op_native_id(a_op) == plumbing_op_native_id(b_op)
            }
            _ => false,
        }
    }

    // True when the reducer must not coalesce a second admission of
    // this shape into a retained latest intent — an
    // [`AdmissionResult::Refused`] outcome instead.
    pub(crate) fn refuses_coalescing(&self) -> bool {
        matches!(self, PdfMutationIntent::HistoryReplay { .. })
    }

    // The pre-mutation document fingerprint captured at admission; a retained snapshot
    // matching it classifies as [`PdfReconciliationClass::NotApplied`].
    pub(crate) fn expected_pre_document_fingerprint(&self) -> String {
        match self {
            PdfMutationIntent::Create(create) => create.expected_document_fingerprint.clone(),
            PdfMutationIntent::EditContents(edit) => edit.expected_document_fingerprint.clone(),
            PdfMutationIntent::UpdateColor(update) => update.expected_document_fingerprint.clone(),
            PdfMutationIntent::Remove(remove) => remove.expected_document_fingerprint.clone(),
            PdfMutationIntent::HistoryReplay {
                expected_document_fingerprint,
                ..
            } => expected_document_fingerprint.clone(),
        }
    }

    // Use the reopened fingerprint when available; an unverified create can reconcile only its
    // admitted annotation id.
    pub(crate) fn intended_reconciliation(
        &self,
        outcome_refreshed: Option<&plumbing::pdf_ops::PdfMutationRefreshed>,
    ) -> PdfReconciliationIntent {
        use plumbing::pdf_ops::annotation_fingerprint;
        let refreshed_native = outcome_refreshed.map(|r| r.native_fingerprint.clone());
        match self {
            PdfMutationIntent::Remove(_) => PdfReconciliationIntent::Absent,
            PdfMutationIntent::HistoryReplay {
                op,
                expected_subtype,
                expected_rect,
                ..
            } => match op {
                plumbing::pdf_ops::PdfMutationOp::Delete { .. } => PdfReconciliationIntent::Absent,
                plumbing::pdf_ops::PdfMutationOp::Create { .. } => {
                    PdfReconciliationIntent::Present {
                        expected_native_fingerprint: refreshed_native,
                    }
                }
                plumbing::pdf_ops::PdfMutationOp::UpdateContents {
                    new_contents,
                    color,
                    ..
                } => PdfReconciliationIntent::Present {
                    expected_native_fingerprint: Some(refreshed_native.unwrap_or_else(|| {
                        annotation_fingerprint(
                            expected_subtype,
                            new_contents,
                            color,
                            *expected_rect,
                        )
                    })),
                },
                plumbing::pdf_ops::PdfMutationOp::UpdateColor {
                    new_color,
                    contents,
                    ..
                } => PdfReconciliationIntent::Present {
                    expected_native_fingerprint: Some(refreshed_native.unwrap_or_else(|| {
                        annotation_fingerprint(
                            expected_subtype,
                            contents,
                            new_color,
                            *expected_rect,
                        )
                    })),
                },
            },
            PdfMutationIntent::Create(_) => PdfReconciliationIntent::Present {
                expected_native_fingerprint: refreshed_native,
            },
            PdfMutationIntent::EditContents(edit) => PdfReconciliationIntent::Present {
                expected_native_fingerprint: Some(refreshed_native.unwrap_or_else(|| {
                    annotation_fingerprint(
                        edit.expected_subtype,
                        &edit.new_contents,
                        &edit.color,
                        edit.expected_rect,
                    )
                })),
            },
            PdfMutationIntent::UpdateColor(update) => PdfReconciliationIntent::Present {
                expected_native_fingerprint: Some(refreshed_native.unwrap_or_else(|| {
                    annotation_fingerprint(
                        update.expected_subtype,
                        &update.contents,
                        &update.new_color,
                        update.expected_rect,
                    )
                })),
            },
        }
    }
}

// Look up the rect and subtype of one native annotation in the current snapshot, so an
// edit or recolour can reproduce the post-mutation fingerprint. Falls back to
// `Highlight`; a worker's refreshed fingerprint supersedes the fallback.
pub(crate) fn native_annotation_rect_and_subtype(
    snapshot: &plumbing::pdf_ops::PdfAnnotationSnapshot,
    native_id: &str,
    page: usize,
) -> (Option<plumbing::pdf_ops::PdfRect>, &'static str) {
    let native = snapshot
        .annotations
        .iter()
        .find(|n| n.native_id == native_id && n.page == page);
    let rect = native.and_then(|n| n.rect);
    let comment_subtype = plumbing::pdf_ops::PdfAnnotationKind::Comment.subtype();
    let subtype = if native
        .map(|n| n.subtype.as_str() == comment_subtype)
        .unwrap_or(false)
    {
        comment_subtype
    } else {
        plumbing::pdf_ops::PdfAnnotationKind::Highlight.subtype()
    };
    (rect, subtype)
}

pub(crate) fn plumbing_op_native_id(op: &plumbing::pdf_ops::PdfMutationOp) -> &str {
    match op {
        plumbing::pdf_ops::PdfMutationOp::Create { native_id, .. }
        | plumbing::pdf_ops::PdfMutationOp::UpdateContents { native_id, .. }
        | plumbing::pdf_ops::PdfMutationOp::UpdateColor { native_id, .. }
        | plumbing::pdf_ops::PdfMutationOp::Delete { native_id, .. } => native_id.as_str(),
    }
}

// Bounded summary of an uncertain publication: it never claims Unchanged or Success and
// promises the exact file is rechecked before another native write.
pub(crate) const UNCERTAIN_PUBLICATION_SUMMARY: &str = "PDF publication occurred, but current identity or durability could not be confirmed. The exact file will be rechecked before another native write.";

// A native mutation applied but the undo stack drifted, so PDF bytes and sidecar
// diverge; the file is rechecked before another native write.
pub(crate) const HISTORY_STACK_DRIFT_SUMMARY: &str = "A native PDF mutation applied, but the undo stack changed before the sidecar could be updated to match. The exact file will be rechecked before another native write.";

// Retry reopened the PDF and the retained snapshot matched the
// record's intended consequence. Sidecar / history were reconciled;
// the record cleared.
pub(crate) const UNCERTAIN_RETRY_CLEARED_SUMMARY: &str =
    "Reopen confirmed the PDF publication matches the intended change; uncertainty cleared.";

// Retry matched the pre-mutation fingerprint: the mutation did not apply, sidecar and
// history were rolled back, and the record cleared.
pub(crate) const UNCERTAIN_RETRY_NOT_APPLIED_SUMMARY: &str = "Reopen confirmed the PDF did not change; any optimistic sidecar changes were rolled back and uncertainty cleared.";

// Retry observed neither the pre-mutation identity nor the intended consequence; that is
// identity drift, so the record stays.
pub(crate) const UNCERTAIN_RETRY_MISMATCH_SUMMARY: &str = "Reopen did not match either the pre-mutation identity or the intended change; uncertainty remains and the exact file will be rechecked before another native write.";

// Retry’s reopen attempt itself failed. The debt stays visible so a
// later retry (or Dismiss) is still the only way to resolve
// close-drain.
pub(crate) const UNCERTAIN_RETRY_FAILED_SUMMARY: &str = "Reopen for uncertainty retry did not complete; uncertainty remains and the exact file will be rechecked before another native write.";

// Dismissal copy: only the banner goes; the record persists and the file is rechecked
// before another native write.
pub(crate) const UNCERTAIN_DISMISSED_SUMMARY: &str = "Uncertainty banner dismissed. Any queued PDF annotation mutations were cancelled; the exact file will be rechecked before another native write.";

// Refusal copy when a reconciliation record still gates this path, repeating the
// Dismiss / Retry promise.
pub(crate) const RECONCILIATION_BLOCKED_ADMIT_SUMMARY: &str = "A previous PDF publication is still awaiting recheck on this file; open the file again to reconcile before another native write.";

// Refusal copy while the writer closure still runs after a Cancelled / Shutdown /
// Panicked terminal.
pub(crate) const WRITER_OCCUPANCY_BLOCKED_ADMIT_SUMMARY: &str =
    "The previous PDF writer has not finished. New annotation writes are refused until it exits.";

// Retry owns the retained authority until scope exit, vetoing close and concurrent mutation.
pub(crate) const RETRY_OCCUPANCY_BLOCKED_ADMIT_SUMMARY: &str = "Retry is still rechecking the exact file. New annotation writes are refused until the recheck exits.";

// Retry sequence exhausted. The `u64` counter cannot advance without
// wrapping into a value that would collide with an in-flight stale
// retry terminal, so the retry is refused visibly instead of silently.
pub(crate) const UNCERTAIN_RETRY_EXHAUSTED_SUMMARY: &str =
    "Uncertainty retry counter exhausted. Restart the reader to retry.";

// Retained slot cancelled by Dismiss. Each queued native mutation
// (older busy-retry and newer pending) was admitted against
// pre-uncertainty bytes and cannot run on unknown disk state.
pub(crate) const RETAINED_SLOT_CANCELLED_BY_DISMISS_SUMMARY: &str =
    "A queued PDF annotation mutation was cancelled by the uncertainty dismissal.";

// Fixed path-free user summary for one worker terminal that never
// reached the worker (Cancelled / Shutdown / Panicked). Distinct from
// [`UNCERTAIN_PUBLICATION_SUMMARY`], which describes an actual publish.
pub(crate) fn pdf_worker_terminal_message(terminal: BlockingTaskError) -> &'static str {
    match terminal {
        BlockingTaskError::Busy => {
            "PDF annotation task refused: the worker pool is temporarily full."
        }
        BlockingTaskError::Cancelled => "PDF annotation task cancelled; the file was not modified.",
        BlockingTaskError::Shutdown => {
            "PDF annotation task did not finish before the worker pool shut down."
        }
        BlockingTaskError::Panicked => {
            "PDF annotation task ended abnormally; the file was not modified."
        }
        _ => "PDF annotation task did not complete; the file was not modified.",
    }
}
