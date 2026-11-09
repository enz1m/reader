//! App support types: custody, picks, intents, guards.

use super::*;

// Probe invoker liveness before restoring focus; hidden invokers are a safe no-op.
pub(crate) fn restore_focus_task(id: iced::advanced::widget::Id) -> Task<Msg> {
    match restore_focus_operation_for::<Msg>(&id) {
        Some(op) => iced::advanced::widget::operate(op),
        None => Task::none(),
    }
}

// The `RestoreFocus` operation for a dismissal restoring focus to `id`, exposed so a
// test can run it over the rendered `UserInterface` and read the landed id back.
pub(crate) fn restore_focus_operation_for<T>(
    id: &iced::advanced::widget::Id,
) -> Option<Box<dyn iced::advanced::widget::Operation<T>>>
where
    T: Send + 'static,
{
    use enzui_iced_core::modal::DismissCause;
    use enzui_iced_widgets::modal_host::{ModalHostEvent, restore_focus_operation};
    restore_focus_operation::<T>(ModalHostEvent::Dismissed(DismissCause::Escape), Some(id))
}

// Transient keyboard-opened overlays have no durable invoker; restore only a live toolbar
// target.
pub(crate) fn overlay_restore_id(
    overlay: Option<model::Overlay>,
) -> Option<iced::advanced::widget::Id> {
    match overlay {
        Some(model::Overlay::PageArranger) => Some(crate::view::widgets::page_arranger_button_id()),
        // Their actions live in the titlebar's overflow menu, so focus returns
        // to its trigger.
        Some(model::Overlay::Metadata | model::Overlay::Properties) => {
            Some(crate::view::widgets::toolbar_overflow_trigger_id())
        }
        _ => None,
    }
}

// Edits retain the destination selected when the document opened, so later
// filesystem changes cannot move annotations within the session.
#[derive(Clone)]
pub(crate) struct AnnotationCustody {
    pub(crate) source_path: PathBuf,
    pub(crate) source_key: String,
    pub(crate) destination: PathBuf,
    pub(crate) store: persistence::AnnotationStore,
}

impl AnnotationCustody {
    pub(crate) fn holds(&self, source_path: &Path) -> bool {
        self.source_path.as_path() == source_path
    }

    // Recovery origins can use an alias that resolved to the selected document.
    pub(crate) fn holds_origin(&self, source_path: &str) -> bool {
        self.source_path.to_string_lossy() == source_path
            || self.source_key == persistence::canonical_key(source_path)
    }

    pub(crate) fn queue_destination(&self) -> String {
        persistence::annotations_destination_for(&self.destination)
    }
}

pub(crate) enum PendingLoad {
    Annotations(PathBuf),
    Recovery(persistence::StateOrigin),
}

pub(crate) struct PendingDocumentLoad {
    pub(crate) attempt: u64,
    pub(crate) path: PathBuf,
    pub(crate) format: Format,
}

#[derive(Clone)]
pub(crate) enum PersistenceSnapshot {
    Library {
        state: std::sync::Arc<persistence::AppState>,
        store: persistence::AppStateStore,
    },
    Annotations {
        source_path: String,
        file: std::sync::Arc<persistence::AnnotationsFile>,
        store: persistence::AnnotationStore,
    },
}

// Retain failed snapshots for retry under a fresh generation; superseded completions cannot
// retire current debt.
#[derive(Clone)]
pub(crate) struct PendingPersistence {
    pub(crate) ticket: persistence::PersistenceTicket,
    pub(crate) snapshot: PersistenceSnapshot,
    pub(crate) status: DebtStatus,
}

#[derive(Clone, Debug)]
pub(crate) enum DebtStatus {
    /// A task has been dispatched for [`PendingPersistence::ticket`] and no
    /// completion has arrived yet.
    InFlight,
    /// The write failed. `summary` is the bounded user-facing string from
    /// [`enzui_iced_core::persist::PersistError::user_summary`]; `diagnostic` is the raw
    /// error, only for structured logs. The recording generation lives on
    /// [`PendingPersistence::ticket`], so a stale completion cannot clear it.
    Failed {
        summary: String,
        /// Raw diagnostic retained for structured logs and inspection by
        /// [`App::persistence_failure_diagnostics`] (test-only). Deliberately
        /// never rendered in the banner; the bounded `summary` is.
        #[allow(dead_code)]
        diagnostic: String,
    },
}

// An in-flight parent export keyed to one picker request.
pub(crate) struct PendingPick {
    pub(crate) generation: u64,
    pub(crate) cancel: enzui_iced_core::portal::cancel::PortalCancel,
}

