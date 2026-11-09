//! Annotation mutations: ops, custody, and outcomes.

use super::*;

/// One native-PDF annotation mutation staged for off-reducer publication.
///
/// The reducer captures everything the worker needs at admission; the worker never reads
/// reducer state and the reducer never touches lopdf.
#[derive(Clone, Debug)]
pub struct PdfMutationRequest {
    /// The exact PDF path this mutation is authorised against; the terminal drops the
    /// completion when another document is open.
    pub path: PathBuf,
    /// Document fingerprint from the last reconciled snapshot; the worker refuses if the
    /// reopened document's differs.
    pub expected_document_fingerprint: String,
    /// The operation to apply: create, update or delete, including undo/redo reissues.
    pub op: PdfMutationOp,
}

/// One native annotation operation shared across create, edit, colour,
/// delete, and their history (undo/redo) reissues.
#[derive(Clone, Debug)]
pub enum PdfMutationOp {
    /// Write a new highlight or comment. The reducer chooses `native_id` from the sidecar
    /// counter, so the terminal knows which annotation to bind.
    Create {
        page: usize,
        expected_page_fingerprint: String,
        kind: PdfAnnotationKind,
        rect: PdfRect,
        /// A highlight's per-line boxes; empty means `rect` alone.
        quads: Vec<PdfRect>,
        contents: String,
        color: String,
        native_id: String,
    },
    /// Rewrite an existing annotation’s `/Contents`.
    UpdateContents {
        page: usize,
        native_id: String,
        expected_page_fingerprint: String,
        expected_native_fingerprint: String,
        color: String,
        new_contents: String,
    },
    /// Rewrite an existing highlight’s `/C` colour.
    UpdateColor {
        page: usize,
        native_id: String,
        expected_page_fingerprint: String,
        expected_native_fingerprint: String,
        contents: String,
        new_color: String,
    },
    /// Remove one annotation by its `/NM`.
    Delete {
        page: usize,
        native_id: String,
        expected_page_fingerprint: String,
        expected_native_fingerprint: String,
    },
}

/// Maximum bytes the annotation-mutation authority path accepts for one input read or one
/// serialized output, bounding the retained descriptor's memory per mutation.
pub const PDF_MUTATION_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The custody of one native-PDF annotation mutation on return from the
/// worker.
#[derive(Clone, Debug)]
pub enum PdfMutationCustody {
    /// The mutation was applied and the retained rotated authority bytes verify it.
    Applied,
    /// The mutation was refused before any publication syscall; the bytes on disk are
    /// unchanged.
    Refused { reason: PdfMutationRefusal },
    /// A publication syscall or later read failed in a way that cannot count as Applied;
    /// the bytes on disk may or may not be the pre-mutation document, so this is never
    /// presented as unchanged.
    ///
    /// `adopted_via_reopen` means a best-effort read of the rotated authority found the
    /// target identity. `lost_custody` means `DocumentAuthority::replace` or a post-publish
    /// `read_bounded` reported `Lost`; the reducer then claims the retained authority from
    /// the handoff slot so `lost_snapshots(cap)` reconciles without reopening the path.
    Uncertain {
        detail: String,
        adopted_via_reopen: bool,
        lost_custody: bool,
    },
}

/// The typed reason the worker refused a mutation before any publication.
#[derive(Clone, Debug)]
pub enum PdfMutationRefusal {
    /// Reopen failed or the file cannot be parsed as a PDF.
    OpenFailed { detail: String },
    /// The reopened PDF is encrypted and must be unlocked first.
    Encrypted,
    /// The document fingerprint drifted between admission and the worker
    /// reopen.
    DocumentIdentityDrifted,
    /// The named page’s fingerprint drifted between admission and the
    /// worker reopen.
    PageIdentityDrifted { page: usize },
    /// The named native annotation is no longer present at the expected
    /// page.
    NativeAnnotationMissing { page: usize, native_id: String },
    /// The native annotation’s fingerprint drifted between admission and
    /// the worker reopen.
    NativeAnnotationChanged { page: usize, native_id: String },
    /// The named page does not exist in the reopened document.
    PageMissing { page: usize },
    /// Any other worker-visible write failure, surfaced through
    /// [`Self::user_summary`].
    WriteFailed { detail: String },
}

