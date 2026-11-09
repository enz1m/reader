//! Persistence queue and destinations.

use super::*;

// Register destination generations before dispatch; serialize and recheck at write time so the
// latest scheduled snapshot wins.
#[derive(Clone, Default)]
pub(crate) struct PersistenceQueue {
    inner: Arc<PersistenceQueueInner>,
}

#[derive(Default)]
struct PersistenceQueueInner {
    next_generation: AtomicU64,
    latest_by_destination: Mutex<HashMap<String, u64>>,
    writer: Mutex<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct PersistenceTicket {
    destination: String,
    generation: u64,
}

impl PersistenceTicket {
    pub(crate) fn destination(&self) -> &str {
        &self.destination
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

/// Typed outcome of one persistence attempt.
///
/// * [`Saved`](Self::Saved): the destination holds this snapshot; the reducer retires the
///   matching debt entry.
/// * [`Superseded`](Self::Superseded): a newer snapshot was registered first and the write
///   was skipped; the reducer stays silent.
/// * [`Error`](Self::Error): the write failed. The raw diagnostic goes only to structured
///   logs; the banner shows the [`enzui_iced_core::persist::PersistError`] summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistenceOutcome {
    Saved,
    Superseded,
    Error(String),
}

impl PersistenceQueue {
    // Register the newest intended contents for one logical destination.
    pub(crate) fn register(&self, destination: impl Into<String>) -> PersistenceTicket {
        let destination = destination.into();
        let generation = self
            .inner
            .next_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.inner
            .latest_by_destination
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(destination.clone(), generation);
        PersistenceTicket {
            destination,
            generation,
        }
    }

    // Skip superseded destination tickets before publication; write failures retain their raw
    // diagnostic for logging.
    pub(crate) fn execute(
        &self,
        ticket: &PersistenceTicket,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> PersistenceOutcome {
        let _writer = self
            .inner
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let is_latest = self
            .inner
            .latest_by_destination
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&ticket.destination)
            .copied()
            == Some(ticket.generation);
        if !is_latest {
            return PersistenceOutcome::Superseded;
        }
        match publish() {
            Ok(()) => PersistenceOutcome::Saved,
            Err(msg) => PersistenceOutcome::Error(msg),
        }
    }
}

// Stable queue key for the one whole-library state file.
pub(crate) const LIBRARY_STATE_DESTINATION: &str = "library-state";

// Native path bytes are escaped injectively so lossy path aliases cannot share
// queue generations or persistence debt.
pub(crate) fn annotations_destination_for(destination: &Path) -> String {
    const UNRESERVED: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.";
    let mut escaped = String::new();
    for byte in destination.as_os_str().as_encoded_bytes() {
        if UNRESERVED.contains(byte) {
            escaped.push(char::from(*byte));
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("annotations:{escaped}")
}
