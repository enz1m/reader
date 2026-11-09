//! Recovery authority and blocking loads.

use super::*;

/// Logical persistence destination whose write authority is tracked independently.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StateOrigin {
    Library,
    Annotations { source_path: String },
}

/// Failure class that determines whether replacement can ever be authorized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateFailureKind {
    Read,
    Parse,
    FutureVersion,
    SplitAuthority,
}

/// Result of preserving source bytes without replacing their original path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuarantineStatus {
    Preserved { path: PathBuf, durable: bool },
    OriginalRetained,
    Failed,
}

/// Recovery decision and write-protection state for one persisted destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryAuthority {
    pub origin: StateOrigin,
    pub source_path: PathBuf,
    pub failure: StateFailureKind,
    pub quarantine: QuarantineStatus,
}

impl RecoveryAuthority {
    pub fn recovery_path(&self) -> Option<&Path> {
        match &self.quarantine {
            QuarantineStatus::Preserved { path, .. } => Some(path),
            QuarantineStatus::OriginalRetained | QuarantineStatus::Failed => None,
        }
    }

    pub fn can_start_empty(&self) -> bool {
        self.start_empty_refusal().is_none()
    }

    /// Returns the reason replacement cannot be authorized.
    pub fn start_empty_refusal(&self) -> Option<&'static str> {
        match self.failure {
            StateFailureKind::Read | StateFailureKind::Parse => None,
            StateFailureKind::FutureVersion => {
                Some("State from a newer Reader version remains read-only.")
            }
            StateFailureKind::SplitAuthority => Some(
                "Annotations for this document exist in two locations and must be resolved first.",
            ),
        }
    }

    pub fn title(&self) -> &'static str {
        match &self.origin {
            StateOrigin::Library => "Library state needs recovery",
            StateOrigin::Annotations { .. } => "Document annotations need recovery",
        }
    }

    pub fn message(&self) -> String {
        let failure = match self.failure {
            StateFailureKind::Read => "Reader could not safely read this state.",
            StateFailureKind::Parse => "Reader could not parse this state.",
            StateFailureKind::FutureVersion => {
                return "This state was written by a newer Reader version and remains read-only."
                    .into();
            }
            StateFailureKind::SplitAuthority => {
                "Reader found annotations for this document both next to the document and in its private data directory."
            }
        };
        let protection = match &self.quarantine {
            QuarantineStatus::Preserved { durable: true, .. } => {
                "The original was not replaced, and its bytes were preserved in Recovery."
            }
            QuarantineStatus::Preserved { durable: false, .. } => {
                "Recovery bytes were copied, but directory durability is uncertain. The original remains protected."
            }
            QuarantineStatus::OriginalRetained => {
                "The original remains in place and writes are disabled."
            }
            QuarantineStatus::Failed => {
                "Recovery copying failed. The original remains in place and writes are disabled."
            }
        };
        format!("{failure} {protection}")
    }
}

/// Typed load result retaining absence, valid data, and protected recovery.
#[derive(Clone, Debug)]
pub enum AuthorityLoad<T> {
    Missing(T),
    Loaded(T),
    Unavailable(T),
    Recovery {
        fallback: T,
        authority: RecoveryAuthority,
    },
}

impl<T> AuthorityLoad<T> {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }

    pub fn into_parts(self) -> (T, Option<RecoveryAuthority>) {
        match self {
            Self::Missing(value) | Self::Loaded(value) | Self::Unavailable(value) => (value, None),
            Self::Recovery {
                fallback,
                authority,
            } => (fallback, Some(authority)),
        }
    }
}

/// Reload payload carrying the value and exact store that performed the read.
#[derive(Clone)]
pub enum RecoveryReload {
    Library {
        store: AppStateStore,
        outcome: AuthorityLoad<AppState>,
    },
    Annotations {
        source_path: String,
        source_key: String,
        destination: Option<PathBuf>,
        store: AnnotationStore,
        outcome: AuthorityLoad<AnnotationsFile>,
    },
}

impl std::fmt::Debug for RecoveryReload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Library { outcome, .. } => formatter
                .debug_struct("Library")
                .field("outcome", outcome)
                .finish(),
            Self::Annotations {
                source_path,
                outcome,
                ..
            } => formatter
                .debug_struct("Annotations")
                .field("source_path", source_path)
                .field("outcome", outcome)
                .finish(),
        }
    }
}

/// Load settlement preserving distinct Busy, Cancelled, Shutdown and Panicked worker outcomes.
#[derive(Clone, Debug)]
pub enum BlockingLoadSettlement<T = RecoveryReload> {
    Applied(T),
    WorkerFailed(BlockingTaskError),
    DiscardedStale,
}

// A busy begin preserves the incumbent generation; invalidation discards its result without
// abandoning settlement custody.
#[derive(Clone, Default)]
pub(crate) struct BlockingLoadController {
    inner: Arc<Mutex<BlockingLoadState>>,
}

#[derive(Default)]
struct BlockingLoadState {
    /// Generation of the load that currently owns the single-flight slot.
    active: Option<u64>,
    /// Any generation `<= stale_before` must settle as `DiscardedStale`.
    stale_before: u64,
    /// Monotonic counter that mints per-submission generations.
    next_generation: u64,
}

impl BlockingLoadController {
    // Busy leaves the incumbent intact; settlement retires only its own generation and
    // invalidation yields DiscardedStale.
    pub(crate) fn begin<T: Send + 'static>(
        &self,
        dispatcher: &BlockingTaskDispatcher,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<Task<BlockingLoadSettlement<T>>, BlockingTaskError> {
        let generation = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.active.is_some() {
                // Typed Busy that never entered the queue must not clear
                // the incumbent's active custody; the state is untouched.
                return Err(BlockingTaskError::Busy);
            }
            state.next_generation = state.next_generation.wrapping_add(1);
            let generation = state.next_generation;
            state.active = Some(generation);
            generation
        };

        let controller = Arc::clone(&self.inner);
        let task = dispatcher.submit(generation, move |_context| job());
        Ok(Task::future(async move {
            let settlement = task.await;
            let stale = {
                let mut state = controller
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let matches_active = state.active == Some(generation);
                if matches_active {
                    // Clear only this generation; a later begin may have reassigned `active`.
                    state.active = None;
                }
                !matches_active || generation <= state.stale_before
            };
            if stale {
                return BlockingLoadSettlement::DiscardedStale;
            }
            match settlement.into_result() {
                Ok(value) => BlockingLoadSettlement::Applied(value),
                Err(terminal) => BlockingLoadSettlement::WorkerFailed(terminal),
            }
        }))
    }

    // Mark every currently-live generation stale so its settlement is
    // dropped as [`BlockingLoadSettlement::DiscardedStale`]. The running
    // closure is not interrupted, so `is_loading()` stays true until its
    // dispatcher submission exits.
    pub(crate) fn invalidate(&self) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.stale_before = state.next_generation;
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .is_some()
    }
}
