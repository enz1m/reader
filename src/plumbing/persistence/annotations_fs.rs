//! Annotation authorities and path resolution.

use super::backend::RawJsonBackend;
use super::records::{ANNOTATIONS_FORMAT_ID, ANNOTATIONS_RAW_BYTE_LIMIT};
use super::state::resolved_state_path;
use super::*;

/// Environment variable that overrides where annotation sidecars are stored.
///
/// When set, every document's sidecar goes into this directory, hashed by source path,
/// instead of beside the source. Unset in production; tests point it at a temp
/// directory so committed fixtures never get sidecars written beside them.
pub const ANNOTATIONS_DIR_ENV: &str = "READER_APP_ANNOTATIONS_DIR";

thread_local! {
    // Thread-local annotations root, taking precedence over [`ANNOTATIONS_DIR_ENV`]. The
    // env var is shared by parallel test threads, while this root isolates each test.
    // Set it with [`set_thread_annotations_root`].
    static THREAD_ANNOTATIONS_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Point this thread's annotation persistence at `dir` (typically a fresh
/// `TempDir` per test). Overrides both [`ANNOTATIONS_DIR_ENV`] and the
/// next-to-source / data-dir resolution for the calling thread only, so
/// parallel tests are isolated from one another. Returns the previous value so
/// a caller can restore it.
pub fn set_thread_annotations_root(dir: Option<PathBuf>) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = dir.as_ref() {
        TEST_ANNOTATION_ROOTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(dir.clone());
    }
    THREAD_ANNOTATIONS_ROOT.with(|slot| slot.replace(dir))
}

// Carry thread-local test annotation roots explicitly to workers; registered roots bypass
// production override redirection.
#[cfg(test)]
static TEST_ANNOTATION_ROOTS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn is_test_annotation_root(dir: &Path) -> bool {
    TEST_ANNOTATION_ROOTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|root| root == dir)
}

/// The current thread-local annotations root, if any. Public so tests can
/// assert *where* a write landed (the location, not just the contents).
pub fn thread_annotations_root() -> Option<PathBuf> {
    THREAD_ANNOTATIONS_ROOT.with(|slot| slot.borrow().clone())
}

// Canonical-path hashing selects the fallback filename; source binding still rejects collisions.
pub(crate) fn annotations_hashed_name(source_path: &str) -> String {
    let key = canonical_key(source_path);
    let mut h: u64 = 1469598103934665603; // FNV offset
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("annotations-{:016x}.json", h)
}

// The walk follows the selected path rather than a symlink target and does not
// require the selected leaf to survive until authority resolution.
fn worktree_walk_start(start: &Path) -> Option<PathBuf> {
    let absolute = match std::env::current_dir().ok() {
        Some(cwd) if !start.is_absolute() => cwd.join(start),
        _ => start.to_path_buf(),
    };
    if absolute.is_dir() {
        return Some(absolute);
    }
    match absolute.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        _ => None,
    }
}

/// Returns whether the selected path has a Git-worktree ancestor.
pub fn is_inside_git_worktree(start: &Path) -> bool {
    let Some(mut dir) = worktree_walk_start(start) else {
        return false;
    };
    loop {
        match fs::symlink_metadata(dir.join(".git")) {
            Ok(_) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true,
        }
        if !dir.pop() {
            return false;
        }
    }
}

// Redirected roots are verified before any sidecar pathname is derived.
pub(crate) fn ensure_redirect_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("redirected annotation root is not absolute".into());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "redirected annotation root has no valid leaf name".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "redirected annotation root has no parent".to_string())?;
    let app = AppId::new(name).map_err(|error| error.to_string())?;
    AppDataResolver::new(Some(parent.to_path_buf()), None::<PathBuf>)
        .ensure_tightening(&app)
        .map(|directory| directory.path().to_path_buf())
        .map_err(|error| error.to_string())
}

/// The retained read and write location for one document's annotations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnotationsAuthority {
    pub destination: PathBuf,
    /// A second durable record that must remain untouched until resolved.
    pub conflicting_record: Option<PathBuf>,
}

pub(crate) type WritabilityProbeObserver = Arc<Mutex<Box<dyn FnMut(&Path) + Send>>>;

// Authority requires identity-checked cleanup of the exact staged inode.
fn sidecar_directory_accepts_new_entry(
    sidecar: &Path,
    observer: Option<&WritabilityProbeObserver>,
) -> bool {
    let Ok(mut stage) = enzui_iced_core::tempstage::TempStage::create(sidecar) else {
        return false;
    };
    let staged = stage.path().to_path_buf();
    if let Some(observe) = observer {
        let mut observe = observe
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        observe(&staged);
    }
    enzui_iced_core::tempstage::remove_if_path_names_file(&staged, stage.file_mut()).is_ok()
}

