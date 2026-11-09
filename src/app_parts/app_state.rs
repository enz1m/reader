//! Reader application state.

use super::*;

pub struct App {
    pub(crate) model: Model,
    /// The shared-XDG theme host for chrome and shell-following reading palettes.
    pub themes: ThemeIntegration,
    pub(crate) window_width: f32,
    // The last known window height: paginated EPUB cuts pages to the viewport, so the
    // reducer needs it to clamp `next`/`previous`.
    pub(crate) window_height: f32,
    // When `false`, the reducer never writes to disk. [`App::test_new`] clears it so
    // parallel tests cannot touch the developer's real state; `App::new` sets it.
    pub(crate) persist_enabled: bool,
    // XDG roots captured once at production boot or supplied literally by a
    // persistence test. Keeping this as data avoids process-global env races.
    pub(crate) xdg_resolver: XdgResolver,
    pub(crate) app_data_resolver: AppDataResolver,
    // Snapshot the configured annotation root so later environment changes cannot redirect writes.
    pub(crate) annotations_root: Option<PathBuf>,
    pub(crate) annotation_probe_observer: Option<persistence::WritabilityProbeObserver>,
    pub(crate) library_store: persistence::AppStateStore,
    pub(crate) annotation_store: Option<AnnotationCustody>,
    // Keep blocking work on the bounded owner; async wrapping alone does not move it off-thread.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held for its Drop, which drains the pool")
    )]
    pub(crate) blocking_tasks: enzui_iced_core::blocking_task::BlockingTaskOwner,
    // Cloned once at boot so `&self` helpers can dispatch without borrowing
    // `self.blocking_tasks` again.
    pub(crate) blocking_dispatcher: enzui_iced_core::blocking_task::BlockingTaskDispatcher,
    // Monotonic per-submission generation for diagnostic snapshot accuracy.
    pub(crate) blocking_generation: std::sync::atomic::AtomicU64,
    // The process-wide enzui-center session, when opted in and enrolled.
    #[cfg(feature = "center")]
    pub(crate) center: Option<enzui_iced_agent::center::CenterHost<crate::center::CenterCall>>,
    // Async close FSM: it drains through `Msg::PersistenceFinished` and
    // `Msg::CloseDrainTick`, committing close only with an idle pool, no durable debt and
    // clean dirty flags.
    pub(crate) close_state: CloseState,
    // Preview paths whose thumbnail render is currently on the
    // shared bounded owner (pool + queue). Guards against duplicate
    // submissions on repeated `thumbnail_tasks` calls.
    pub(crate) preview_in_flight: std::collections::HashSet<String>,
    // Preview paths waiting for a slot below [`PREVIEW_INFLIGHT_CAP`], so an oversized
    // initial fan-out loses no artwork.
    pub(crate) preview_pending: std::collections::VecDeque<(String, PreviewSource)>,
    // Shared coalesced busy-retry flag, set by any `Busy` arm that retained intent and
    // cleared when the retry pumps. It drives the [`BUSY_RETRY_INTERVAL`] wake, so a
    // resubmit never spins within one reducer turn.
    pub(crate) busy_retry_pending: bool,
    // The single accepted native-PDF annotation mutation on the shared
    // bounded owner.
    pub(crate) pdf_mutation_active: Option<PdfMutationInFlight>,
    // The busied active operation awaiting re-dispatch, set only by
    // [`Msg::PdfMutationRefusedBusy`] and drained before any newer
    // [`Self::pdf_mutation_pending`] intent.
    pub(crate) pdf_mutation_busy_retry: Option<PdfMutationPending>,
    // One newer bounded user operation queued while an active mutation
    // runs. A history intent is never queued.
    pub(crate) pdf_mutation_pending: Option<PdfMutationPending>,
    // Retained record of the last publication whose worker returned
    // [`crate::plumbing::pdf_ops::PdfMutationCustody::Uncertain`]. While `Some`, native
    // PDF mutations and pending releases refuse until Retry or Dismiss; Retry is
    // single-flight via [`UncertainPublication::retry_active`].
    pub(crate) pdf_mutation_uncertain: Option<UncertainPublication>,
    // Authority handoff slot for the current Save As arrangement publication, `Some`
    // while a worker holds a clone. [`App::finish_arrangement_publication`] drains it into
    // [`Self::arrangement_uncertain_authority`]; it is `None` between publications so a
    // stale worker cannot write into it.
    pub(crate) arrangement_publication_slot:
        Option<crate::plumbing::pdf_ops::ArrangementUncertainSlot>,
    // Retain non-cloneable publication custody until explicit acknowledgement; unresolved debt
    // vetoes close and writes.
    pub(crate) arrangement_uncertain_authority:
        Option<crate::plumbing::pdf_ops::ArrangementUncertainAuthority>,
    // Keep output custody until acknowledgement, without claiming that published files were
    // removed.
    pub(crate) extract_publication_debt: Option<crate::plumbing::pdf_ops::ExtractPublicationDebt>,
    // Monotonic op-token counter minted per admitted native PDF
    // annotation mutation. Zero is reserved; exhaustion refuses.
    pub(crate) pdf_mutation_sequence: u64,
    // Monotonic retry-generation counter minted per uncertain-retry
    // dispatch. Zero is reserved; a stale terminal (different
    // generation) cannot clear a newer debt.
    pub(crate) pdf_mutation_uncertain_retry_sequence: u64,
    // Terminals may precede writer exit; publish retained authority before clearing the flag
    // that gates admission and close.
    pub(crate) pdf_mutation_writer_running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // Claim retained writer authority only after its scope-exit guard releases.
    pub(crate) pdf_mutation_writer_authority_slot: Option<
        std::sync::Arc<std::sync::Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>>,
    >,
    // Running flag for the dispatched Retry closure, mirroring
    // [`Self::pdf_mutation_writer_running`]. Admit, close-drain and a fresh Retry refuse
    // while it is `true`; the closure restores the authority through
    // [`Self::pdf_mutation_retry_authority_slot`] before clearing it.
    pub(crate) pdf_mutation_retry_running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // Dispatch-side handle to the `Arc` shared with
    // [`UncertainPublication::lost_authority`], kept so
    // `finalize_retry_authority_handoff` can wait for the running flag before dropping it.
    // Reset per dispatch.
    pub(crate) pdf_mutation_retry_authority_slot: Option<
        std::sync::Arc<std::sync::Mutex<Option<enzui_iced_core::user_file::DocumentAuthority>>>,
    >,
    pub(crate) load_controller: persistence::BlockingLoadController,
    pub(crate) document_load_controller: persistence::BlockingLoadController,
    pub(crate) pending_document_load: Option<PendingDocumentLoad>,
    pub(crate) native_drop: Option<NativeDocumentDrop>,
    pub(crate) active_load_origin: Option<persistence::StateOrigin>,
    pub(crate) pending_load: Option<PendingLoad>,
    pub(crate) library_recovery_generation: u64,
    pub(crate) library_state_revision: u64,
    pub(crate) library_removed_during_restore: HashSet<String>,
    pub(crate) annotation_recovery_generation: u64,
    pub(crate) annotation_state_revision: u64,
    // Orders off-thread state/sidecar writes by logical destination. The
    // reducer only registers immutable snapshots; filesystem work and JSON
    // serialisation happen in the returned task.
    pub(crate) persistence_queue: persistence::PersistenceQueue,
    // Retain the newest snapshot until Saved; failed writes veto close and superseded
    // completions cannot settle newer debt.
    pub(crate) persistence_debt: HashMap<String, PendingPersistence>,
    // Flush deferred progress on the persistence tick, navigation and close.
    pub(crate) progress_dirty: bool,
    // Flush before replacing annotations so each snapshot retains its own destination.
    pub(crate) annotations_dirty: bool,
    // Create the portal runtime lazily on the first chooser request.
    pub(crate) picker: crate::picker::Picker,
    pub(crate) instance: Option<enzui_iced_widgets::single_instance::SingleInstance>,
    pub(crate) pending_launch_document: Option<PathBuf>,
    pub(crate) recovery_location_opener: crate::recovery_location::RecoveryLocationOpener,
    // Current main window used for parent liveness.
    pub(crate) main_window: Option<iced::window::Id>,
    // In-flight parent export, if any.
    pub(crate) pending_pick: Option<PendingPick>,
    // Next picker request generation.
    pub(crate) next_pick_generation: u64,
    pub(crate) save_arrangement_pick_source: Option<PathBuf>,
    // Filesystem availability probes run off the reducer, including on slow mounts.
    pub(crate) metadata: std::sync::Arc<dyn MetadataBackend>,
    // One owned availability scan at a time, with one coalesced pending
    // request. Bounds concurrent scan work regardless of navigation bursts;
    // results reconcile by `generation`.
    pub(crate) availability: AvailabilityScan,
    // The library/recents projection is stale relative to `app_state`. Set by
    // routine progress flushes (O(1)); a rebuild is deferred until the library
    // is shown, so a page turn does not re-sort the whole store.
    pub(crate) projection_dirty: bool,
}

pub(crate) type NativeDocumentDrop = (
    u64,
    PathBuf,
    enzui_iced_widgets::file_drop::FileDrop,
    Option<enzui_iced_core::drop_files::CopiedFiles>,
);