pub(crate) fn export_result_to_outcome(
    result: Result<
        Result<enzui_iced_wayland::ParentExport, enzui_iced_wayland::ParentExportError>,
        enzui_iced_core::blocking_task::BlockingTaskError,
    >,
) -> crate::message::PickOutcome {
    use crate::message::PickOutcome;
    use enzui_iced_parent_lease::Handoff;
    use enzui_iced_wayland::{ExportError, ParentExportError};
    match result {
        Ok(Ok(export)) => PickOutcome::Parented(Handoff::new(export)),
        Ok(Err(ParentExportError::Export(
            ExportError::Unsupported
            | ExportError::Disconnected
            | ExportError::TimedOut
            | ExportError::Protocol(_),
        ))) => PickOutcome::Unparented,
        Ok(Err(ParentExportError::Export(ExportError::WindowGone | ExportError::Cancelled(_))))
        | Ok(Err(ParentExportError::WindowChanged { .. })) => PickOutcome::Aborted,
        // A Busy/Cancelled/Shutdown/Panicked terminal from the shared
        // dispatcher aborts this pick rather than misclassifying it as one of
        // the compositor's export-side refusals above.
        Err(_terminal) => PickOutcome::Aborted,
    }
}

// Coalesce concurrent scan requests; generation rejects stale batches and revision drift
// restarts traversal.
#[derive(Default)]
pub(crate) struct AvailabilityScan {
    pub(crate) generation: u64,
    pub(crate) active: bool,
    pub(crate) pending: bool,
    pub(crate) revision: u64,
    pub(crate) cancel: Option<cancellation::CancelToken>,
}

// The `stat()` seam behind the availability scan. `Send + Sync` because the
// scan runs on its own thread; injectable so a test can block or count probes.
pub(crate) trait MetadataBackend: Send + Sync {
    fn exists(&self, path: &str) -> bool;
}

// One real `stat()` per query, counted per-thread for the reducer-stays-flat
// gates.
#[derive(Debug, Default)]
pub(crate) struct FsMetadataBackend;

impl MetadataBackend for FsMetadataBackend {
    fn exists(&self, path: &str) -> bool {
        probe_exists(path)
    }
}

/// How often the pending-write tick fires while something is dirty.
///
/// Short enough that an interrupted session loses little, long enough that holding A+
/// coalesces a burst into one write. The timer exists only while a write is pending.
pub const PERSIST_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Close-drain deadline: if the blocking owner, durable snapshots and dirty flags have
/// not all settled within it, close is vetoed with a visible summary, so a wedged worker
/// cannot swallow shutdown.
pub const CLOSE_DRAIN_BUDGET: std::time::Duration = std::time::Duration::from_secs(4);

/// Close-drain wake cadence. Only subscribed while a close is draining; a
/// completed close and an idle reader hold no timer.
pub const CLOSE_DRAIN_TICK: std::time::Duration = std::time::Duration::from_millis(25);

/// Initial-view preview fan-out cap: at most this many concurrent preview renders, the
/// rest waiting in [`App::preview_pending`]. Well below
/// [`enzui_iced_core::blocking_task::DEFAULT_QUEUE_CAPACITY`], so a large library cannot
/// Busy-refuse a thumbnail.
pub const PREVIEW_INFLIGHT_CAP: usize = 4;

/// Coalesced busy-retry cadence, subscribed only while [`App::busy_retry_pending`] is set.
/// Short so retained intent is released within a visible frame even when no Reader
/// completion arrives.
pub const BUSY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Close lifecycle for one captured window target.
#[derive(Clone, Debug)]
pub enum CloseState {
    Idle,
    /// The drain is committed; late callbacks cannot reopen admission.
    Committed {
        /// Resolved close target, or `None` when no live window remained.
        window: Option<iced::window::Id>,
    },
    /// Waiting for worker occupancy and durable state to settle before the deadline.
    Draining {
        /// Requested window, or `None` during teardown without a live target.
        window: Option<iced::window::Id>,
        /// Monotonic instant after which the drain vetoes with a summary.
        deadline: std::time::Instant,
    },
    /// An expired drain kept the window open; an explicit retry reuses its target.
    Vetoed {
        window: Option<iced::window::Id>,
        summary: String,
    },
}

impl CloseState {
    pub(crate) fn pauses_expendable_work(&self) -> bool {
        matches!(self, Self::Draining { .. } | Self::Committed { .. })
    }
}

