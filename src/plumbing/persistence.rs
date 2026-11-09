//! Atomic reading-progress and document-bound annotation persistence.
//!
//! Library state uses XDG state storage. Sidecars use the source parent or a private
//! fallback, preserving recovery custody until explicitly resolved. PDF positions
//! use unrotated page coordinates; EPUB positions use plain-text character offsets.

use enzui_iced_core::app_data::{AppDataResolver, AppId};
use enzui_iced_core::atomic_state::StateRefusal;
use enzui_iced_core::blocking_task::{BlockingTaskDispatcher, BlockingTaskError};
use enzui_iced_core::persist::{
    BackendLoad, PersistBackend, PersistError, Persistable, PersistenceKind, RawByteLimit, Store,
    XdgResolver,
};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

mod bounded_json;

mod annotation_ops;
mod annotations_fs;
mod annotations_io;
mod backend;
mod citations;
mod progress;
mod queue;
mod reading;
mod records;
mod recovery;
mod state;
mod state_support;

#[cfg(test)]
thread_local! {
    // Serialisations performed on this exact thread. Thread-locality keeps
    // parallel tests isolated and lets a test distinguish reducer-thread work
    // from work performed only when its returned task is polled.
    static SERIALIZATION_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn record_serialization() {
    #[cfg(test)]
    SERIALIZATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
thread_local! {
    static DESERIALIZATION_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn record_deserialization() {
    #[cfg(test)]
    DESERIALIZATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
pub(crate) fn serialization_count_on_current_thread() -> u64 {
    SERIALIZATION_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn deserialization_count_on_current_thread() -> u64 {
    DESERIALIZATION_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod atomic_publication_tests;

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod bounded_restore_tests;

#[cfg(test)]
mod writability_probe_tests;

pub use annotation_ops::{
    is_bookmarked, remove_annotation, remove_bookmark, restore_annotation, toggle_bookmark,
    update_annotation_color, update_annotation_note, update_annotation_style,
    update_annotation_text,
};
pub(crate) use annotations_fs::WritabilityProbeObserver;
pub use annotations_fs::{
    ANNOTATIONS_DIR_ENV, AnnotationsAuthority, OpenedAnnotations, annotation_store_at,
    annotation_store_with_roots, annotations_path, annotations_path_with_resolver,
    annotations_path_with_roots, is_inside_git_worktree, resolve_annotations_authority,
    set_thread_annotations_root, thread_annotations_root,
};
pub use annotations_io::{
    add_annotation, add_annotation_full, add_annotation_with_position,
    annotation_bookmark_count_with_roots, load_annotations, load_annotations_authority_with_roots,
    load_annotations_with_resolver, load_annotations_with_roots, open_annotations_authority,
    reload_recovery, save_annotations, save_annotations_with_resolver, save_annotations_with_roots,
};
pub(crate) use annotations_io::{
    annotation_bookmark_count_with_data, recovery_worker_failure_with_data,
    reload_recovery_with_data, reload_recovery_with_observer,
};
#[cfg(test)]
pub(crate) use annotations_io::{
    open_annotations_authority_with_observer, serialize_annotations_pretty,
    write_serialized_annotations_with_roots,
};
pub use citations::{
    CitationFidelity, accumulate_reading_time, annotation_citation, annotations_to_json,
    annotations_to_markdown, current_timestamp, now_unix_secs, render_annotation,
};
pub use progress::{
    RecentBook, canonical_key, get_progress, recent_books, remove_recent, set_progress,
};
pub use queue::PersistenceOutcome;
pub(crate) use queue::{
    LIBRARY_STATE_DESTINATION, PersistenceQueue, PersistenceTicket, annotations_destination_for,
};
pub use records::{
    Annotation, AnnotationPosition, AnnotationsFile, BookmarkRecord, PdfAnnotationBinding,
};
pub(crate) use records::{JSON_STATE_VERSION, default_version};
pub(crate) use recovery::BlockingLoadController;
pub use recovery::{
    AuthorityLoad, BlockingLoadSettlement, QuarantineStatus, RecoveryAuthority, RecoveryReload,
    StateFailureKind, StateOrigin,
};
pub use state::{
    AnnotationStore, AppState, AppStateStore, EpubProgressAnchor, JsonStore, ProgressRecord,
    app_state_store_with_resolver, empty_annotations, empty_app_state, load_state,
    load_state_authority, load_state_with_resolver, save_state, save_state_with_resolver,
    state_path, state_path_with_resolver,
};
#[cfg(test)]
pub(crate) use state::{serialize_state_pretty, write_serialized_state_with_resolver};
pub(crate) use state::{state_location_available, unavailable_app_state_store};
#[cfg(test)]
pub use state_support::fixtures_dir;
pub(crate) use state_support::load_startup_library;