impl PdfMutationRefusal {
    /// User-visible summary for the banner path that also carries [`PdfWriteError`].
    pub fn user_summary(&self) -> String {
        match self {
            Self::OpenFailed { .. } => "Native PDF annotations could not be verified.".to_string(),
            Self::Encrypted => {
                "This PDF is encrypted and cannot be annotated while locked.".to_string()
            }
            Self::DocumentIdentityDrifted => {
                "The PDF changed; the mutation was not applied.".to_string()
            }
            Self::PageIdentityDrifted { .. } => {
                "The PDF page changed; its annotations were left unmodified and marked orphaned."
                    .to_string()
            }
            Self::NativeAnnotationMissing { .. } => {
                "The native PDF annotation no longer exists.".to_string()
            }
            Self::NativeAnnotationChanged { .. } => {
                "The native PDF annotation changed; the mutation was not applied.".to_string()
            }
            Self::PageMissing { page } => {
                format!("Page {} does not exist.", page + 1)
            }
            Self::WriteFailed { detail } => detail.clone(),
        }
    }
}

/// The terminal the worker returns and the reducer consumes for every
/// annotation mutation, including history reissues.
#[derive(Clone, Debug)]
pub struct PdfMutationOutcome {
    /// The path the worker actually operated against.
    pub path: PathBuf,
    /// The custody of the publication (applied / refused / uncertain).
    pub custody: PdfMutationCustody,
    /// The reopened snapshot, when the worker read one. Normally `None` on `Refused`;
    /// present on `Uncertain { adopted_via_reopen: true }`.
    pub snapshot: Option<PdfAnnotationSnapshot>,
    /// The target native identity as the worker saw it, kept on `Refused` so the reducer
    /// can name the annotation.
    pub native_id: String,
    /// The 0-indexed page the mutation targeted.
    pub page: usize,
    /// Fingerprints of the reopened annotation, from `Applied` or from an
    /// `Uncertain { adopted_via_reopen: true }` snapshot containing the target.
    pub refreshed: Option<PdfMutationRefreshed>,
}

/// The subset of a reopened snapshot the reducer needs to refresh a
/// [`crate::plumbing::persistence::PdfAnnotationBinding`] without touching
/// the file itself.
#[derive(Clone, Debug)]
pub struct PdfMutationRefreshed {
    pub document_fingerprint: String,
    pub page_fingerprint: String,
    pub native_fingerprint: String,
    pub native_contents: String,
    pub native_color: String,
    pub native_rect: Option<PdfRect>,
    pub native_has_quad_points: bool,
}

impl PdfMutationOp {
    pub fn page(&self) -> usize {
        match self {
            Self::Create { page, .. }
            | Self::UpdateContents { page, .. }
            | Self::UpdateColor { page, .. }
            | Self::Delete { page, .. } => *page,
        }
    }

    fn expected_page_fingerprint(&self) -> &str {
        match self {
            Self::Create {
                expected_page_fingerprint,
                ..
            }
            | Self::UpdateContents {
                expected_page_fingerprint,
                ..
            }
            | Self::UpdateColor {
                expected_page_fingerprint,
                ..
            }
            | Self::Delete {
                expected_page_fingerprint,
                ..
            } => expected_page_fingerprint,
        }
    }

    pub fn native_id(&self) -> &str {
        match self {
            Self::Create { native_id, .. }
            | Self::UpdateContents { native_id, .. }
            | Self::UpdateColor { native_id, .. }
            | Self::Delete { native_id, .. } => native_id,
        }
    }
}