// Seed the reading palette from the synchronous shell theme; later OS updates must not
// override a user-selected palette.
pub(crate) fn reading_theme_for_mode(mode: EnzThemeMode) -> ReadingTheme {
    match mode {
        EnzThemeMode::Dark => ReadingTheme::Dark,
        EnzThemeMode::Light => ReadingTheme::Light,
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.recovery_location_opener.cancel();
        // Signal a running scan to stop after its current probe. Atomic, no
        // join: a wedged `stat()` never blocks shutdown.
        if let Some(token) = self.availability.cancel.take() {
            token.cancel(cancellation::CancelCause::Shutdown);
        }
    }
}

// The single accepted native-PDF annotation mutation on the bounded owner. A terminal
// matches only when its [`op_token`] and `document_generation` agree with this record,
// so an old duplicate cannot clear a newer mutation.
#[derive(Clone, Debug)]
pub(crate) struct PdfMutationInFlight {
    pub op_token: u64,
    pub document_generation: u64,
    pub intent: PdfMutationIntent,
}

// Admission result distinguishing dispatched, retained-but-queued and refused intents; only
// dispatch releases caller drafts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionResult {
    Admitted,
    Queued,
    Refused,
}

// Result of one [`App::edit_pdf_annotation_comment`] call. Named so the
// reducer can distinguish `NonNative` (fall through to the sidecar
// path) from `NoChange` (composer may close) from a native admission
// whose [`AdmissionResult`] governs whether the composer may close.
pub(crate) enum EditCommentOutcome {
    /// Native mutation dispatched (or coalesced onto the retained slot).
    Native {
        task: Task<Msg>,
        result: AdmissionResult,
    },
    /// No change was needed (text identical or annotation missing).
    /// Composer may close.
    NoChange,
    /// No PDF binding — caller should fall through to the sidecar-only
    /// edit path via `CommitEditAnnotation`.
    NonNative,
    /// Admission failed with a user-visible refusal already recorded.
    /// Composer stays open.
    Failed,
}

// Three-valued decision from [`App::apply_native_history`] that tells
// the reducer whether it may run the sidecar undo/redo now, must wait
// for the mutation terminal, or must leave the stack unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryDecision {
    /// Non-PDF or sidecar-only action; caller runs
    /// [`UndoStack::undo`] / `redo` immediately.
    NonNative,
    /// Native mutation admitted; the terminal in
    /// [`App::commit_history_replay`] runs the stack once identity is
    /// re-verified.
    NativeAdmitted,
    /// Admission refused; the stack stays as-is.
    Refused,
}

/// Admitted native mutation with stable identity; history replay commits only after its
/// terminal revalidates the top action.
#[derive(Clone, Debug)]
pub enum PdfMutationIntent {
    Create(PdfCreateIntent),
    EditContents(PdfEditContentsIntent),
    UpdateColor(PdfUpdateColorIntent),
    Remove(PdfRemoveIntent),
    HistoryReplay {
        path: std::path::PathBuf,
        expected_document_fingerprint: String,
        op: plumbing::pdf_ops::PdfMutationOp,
        sidecar_id: u64,
        action: Box<crate::undo::UndoableAction>,
        forward: bool,
        /// Native subtype (`Highlight` / `Text`) captured at admission
        /// from the target annotation, so a retry recomputes the
        /// post-native fingerprint under the same subtype.
        expected_subtype: &'static str,
        /// Absolute PDF user-space rectangle captured at admission.
        /// `None` when the operation is Delete (no rect needed).
        expected_rect: Option<plumbing::pdf_ops::PdfRect>,
    },
}

/// Native-PDF annotation `Create` fields captured by reducer admission.
#[derive(Clone, Debug)]
pub struct PdfCreateIntent {
    pub path: std::path::PathBuf,
    pub expected_document_fingerprint: String,
    pub expected_page_fingerprint: String,
    pub selection: model::PdfSelection,
    pub position: persistence::AnnotationPosition,
    pub kind: plumbing::pdf_ops::PdfAnnotationKind,
    pub color: String,
    pub authored_contents: String,
    pub native_id: String,
}

/// Native-PDF annotation `Edit note / edit contents` fields captured by
/// reducer admission.
#[derive(Clone, Debug)]
pub struct PdfEditContentsIntent {
    pub path: std::path::PathBuf,
    pub expected_document_fingerprint: String,
    pub expected_page_fingerprint: String,
    pub expected_native_fingerprint: String,
    pub page: usize,
    pub native_id: String,
    pub color: String,
    pub sidecar_id: u64,
    pub new_contents: String,
    /// Absolute PDF user-space rectangle of the target at admission, so a retry without a
    /// refreshed fingerprint can still compute the expected post-mutation fingerprint.
    pub expected_rect: Option<plumbing::pdf_ops::PdfRect>,
    /// Native subtype (`Highlight` or `Text`) of the target annotation,
    /// so the fingerprint computed at retry uses the same subtype the
    /// worker used at admission.
    pub expected_subtype: &'static str,
}

