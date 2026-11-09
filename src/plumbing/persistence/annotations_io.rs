//! Annotation open, load, and save.

use super::annotations_fs::{
    WritabilityProbe, annotations_authority, annotations_record_authority,
    compatibility_data_resolver, try_annotation_store_at, unavailable_annotation_store,
};
use super::state::{authority_outcome, quarantine_retained_read};
use super::*;
#[cfg(test)]
use super::{annotations_fs::annotations_path_with_authorities, state_support::publish_raw};

// Resolve the parent once while it is reachable. StateAuthority then retains
// that directory by descriptor even if the selected alias is renamed later.
fn retained_publication_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "annotation destination has no parent".to_string())?;
    let name = destination
        .file_name()
        .ok_or_else(|| "annotation destination has no file name".to_string())?;
    parent
        .canonicalize()
        .map(|parent| parent.join(name))
        .map_err(|error| error.to_string())
}

pub(crate) fn open_annotations_authority_with_data(
    source_path: &str,
    resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> OpenedAnnotations {
    open_annotations_authority_with_observer(
        source_path,
        resolver,
        data_resolver,
        annotations_root,
        None,
    )
}

pub(crate) fn open_annotations_authority_with_observer(
    source_path: &str,
    resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
    observer: Option<&WritabilityProbeObserver>,
) -> OpenedAnnotations {
    let authority = match annotations_authority(
        source_path,
        data_resolver,
        annotations_root,
        WritabilityProbe::Allowed,
        observer,
    ) {
        Ok(authority) => authority,
        Err(_) => {
            return OpenedAnnotations {
                authority: None,
                store: unavailable_annotation_store(),
                load: AuthorityLoad::Unavailable(empty_annotations(source_path)),
            };
        }
    };
    let publication_path = match retained_publication_path(&authority.destination) {
        Ok(path) => path,
        Err(_) => {
            return OpenedAnnotations {
                authority: None,
                store: unavailable_annotation_store(),
                load: AuthorityLoad::Unavailable(empty_annotations(source_path)),
            };
        }
    };
    let store = match try_annotation_store_at(publication_path.clone(), source_path) {
        Ok(store) => store,
        Err(_) => {
            return OpenedAnnotations {
                authority: None,
                store: unavailable_annotation_store(),
                load: AuthorityLoad::Unavailable(empty_annotations(source_path)),
            };
        }
    };
    let origin = StateOrigin::Annotations {
        source_path: source_path.to_string(),
    };
    let loaded = store.load();
    let retained_read = store.take_retained_read();
    let outcome = authority_outcome(
        loaded,
        origin.clone(),
        publication_path.clone(),
        resolver,
        retained_read.as_deref(),
    );
    let outcome = match outcome {
        AuthorityLoad::Loaded(file)
            if !file.source_path.is_empty()
                && canonical_key(&file.source_path) != canonical_key(source_path) =>
        {
            store.block_replacement();
            AuthorityLoad::Recovery {
                fallback: empty_annotations(source_path),
                authority: RecoveryAuthority {
                    origin,
                    source_path: publication_path.clone(),
                    failure: StateFailureKind::Read,
                    quarantine: quarantine_retained_read(
                        retained_read.as_deref(),
                        resolver,
                        &StateOrigin::Annotations {
                            source_path: source_path.to_string(),
                        },
                    ),
                },
            }
        }
        AuthorityLoad::Missing(mut file) => {
            initialise_annotation_fallback(&mut file, source_path);
            AuthorityLoad::Missing(file)
        }
        AuthorityLoad::Loaded(mut file) => {
            if file.source_path.is_empty() {
                file.source_path = source_path.to_string();
            }
            AuthorityLoad::Loaded(file)
        }
        AuthorityLoad::Unavailable(mut file) => {
            initialise_annotation_fallback(&mut file, source_path);
            AuthorityLoad::Unavailable(file)
        }
        AuthorityLoad::Recovery {
            mut fallback,
            authority,
        } => {
            initialise_annotation_fallback(&mut fallback, source_path);
            AuthorityLoad::Recovery {
                fallback,
                authority,
            }
        }
    };
    let load = match (&authority.conflicting_record, outcome) {
        (Some(retained), AuthorityLoad::Missing(file) | AuthorityLoad::Loaded(file)) => {
            store.block_replacement();
            AuthorityLoad::Recovery {
                fallback: file,
                authority: split_authority_custody(source_path, retained.clone()),
            }
        }
        (_, outcome) => outcome,
    };
    OpenedAnnotations {
        authority: Some(authority),
        store,
        load,
    }
}

/// Opens one annotation authority and retains its exact destination.
pub fn open_annotations_authority(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> OpenedAnnotations {
    open_annotations_authority_with_data(
        source_path,
        resolver,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )
}

// The conflicting record is retained in place; this authority only disables
// replacement until the user resolves the split.
fn split_authority_custody(source_path: &str, retained: PathBuf) -> RecoveryAuthority {
    RecoveryAuthority {
        origin: StateOrigin::Annotations {
            source_path: source_path.to_string(),
        },
        source_path: retained,
        failure: StateFailureKind::SplitAuthority,
        quarantine: QuarantineStatus::OriginalRetained,
    }
}

pub(crate) fn load_annotations_authority_with_data(
    source_path: &str,
    resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> (AnnotationStore, AuthorityLoad<AnnotationsFile>) {
    let opened = open_annotations_authority_with_data(
        source_path,
        resolver,
        data_resolver,
        annotations_root,
    );
    (opened.store, opened.load)
}

pub fn load_annotations_authority_with_roots(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> (AnnotationStore, AuthorityLoad<AnnotationsFile>) {
    load_annotations_authority_with_data(
        source_path,
        resolver,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )
}

pub(crate) fn recovery_worker_failure_with_data(
    origin: StateOrigin,
    _resolver: &XdgResolver,
    _data_resolver: &AppDataResolver,
    _annotations_root: Option<&Path>,
) -> RecoveryReload {
    match origin {
        StateOrigin::Library => RecoveryReload::Library {
            store: unavailable_app_state_store(),
            outcome: AuthorityLoad::Unavailable(empty_app_state()),
        },
        StateOrigin::Annotations { source_path } => RecoveryReload::Annotations {
            source_key: canonical_key(&source_path),
            destination: None,
            source_path: source_path.clone(),
            store: unavailable_annotation_store(),
            outcome: AuthorityLoad::Unavailable(empty_annotations(&source_path)),
        },
    }
}

pub(crate) fn reload_recovery_with_data(
    origin: StateOrigin,
    resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> RecoveryReload {
    reload_recovery_with_observer(origin, resolver, data_resolver, annotations_root, None)
}

pub(crate) fn reload_recovery_with_observer(
    origin: StateOrigin,
    resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
    observer: Option<&WritabilityProbeObserver>,
) -> RecoveryReload {
    match origin {
        StateOrigin::Library => {
            let (store, outcome) = load_state_authority(resolver);
            RecoveryReload::Library { store, outcome }
        }
        StateOrigin::Annotations { source_path } => {
            let opened = open_annotations_authority_with_observer(
                &source_path,
                resolver,
                data_resolver,
                annotations_root,
                observer,
            );
            RecoveryReload::Annotations {
                source_key: canonical_key(&source_path),
                destination: opened
                    .authority
                    .as_ref()
                    .map(|authority| authority.destination.clone()),
                source_path,
                store: opened.store,
                outcome: opened.load,
            }
        }
    }
}

pub fn reload_recovery(
    origin: StateOrigin,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> RecoveryReload {
    reload_recovery_with_data(
        origin,
        resolver,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )
}

pub(crate) fn annotation_bookmark_count_with_data(
    source_path: &str,
    _resolver: &XdgResolver,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> Option<usize> {
    let authority =
        annotations_record_authority(source_path, data_resolver, annotations_root).ok()?;
    let store = annotation_store_at(authority.destination, source_path);
    match store.load() {
        enzui_iced_core::persist::LoadOutcome::Loaded(file) => Some(file.bookmarks.len()),
        enzui_iced_core::persist::LoadOutcome::Absent { .. } => Some(0),
        enzui_iced_core::persist::LoadOutcome::Refused { .. }
        | enzui_iced_core::persist::LoadOutcome::DeserializeFailed { .. } => None,
        _ => None,
    }
}

pub fn annotation_bookmark_count_with_roots(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> Option<usize> {
    annotation_bookmark_count_with_data(
        source_path,
        resolver,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )
}

pub(crate) fn initialise_annotation_fallback(file: &mut AnnotationsFile, source_path: &str) {
    file.version = default_version();
    file.source_path = source_path.to_string();
    file.next_id = 1;
}

/// Compatibility loader returning a fallback without exposing recovery authority.
/// Load failures may still preserve a recovery copy; document startup uses
/// [`load_annotations_authority_with_roots`] to retain write protection.
pub fn load_annotations_with_roots(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> AnnotationsFile {
    load_annotations_authority_with_roots(source_path, resolver, annotations_root)
        .1
        .into_parts()
        .0
}

pub fn load_annotations_with_resolver(
    source_path: &str,
    resolver: &XdgResolver,
) -> AnnotationsFile {
    load_annotations_with_roots(source_path, resolver, None)
}

pub fn load_annotations(source_path: &str) -> AnnotationsFile {
    let annotations_root = std::env::var_os(ANNOTATIONS_DIR_ENV).map(PathBuf::from);
    load_annotations_with_roots(
        source_path,
        &XdgResolver::from_env(),
        annotations_root.as_deref(),
    )
}

#[cfg(test)]
pub(crate) fn serialize_annotations_pretty(file: &AnnotationsFile) -> Result<String, String> {
    record_serialization();
    bounded_json::validate_annotations(file)?;
    let body = serde_json::to_string_pretty(file)
        .map_err(|error| format!("Cannot serialise annotations: {error}"))?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX) > bounded_json::MAX_ANNOTATION_JSON_BYTES {
        return Err("serialized annotations exceed the byte limit".into());
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) fn write_serialized_annotations_with_roots(
    source_path: &str,
    serialized: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> Result<(), String> {
    if u64::try_from(serialized.len()).unwrap_or(u64::MAX) > bounded_json::MAX_ANNOTATION_JSON_BYTES
    {
        return Err("serialized annotations exceed the byte limit".into());
    }
    let path = annotations_path_with_authorities(
        source_path,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create annotations dir: {e}"))?;
    }
    publish_raw(&path, serialized.as_bytes()).map_err(|e| format!("Cannot write annotations: {e}"))
}

/// Persist a document's annotations using an explicit resolver.
pub fn save_annotations_with_roots(
    source_path: &str,
    file: &AnnotationsFile,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> Result<(), String> {
    let (store, outcome) =
        load_annotations_authority_with_roots(source_path, resolver, annotations_root);
    if !matches!(
        outcome,
        AuthorityLoad::Missing(_) | AuthorityLoad::Loaded(_)
    ) {
        return Err("Reader annotation replacement is not authorized".into());
    }
    store
        .save_blocking(file)
        .map_err(|error| format!("Cannot write annotations: {error}"))
}

pub fn save_annotations_with_resolver(
    source_path: &str,
    file: &AnnotationsFile,
    resolver: &XdgResolver,
) -> Result<(), String> {
    save_annotations_with_roots(source_path, file, resolver, None)
}

/// Persist a document's annotations using the current production environment.
pub fn save_annotations(source_path: &str, file: &AnnotationsFile) -> Result<(), String> {
    let annotations_root = std::env::var_os(ANNOTATIONS_DIR_ENV).map(PathBuf::from);
    save_annotations_with_roots(
        source_path,
        file,
        &XdgResolver::from_env(),
        annotations_root.as_deref(),
    )
}

pub fn add_annotation(
    file: &mut AnnotationsFile,
    page: usize,
    kind: &str,
    text: &str,
    color: &str,
) -> u64 {
    add_annotation_with_position(file, page, kind, text, color, AnnotationPosition::None)
}

/// Full form: also stamps a typed position on the new annotation.
/// Prefer this at every callsite where the position is known — plain
/// `add_annotation` remains for chapter/page-level notes without a rect.
pub fn add_annotation_with_position(
    file: &mut AnnotationsFile,
    page: usize,
    kind: &str,
    text: &str,
    color: &str,
    position: AnnotationPosition,
) -> u64 {
    add_annotation_full(file, page, kind, text, color, position, "", "")
}

/// Full form: also stamps a style + optional sticky-note body.
/// - `style` accepts `"highlight"` (filled rect) | `"underline"` |
///   `"strikethrough"` | `"squiggly"`. Empty defaults to `"highlight"` for
///   annotations of kind `"highlight"`, and to `""` for kind `"note"`.
/// - `note` is a sticky-note body attached to a highlight; empty means the
///   annotation is just a coloured highlight (or, for `kind == "note"`, the
///   note text lives in `text`).
#[allow(clippy::too_many_arguments)]
pub fn add_annotation_full(
    file: &mut AnnotationsFile,
    page: usize,
    kind: &str,
    text: &str,
    color: &str,
    position: AnnotationPosition,
    style: &str,
    note: &str,
) -> u64 {
    let id = file.next_id.max(1);
    file.next_id = id + 1;
    let style = if style.is_empty() && kind == "highlight" {
        "highlight".to_string()
    } else {
        style.to_string()
    };
    file.annotations.push(Annotation {
        id,
        page,
        kind: kind.to_string(),
        text: text.to_string(),
        color: color.to_string(),
        created: current_timestamp(),
        position: position.to_stored(),
        style,
        note: note.to_string(),
        pdf_binding: None,
        quads: Vec::new(),
    });
    id
}