/// Apply one native-PDF annotation mutation off the reducer.
///
/// Reopens the PDF, revalidates every expected identity in `request`, applies the
/// mutation with `lopdf`, publishes atomically through
/// [`user_file::save_with`](enzui_iced_core::user_file::save_with) under
/// `SavePolicy::ReplaceExisting`, and returns the reopened [`PdfAnnotationSnapshot`] with
/// a refreshed binding. Failures are typed [`PdfMutationCustody`], so the reducer decides
/// whether to update the sidecar and undo history.
pub fn apply_annotation_mutation(
    request: PdfMutationRequest,
    lost_authority_slot: std::sync::Arc<
        std::sync::Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>,
    >,
) -> PdfMutationOutcome {
    use enzui_iced_core::user_file::{
        DocumentAuthority, DocumentSnapshot, ReadError, ReplaceCustody,
    };

    let path = request.path.clone();
    let page = request.op.page();
    let native_id = request.op.native_id().to_string();
    let refused =
        |reason: PdfMutationRefusal, snapshot: Option<PdfAnnotationSnapshot>| PdfMutationOutcome {
            path: path.clone(),
            custody: PdfMutationCustody::Refused { reason },
            snapshot,
            native_id: native_id.clone(),
            page,
            refreshed: None,
        };
    let uncertain = |detail: String,
                     adopted: bool,
                     lost: bool,
                     snapshot: Option<PdfAnnotationSnapshot>,
                     refreshed: Option<PdfMutationRefreshed>| {
        PdfMutationOutcome {
            path: path.clone(),
            custody: PdfMutationCustody::Uncertain {
                detail,
                adopted_via_reopen: adopted,
                lost_custody: lost,
            },
            snapshot,
            native_id: native_id.clone(),
            page,
            refreshed,
        }
    };
    // On `Lost` custody, move `authority` into the handoff slot so the reducer can call
    // `lost_snapshots(cap)`. Called at most once.
    let hand_off = |slot: &std::sync::Arc<std::sync::Mutex<Option<DocumentAuthority>>>,
                    authority: DocumentAuthority| {
        if let Ok(mut guard) = slot.lock() {
            let _ = guard.replace(authority);
        }
    };

    // One retained authority: every read, mutation and publish addresses the same
    // descriptor pair, and no pathname read runs on this path.
    let mut authority = match DocumentAuthority::open_existing(&path) {
        Ok(authority) => authority,
        Err(error) => {
            return refused(
                PdfMutationRefusal::OpenFailed {
                    detail: error.to_string(),
                },
                None,
            );
        }
    };
    let admission_bytes = match authority.read_bounded(PDF_MUTATION_MAX_BYTES) {
        Ok(DocumentSnapshot::Current {
            bytes,
            overflow: false,
            ..
        }) => bytes,
        Ok(DocumentSnapshot::Current { overflow: true, .. }) => {
            return refused(
                PdfMutationRefusal::WriteFailed {
                    detail: format!(
                        "PDF exceeds the {PDF_MUTATION_MAX_BYTES}-byte annotation mutation cap."
                    ),
                },
                None,
            );
        }
        Ok(DocumentSnapshot::Lost) => {
            hand_off(&lost_authority_slot, authority);
            return uncertain(
                "retained authority reported lost custody before the first read.".to_string(),
                false,
                true,
                None,
                None,
            );
        }
        Err(ReadError::ConcurrentMutation) => {
            return refused(
                PdfMutationRefusal::WriteFailed {
                    detail: "the retained descriptor observed a concurrent in-place mutation."
                        .to_string(),
                },
                None,
            );
        }
        Err(ReadError::CapOverflow) => {
            return refused(
                PdfMutationRefusal::WriteFailed {
                    detail: "annotation mutation cap cannot represent an overflow buffer."
                        .to_string(),
                },
                None,
            );
        }
        Err(ReadError::Io(error)) => {
            return refused(
                PdfMutationRefusal::OpenFailed {
                    detail: error.to_string(),
                },
                None,
            );
        }
    };

    let mut doc = match Document::load_mem(&admission_bytes) {
        Ok(doc) => doc,
        Err(error) => {
            return refused(
                PdfMutationRefusal::OpenFailed {
                    detail: error.to_string(),
                },
                None,
            );
        }
    };
    if doc.is_encrypted() {
        return refused(PdfMutationRefusal::Encrypted, None);
    }
    let admission_snapshot = match snapshot_from_document(&doc) {
        Ok(snapshot) => snapshot,
        Err(detail) => return refused(PdfMutationRefusal::OpenFailed { detail }, None),
    };
    if admission_snapshot.document_fingerprint != request.expected_document_fingerprint {
        return refused(
            PdfMutationRefusal::DocumentIdentityDrifted,
            Some(admission_snapshot),
        );
    }
    let Some(page_identity) = admission_snapshot.pages.get(page).cloned() else {
        return refused(
            PdfMutationRefusal::PageMissing { page },
            Some(admission_snapshot),
        );
    };
    if page_identity.fingerprint != request.op.expected_page_fingerprint() {
        return refused(
            PdfMutationRefusal::PageIdentityDrifted { page },
            Some(admission_snapshot),
        );
    }
    match &request.op {
        PdfMutationOp::UpdateContents {
            expected_native_fingerprint,
            ..
        }
        | PdfMutationOp::UpdateColor {
            expected_native_fingerprint,
            ..
        }
        | PdfMutationOp::Delete {
            expected_native_fingerprint,
            ..
        } => {
            let native = admission_snapshot
                .annotations
                .iter()
                .find(|native| native.page == page && native.native_id == native_id);
            let Some(native) = native else {
                return refused(
                    PdfMutationRefusal::NativeAnnotationMissing {
                        page,
                        native_id: native_id.clone(),
                    },
                    Some(admission_snapshot),
                );
            };
            if &native.fingerprint != expected_native_fingerprint {
                return refused(
                    PdfMutationRefusal::NativeAnnotationChanged {
                        page,
                        native_id: native_id.clone(),
                    },
                    Some(admission_snapshot),
                );
            }
        }
        PdfMutationOp::Create { .. } => {}
    }
    drop(admission_bytes);

    let inline_error = match &request.op {
        PdfMutationOp::Create {
            kind,
            rect,
            quads,
            contents,
            color,
            ..
        } => add_annotation_to_document(
            &mut doc,
            page,
            *kind,
            *rect,
            quads,
            contents,
            color,
            Some(&native_id),
        )
        .err(),
        PdfMutationOp::UpdateContents { new_contents, .. } => {
            update_annotation_in_document(&mut doc, page, &native_id, Some(new_contents), None)
                .err()
        }
        PdfMutationOp::UpdateColor { new_color, .. } => {
            update_annotation_in_document(&mut doc, page, &native_id, None, Some(new_color)).err()
        }
        PdfMutationOp::Delete { .. } => {
            delete_annotation_from_document(&mut doc, page, &native_id).err()
        }
    };
    if let Some(error) = inline_error {
        return refused(
            PdfMutationRefusal::WriteFailed {
                detail: error.to_string(),
            },
            Some(admission_snapshot),
        );
    }

    // `Vec::write_all` cannot fail; only lopdf's serializer can.
    let mut serialized: Vec<u8> =
        Vec::with_capacity((PDF_MUTATION_MAX_BYTES / 8).min(admission_snapshot.pages.len() * 4096));
    if let Err(error) = doc.save_to(&mut serialized) {
        return refused(
            PdfMutationRefusal::WriteFailed {
                detail: error.to_string(),
            },
            Some(admission_snapshot),
        );
    }
    if serialized.len() > PDF_MUTATION_MAX_BYTES {
        return refused(
            PdfMutationRefusal::WriteFailed {
                detail: format!(
                    "serialized post-mutation PDF ({} bytes) exceeds the {PDF_MUTATION_MAX_BYTES}-byte cap.",
                    serialized.len()
                ),
            },
            Some(admission_snapshot),
        );
    }
    drop(doc);

    // The expected post-publication fingerprint, from the exact bytes being published.
    let expected_post_snapshot = match Document::load_mem(&serialized)
        .map_err(|e| e.to_string())
        .and_then(|d| snapshot_from_document(&d))
    {
        Ok(snapshot) => snapshot,
        Err(detail) => return uncertain(detail, false, false, None, None),
    };

    match authority.replace(&serialized) {
        Ok(_identity) => {
            drop(serialized);
            // Test seam for the published-but-unconfirmed fault, reaching the reducer's
            // uncertain-publication phase.
            #[cfg(test)]
            if let Some(detail) = crate::plumbing::pdf_ops::save_fault::take_published_failure() {
                return uncertain(detail, false, false, None, None);
            }
            // Verify from the rotated authority's retained bytes, never the pathname.
            let verify_bytes = match authority.read_bounded(PDF_MUTATION_MAX_BYTES) {
                Ok(DocumentSnapshot::Current {
                    bytes,
                    overflow: false,
                    ..
                }) => bytes,
                Ok(DocumentSnapshot::Current { overflow: true, .. }) => {
                    return uncertain(
                        "post-publication read exceeded the mutation cap.".to_string(),
                        false,
                        false,
                        None,
                        None,
                    );
                }
                Ok(DocumentSnapshot::Lost) => {
                    hand_off(&lost_authority_slot, authority);
                    return uncertain(
                        "authority reported lost custody after publication.".to_string(),
                        false,
                        true,
                        None,
                        None,
                    );
                }
                Err(error) => {
                    return uncertain(
                        format!("post-publication read failed: {error}"),
                        false,
                        false,
                        None,
                        None,
                    );
                }
            };
            let verify_snapshot = match Document::load_mem(&verify_bytes)
                .map_err(|e| e.to_string())
                .and_then(|d| snapshot_from_document(&d))
            {
                Ok(s) => s,
                Err(detail) => return uncertain(detail, false, false, None, None),
            };
            if verify_snapshot.document_fingerprint != expected_post_snapshot.document_fingerprint {
                return uncertain(
                    "rotated authority bytes do not match the intended serialized post-mutation state."
                        .to_string(),
                    false,
                    false,
                    Some(verify_snapshot),
                    None,
                );
            }
            let refreshed =
                refreshed_from_snapshot(&verify_snapshot, page, &native_id, &request.op);
            let expected_present = !matches!(&request.op, PdfMutationOp::Delete { .. });
            if expected_present && refreshed.is_none() {
                return uncertain(
                    "rotated authority does not carry the expected native annotation.".to_string(),
                    false,
                    false,
                    Some(verify_snapshot),
                    None,
                );
            }
            if !expected_present
                && verify_snapshot
                    .annotations
                    .iter()
                    .any(|native| native.page == page && native.native_id == native_id)
            {
                return uncertain(
                    "rotated authority still carries the annotation Delete was supposed to remove."
                        .to_string(),
                    false,
                    false,
                    Some(verify_snapshot),
                    None,
                );
            }
            PdfMutationOutcome {
                path,
                custody: PdfMutationCustody::Applied,
                snapshot: Some(verify_snapshot),
                native_id,
                page,
                refreshed,
            }
        }
        Err(replace_err) => match replace_err.custody() {
            ReplaceCustody::Unchanged => {
                // Nothing published: a between-open substitution or generation exhaustion.
                // Neither may project sidecar or history.
                refused(
                    PdfMutationRefusal::WriteFailed {
                        detail: replace_err.to_string(),
                    },
                    Some(admission_snapshot),
                )
            }
            ReplaceCustody::Rotated => {
                // Published, but a later step failed: verify from the retained bytes, then
                // hand the authority off so Retry re-verifies the same descriptor.
                let read = authority.read_bounded(PDF_MUTATION_MAX_BYTES);
                let (snapshot, refreshed) = match read {
                    Ok(DocumentSnapshot::Current {
                        bytes,
                        overflow: false,
                        ..
                    }) => match Document::load_mem(&bytes)
                        .map_err(|e| e.to_string())
                        .and_then(|d| snapshot_from_document(&d))
                    {
                        Ok(s) => {
                            let refreshed =
                                refreshed_from_snapshot(&s, page, &native_id, &request.op);
                            (Some(s), refreshed)
                        }
                        Err(_) => (None, None),
                    },
                    _ => (None, None),
                };
                let adopted =
                    refreshed.is_some() || matches!(&request.op, PdfMutationOp::Delete { .. });
                hand_off(&lost_authority_slot, authority);
                PdfMutationOutcome {
                    path,
                    custody: PdfMutationCustody::Uncertain {
                        detail: replace_err.to_string(),
                        adopted_via_reopen: adopted,
                        // Rotated is not lost custody, but the reducer claims the handed-off
                        // authority for Retry through the same route.
                        lost_custody: true,
                    },
                    snapshot,
                    native_id,
                    page,
                    refreshed,
                }
            }
            ReplaceCustody::Lost => {
                let detail = replace_err.to_string();
                hand_off(&lost_authority_slot, authority);
                uncertain(detail, false, true, None, None)
            }
            _ => refused(
                PdfMutationRefusal::WriteFailed {
                    detail: replace_err.to_string(),
                },
                Some(admission_snapshot),
            ),
        },
    }
}

fn refreshed_from_snapshot(
    snapshot: &PdfAnnotationSnapshot,
    page: usize,
    native_id: &str,
    op: &PdfMutationOp,
) -> Option<PdfMutationRefreshed> {
    let native = snapshot
        .annotations
        .iter()
        .find(|native| native.page == page && native.native_id == native_id)?;
    if matches!(op, PdfMutationOp::Delete { .. }) {
        return None;
    }
    let page_fingerprint = snapshot
        .pages
        .get(page)
        .map(|identity| identity.fingerprint.clone())
        .unwrap_or_default();
    Some(PdfMutationRefreshed {
        document_fingerprint: snapshot.document_fingerprint.clone(),
        page_fingerprint,
        native_fingerprint: native.fingerprint.clone(),
        native_contents: native.contents.clone(),
        native_color: native.color.clone(),
        native_rect: native.rect,
        native_has_quad_points: native.has_quad_points,
    })
}