pub(crate) fn durable_record_at(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WritabilityProbe {
    Allowed,
    Suppressed,
}

fn fallback_annotations_path(
    source_path: &str,
    data_resolver: &AppDataResolver,
) -> Result<PathBuf, String> {
    let app = AppId::new("reader-app").map_err(|error| error.to_string())?;
    data_resolver
        .ensure_tightening(&app)
        .map_err(|error| error.to_string())?
        .join(annotations_hashed_name(source_path))
        .map_err(|error| error.to_string())
}

pub fn resolve_annotations_authority(
    source_path: &str,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> Result<AnnotationsAuthority, String> {
    annotations_authority(
        source_path,
        data_resolver,
        annotations_root,
        WritabilityProbe::Allowed,
        None,
    )
}

pub(crate) fn annotations_record_authority(
    source_path: &str,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> Result<AnnotationsAuthority, String> {
    annotations_authority(
        source_path,
        data_resolver,
        annotations_root,
        WritabilityProbe::Suppressed,
        None,
    )
}

pub(crate) fn annotations_authority(
    source_path: &str,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
    probe: WritabilityProbe,
    observer: Option<&WritabilityProbeObserver>,
) -> Result<AnnotationsAuthority, String> {
    if let Some(dir) = annotations_root
        && !dir.as_os_str().is_empty()
    {
        #[cfg(test)]
        if is_test_annotation_root(dir) {
            return Ok(AnnotationsAuthority {
                destination: dir.join(annotations_hashed_name(source_path)),
                conflicting_record: None,
            });
        }
        return ensure_redirect_root(dir).map(|dir| AnnotationsAuthority {
            destination: dir.join(annotations_hashed_name(source_path)),
            conflicting_record: None,
        });
    }
    #[cfg(test)]
    if let Some(dir) = thread_annotations_root()
        && !dir.as_os_str().is_empty()
    {
        return Ok(AnnotationsAuthority {
            destination: dir.join(annotations_hashed_name(source_path)),
            conflicting_record: None,
        });
    }

    let source = Path::new(source_path);
    let sidecar = source.with_extension(format!(
        "{}.annotations.json",
        source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("bin")
    ));
    let inside_worktree = is_inside_git_worktree(source);
    let sidecar_record = durable_record_at(&sidecar);
    let fallback = fallback_annotations_path(source_path, data_resolver);

    // A checkout is a no-write branch: neither a probe nor a sidecar creation
    // may precede selection of the private fallback authority.
    if inside_worktree {
        let fallback = fallback?;
        return Ok(AnnotationsAuthority {
            conflicting_record: sidecar_record.then_some(sidecar),
            destination: fallback,
        });
    }

    let fallback_record = fallback
        .as_ref()
        .ok()
        .filter(|path| durable_record_at(path))
        .cloned();
    match (sidecar_record, fallback_record) {
        (true, Some(fallback)) => Ok(AnnotationsAuthority {
            destination: fallback,
            conflicting_record: Some(sidecar),
        }),
        (true, None) => Ok(AnnotationsAuthority {
            destination: sidecar,
            conflicting_record: None,
        }),
        (false, Some(fallback)) => Ok(AnnotationsAuthority {
            destination: fallback,
            conflicting_record: None,
        }),
        (false, None) => {
            let destination = if probe == WritabilityProbe::Allowed
                && sidecar_directory_accepts_new_entry(&sidecar, observer)
            {
                sidecar
            } else {
                fallback?
            };
            Ok(AnnotationsAuthority {
                destination,
                conflicting_record: None,
            })
        }
    }
}

pub(crate) fn annotations_path_with_authorities(
    source_path: &str,
    data_resolver: &AppDataResolver,
    annotations_root: Option<&Path>,
) -> Result<PathBuf, String> {
    resolve_annotations_authority(source_path, data_resolver, annotations_root)
        .map(|authority| authority.destination)
}

pub(crate) fn compatibility_data_resolver(resolver: &XdgResolver) -> AppDataResolver {
    let base = resolved_state_path(resolver)
        .and_then(|path| path.parent()?.parent().map(Path::to_path_buf));
    AppDataResolver::new(base, None::<PathBuf>)
}

pub fn annotations_path_with_roots(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> PathBuf {
    annotations_path_with_authorities(
        source_path,
        &compatibility_data_resolver(resolver),
        annotations_root,
    )
    .expect("Reader annotations require a private data or document root")
}

/// Resolve an annotation sidecar with explicit XDG roots and no process-env
/// lookup. Tests use this entry point.
pub fn annotations_path_with_resolver(source_path: &str, resolver: &XdgResolver) -> PathBuf {
    annotations_path_with_roots(source_path, resolver, None)
}

/// Resolve an annotation sidecar using the current production environment.
pub fn annotations_path(source_path: &str) -> PathBuf {
    let annotations_root = std::env::var_os(ANNOTATIONS_DIR_ENV).map(PathBuf::from);
    annotations_path_with_roots(
        source_path,
        &XdgResolver::from_env(),
        annotations_root.as_deref(),
    )
}

pub(crate) fn try_annotation_store_at(
    destination: PathBuf,
    source_path: &str,
) -> Result<AnnotationStore, String> {
    Ok(Arc::new(JsonStore::new(
        RawJsonBackend::for_annotations(destination, source_path)?,
        ANNOTATIONS_FORMAT_ID,
    )))
}

pub fn annotation_store_at(destination: PathBuf, source_path: &str) -> AnnotationStore {
    try_annotation_store_at(destination, source_path)
        .unwrap_or_else(|_| unavailable_annotation_store())
}

pub(crate) fn unavailable_annotation_store() -> AnnotationStore {
    Arc::new(JsonStore::unavailable(
        ANNOTATIONS_FORMAT_ID,
        ANNOTATIONS_RAW_BYTE_LIMIT,
    ))
}

pub fn annotation_store_with_roots(
    source_path: &str,
    resolver: &XdgResolver,
    annotations_root: Option<&Path>,
) -> AnnotationStore {
    let data_resolver = compatibility_data_resolver(resolver);
    annotations_path_with_authorities(source_path, &data_resolver, annotations_root)
        .map(|path| annotation_store_at(path, source_path))
        .unwrap_or_else(|_| unavailable_annotation_store())
}

pub struct OpenedAnnotations {
    pub authority: Option<AnnotationsAuthority>,
    pub store: AnnotationStore,
    pub load: AuthorityLoad<AnnotationsFile>,
}