/// Native-PDF annotation `Cycle color` fields captured by reducer
/// admission.
#[derive(Clone, Debug)]
pub struct PdfUpdateColorIntent {
    pub path: std::path::PathBuf,
    pub expected_document_fingerprint: String,
    pub expected_page_fingerprint: String,
    pub expected_native_fingerprint: String,
    pub page: usize,
    pub native_id: String,
    pub sidecar_id: u64,
    pub contents: String,
    pub new_color: String,
    /// See [`PdfEditContentsIntent::expected_rect`].
    pub expected_rect: Option<plumbing::pdf_ops::PdfRect>,
    /// See [`PdfEditContentsIntent::expected_subtype`].
    pub expected_subtype: &'static str,
}

/// Native-PDF annotation `Delete` fields captured by reducer admission.
#[derive(Clone, Debug)]
pub struct PdfRemoveIntent {
    pub path: std::path::PathBuf,
    pub expected_document_fingerprint: String,
    pub expected_page_fingerprint: String,
    pub expected_native_fingerprint: String,
    pub page: usize,
    pub native_id: String,
    pub sidecar_id: u64,
}

// A queued PDF mutation intent tagged with the exact document
// generation and path identity admission captured. Release only
// dispatches when both still match the currently open document.
#[derive(Clone, Debug)]
pub(crate) struct PdfMutationPending {
    pub document_generation: u64,
    pub intent: PdfMutationIntent,
}

// Retain uncertainty across document switches; same-path identity drift cannot settle it, and
// stale retry terminals cannot clear newer debt.
#[derive(Clone, Debug)]
pub(crate) struct UncertainPublication {
    pub op_token: u64,
    pub page: usize,
    pub native_id: String,
    /// Document fingerprint the worker saw at admission (or, for a
    /// pre-dispatch retry, the last known good load). Snapshot ==
    /// this on next retained open → `NotApplied`.
    pub expected_pre_document_fingerprint: String,
    /// Document fingerprint of the worker's post-write reopen, when the terminal carried
    /// one. Applied requires the next retained snapshot to match it and the intent
    /// predicate to hold, so a substituted PDF satisfying the predicate cannot pass;
    /// without it only `NotApplied` or `StillUncertain` are possible.
    pub expected_post_document_fingerprint: Option<String>,
    /// The consequence the mutation was trying to make true on disk.
    /// Applied classification requires this predicate to hold on the
    /// retained snapshot.
    pub intended: PdfReconciliationIntent,
    /// True while the banner is user-visible. Dismiss flips this to
    /// false; the record itself persists so admits and close-drain
    /// keep gating.
    pub banner_visible: bool,
    pub summary: &'static str,
    pub retry_sequence: u64,
    pub retry_active: bool,
    /// Retained inode custody for reconciliation; Retry refuses without it and restores it
    /// before releasing its running flag.
    pub lost_authority: Option<
        std::sync::Arc<std::sync::Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>>,
    >,
}

// The disk consequence an uncertain publication was trying to achieve, checked against
// the retained snapshot after document-fingerprint drift.
#[derive(Clone, Debug)]
pub(crate) enum PdfReconciliationIntent {
    /// Create / UpdateContents / UpdateColor: the native id must be present. Updates
    /// expect `Some(post_mutation_fingerprint)`; a create has no prior native peer, so
    /// `None`.
    Present {
        expected_native_fingerprint: Option<String>,
    },
    /// Delete: the native id must be absent from the retained
    /// snapshot.
    Absent,
}

/// The classification of one reconciliation record against a fresh
/// retained snapshot on the same exact pathname and current document
/// generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfReconciliationClass {
    /// The retained snapshot matches [`UncertainPublication::intended`]: project the
    /// sidecar if needed and clear the record.
    Applied,
    /// The retained snapshot still matches
    /// [`UncertainPublication::expected_pre_document_fingerprint`]: roll back any
    /// optimistic projection and clear the record.
    NotApplied,
    /// Neither pre nor intended: another valid PDF at the path or an out-of-band change.
    /// Only Retry or Dismiss resolves it.
    StillUncertain,
}

// Occupancy guard shared by the PDF-writer and Retry dispatches. Built before
// [`enzui_iced_core::blocking_task::BlockingTaskOwner::submit`], so a refused submission
// drops it and clears the flag. Inside the closure it is the first local, so it drops
// last: the authority restore or `hand_off` write happens before the flag reads `false`.
pub(crate) struct BlockingOccupancyGuard(pub(crate) std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for BlockingOccupancyGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.0.store(false, Ordering::Release);
    }
}
