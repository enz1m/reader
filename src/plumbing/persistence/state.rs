//! App state stores, loads, saves, and quarantine.

use super::annotations_fs::{annotations_hashed_name, ensure_redirect_root};
use super::annotations_io::initialise_annotation_fallback;
use super::backend::RawJsonBackend;
use super::records::{LIBRARY_FORMAT_ID, LIBRARY_RAW_BYTE_LIMIT};
#[cfg(test)]
use super::state_support::publish_raw;
use super::*;

/// EPUB position within a chapter's rendered paragraph structure.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EpubProgressAnchor {
    /// Paragraph index produced from the chapter's newline-delimited text.
    #[serde(default)]
    pub paragraph: usize,
    /// Unicode-scalar offset within that paragraph.
    #[serde(default)]
    pub char_offset: usize,
}

/// Persisted reading-progress record.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProgressRecord {
    /// Last opened page (PDF/CBZ) or chapter (EPUB), 0-indexed.
    pub page: usize,
    /// Total pages/chapters known at save time (for percentage display).
    #[serde(default)]
    pub total: usize,
    /// EPUB-only: preferred font-size in px.
    #[serde(default)]
    pub epub_font_size: Option<i32>,
    /// EPUB-only: line spacing multiplier (1.0–2.2).
    /// Sidecars without this field deserialise as `None` (default 1.5).
    #[serde(default)]
    pub epub_line_spacing: Option<f32>,
    /// EPUB-only: the older two-way theme preference, read only when `epub_theme` is
    /// `None`.
    #[serde(default)]
    pub epub_dark_theme: Option<bool>,
    /// EPUB-only: 3-way theme, one of "light" | "sepia" | "dark".
    /// Takes precedence over `epub_dark_theme` when both are present.
    #[serde(default)]
    pub epub_theme: Option<String>,
    /// ISO-8601 timestamp of last open.
    #[serde(default)]
    pub last_opened: String,
    #[serde(default)]
    pub title: String,
    /// Author from the source's metadata. Empty when unknown.
    #[serde(default)]
    pub author: String,
    /// "pdf" | "epub" | "cbz". Empty for records written before v0.7.
    #[serde(default)]
    pub format: String,
    /// EPUB view mode: "scrolled" | "paginated". Defaults to "scrolled".
    #[serde(default)]
    pub epub_view_mode: Option<String>,
    /// EPUB-only paragraph and character anchor. Older records resume at the
    /// start of their saved chapter.
    #[serde(default)]
    pub epub_anchor: Option<EpubProgressAnchor>,
    /// Accumulated reading time in seconds (all formats).
    #[serde(default)]
    pub reading_time_secs: Option<u64>,
}

/// Persisted app state — a map keyed by canonicalised file path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub files: HashMap<String, ProgressRecord>,
}

impl Persistable for AppState {
    fn persist_serialize(&self) -> Result<String, String> {
        record_serialization();
        bounded_json::validate_library(self)?;
        serde_json::to_string_pretty(self).map_err(|error| error.to_string())
    }

    fn persist_deserialize(body: &str) -> Result<Self, String> {
        record_deserialization();
        bounded_json::decode_library(body.as_bytes()).map_err(|failure| match failure.kind {
            bounded_json::DecodeFailureKind::FutureVersion => {
                format!("future-version: {}", failure.diagnostic)
            }
            bounded_json::DecodeFailureKind::Parse | bounded_json::DecodeFailureKind::Limit => {
                failure.diagnostic
            }
        })
    }

    fn persist_default() -> Self {
        Self {
            version: default_version(),
            files: HashMap::new(),
        }
    }
}

impl Persistable for AnnotationsFile {
    fn persist_serialize(&self) -> Result<String, String> {
        record_serialization();
        bounded_json::validate_annotations(self)?;
        serde_json::to_string_pretty(self).map_err(|error| error.to_string())
    }

    fn persist_deserialize(body: &str) -> Result<Self, String> {
        record_deserialization();
        bounded_json::decode_annotations(body.as_bytes()).map_err(|failure| match failure.kind {
            bounded_json::DecodeFailureKind::FutureVersion => {
                format!("future-version: {}", failure.diagnostic)
            }
            bounded_json::DecodeFailureKind::Parse | bounded_json::DecodeFailureKind::Limit => {
                failure.diagnostic
            }
        })
    }

    fn persist_default() -> Self {
        Self {
            version: default_version(),
            source_path: String::new(),
            annotations: Vec::new(),
            next_id: 1,
            bookmarks: Vec::new(),
        }
    }
}

// Location of the state file under the resolver's XDG state root.
//
// Tests pass an [`XdgResolver`] built from literal temp roots. Production
// snapshots the process environment once in `App::new` and reuses it.
pub(crate) fn resolved_state_path(resolver: &XdgResolver) -> Option<PathBuf> {
    resolver
        .resolve(PersistenceKind::State, "reader-app", "state.json")
        .expect("the static reader state XDG descendants are valid")
}

pub(crate) fn state_location_available(resolver: &XdgResolver) -> bool {
    resolved_state_path(resolver).is_some()
}

pub fn state_path_with_resolver(resolver: &XdgResolver) -> PathBuf {
    resolved_state_path(resolver).expect("Reader state requires an absolute XDG state or home root")
}

/// Location of the production state file.
///
/// Prefer [`state_path_with_resolver`] outside production boot; this wrapper
/// exists for compatibility with callers that do not own an app context.
pub fn state_path() -> PathBuf {
    state_path_with_resolver(&XdgResolver::from_env())
}

struct UnavailableBackend {
    raw_byte_limit: RawByteLimit,
}

impl PersistBackend for UnavailableBackend {
    fn load_raw(&self, _format_id: &str, _version: u32) -> BackendLoad {
        BackendLoad::Refused {
            reason: StateRefusal::OpenRefused,
            diagnostic: "private persistence root unavailable".into(),
        }
    }

    fn save_raw(&self, _format_id: &str, _version: u32, _body: &str) -> Result<(), PersistError> {
        Err(PersistError::new("private persistence root unavailable"))
    }

    fn raw_byte_limit(&self) -> RawByteLimit {
        self.raw_byte_limit
    }
}

pub struct JsonStore<T: Persistable> {
    inner: Store<T>,
    backend: Option<RawJsonBackend>,
    replacement_blocked: AtomicBool,
}

impl<T: Persistable> JsonStore<T> {
    pub(crate) fn new(backend: RawJsonBackend, format_id: &str) -> Self {
        Self {
            inner: Store::new(backend.clone(), format_id, JSON_STATE_VERSION),
            backend: Some(backend),
            replacement_blocked: AtomicBool::new(false),
        }
    }

    pub(crate) fn unavailable(format_id: &str, raw_byte_limit: RawByteLimit) -> Self {
        Self {
            inner: Store::new(
                UnavailableBackend { raw_byte_limit },
                format_id,
                JSON_STATE_VERSION,
            ),
            backend: None,
            replacement_blocked: AtomicBool::new(true),
        }
    }

    pub(crate) fn load(&self) -> enzui_iced_core::persist::LoadOutcome<T> {
        self.inner.load()
    }

    pub(crate) fn take_retained_read(&self) -> Option<Vec<u8>> {
        self.backend
            .as_ref()
            .and_then(RawJsonBackend::take_retained_read)
    }

    pub(crate) fn block_replacement(&self) {
        self.replacement_blocked.store(true, Ordering::Release);
    }

    pub fn authorize_replacement(&self) {
        self.replacement_blocked.store(false, Ordering::Release);
        self.inner.authorize_replacement();
    }

    pub fn is_write_blocked(&self) -> bool {
        self.replacement_blocked.load(Ordering::Acquire) || self.inner.is_write_blocked()
    }

    pub fn save_blocking(&self, value: &T) -> Result<(), PersistError> {
        if self.replacement_blocked.load(Ordering::Acquire) {
            return Err(PersistError::new("state replacement is not authorized"));
        }
        self.inner.save_blocking(value)
    }
}

pub type AppStateStore = Arc<JsonStore<AppState>>;
pub type AnnotationStore = Arc<JsonStore<AnnotationsFile>>;

pub fn empty_app_state() -> AppState {
    <AppState as Persistable>::persist_default()
}

pub fn empty_annotations(source_path: &str) -> AnnotationsFile {
    let mut file = <AnnotationsFile as Persistable>::persist_default();
    initialise_annotation_fallback(&mut file, source_path);
    file
}

fn ensure_state_path(resolver: &XdgResolver) -> Result<PathBuf, String> {
    let path = resolved_state_path(resolver)
        .ok_or_else(|| "private Reader state root unavailable".to_string())?;
    let app_dir = path
        .parent()
        .ok_or_else(|| "private Reader state root unavailable".to_string())?;
    let base = app_dir
        .parent()
        .ok_or_else(|| "private Reader state root unavailable".to_string())?;
    let app = AppId::new("reader-app").map_err(|error| error.to_string())?;
    let ensured = AppDataResolver::new(Some(base.to_path_buf()), None::<PathBuf>)
        .ensure_tightening(&app)
        .map_err(|error| error.to_string())?;
    if ensured.path() != app_dir {
        return Err("private Reader state root unavailable".into());
    }
    Ok(path)
}

pub(crate) fn unavailable_app_state_store() -> AppStateStore {
    Arc::new(JsonStore::unavailable(
        LIBRARY_FORMAT_ID,
        LIBRARY_RAW_BYTE_LIMIT,
    ))
}

fn app_state_store_for_path(path: PathBuf) -> AppStateStore {
    Arc::new(JsonStore::new(RawJsonBackend::new(path), LIBRARY_FORMAT_ID))
}

pub fn app_state_store_with_resolver(resolver: &XdgResolver) -> AppStateStore {
    ensure_state_path(resolver)
        .map(app_state_store_for_path)
        .unwrap_or_else(|_| unavailable_app_state_store())
}

pub fn load_state_authority(resolver: &XdgResolver) -> (AppStateStore, AuthorityLoad<AppState>) {
    let path = match ensure_state_path(resolver) {
        Ok(path) => path,
        Err(_) => {
            return (
                unavailable_app_state_store(),
                AuthorityLoad::Unavailable(empty_app_state()),
            );
        }
    };
    let store = app_state_store_for_path(path.clone());
    let loaded = store.load();
    let retained_read = store.take_retained_read();
    let outcome = authority_outcome(
        loaded,
        StateOrigin::Library,
        path,
        resolver,
        retained_read.as_deref(),
    );
    (store, outcome)
}

/// Compatibility loader returning a fallback without exposing recovery authority.
/// Load failures may still preserve a recovery copy; production startup uses
/// [`load_state_authority`] so replacement remains an explicit decision.
pub fn load_state_with_resolver(resolver: &XdgResolver) -> AppState {
    load_state_authority(resolver).1.into_parts().0
}

/// Load the library state using the current production environment.
pub fn load_state() -> AppState {
    load_state_with_resolver(&XdgResolver::from_env())
}

pub(crate) fn authority_outcome<T: Persistable>(
    outcome: enzui_iced_core::persist::LoadOutcome<T>,
    origin: StateOrigin,
    source_path: PathBuf,
    resolver: &XdgResolver,
    retained_read: Option<&[u8]>,
) -> AuthorityLoad<T> {
    match outcome {
        enzui_iced_core::persist::LoadOutcome::Loaded(value) => AuthorityLoad::Loaded(value),
        enzui_iced_core::persist::LoadOutcome::Absent { default } => {
            AuthorityLoad::Missing(default)
        }
        enzui_iced_core::persist::LoadOutcome::DeserializeFailed {
            fallback,
            diagnostic,
        } => {
            let future = diagnostic.starts_with("future-version:");
            let failure = if future {
                StateFailureKind::FutureVersion
            } else {
                StateFailureKind::Parse
            };
            let quarantine = if future {
                QuarantineStatus::OriginalRetained
            } else {
                quarantine_retained_read(retained_read, resolver, &origin)
            };
            AuthorityLoad::Recovery {
                fallback,
                authority: RecoveryAuthority {
                    origin,
                    source_path,
                    failure,
                    quarantine,
                },
            }
        }
        enzui_iced_core::persist::LoadOutcome::Refused {
            fallback, reason, ..
        } => {
            let failure = if reason == StateRefusal::UnexpectedHeader {
                StateFailureKind::FutureVersion
            } else {
                StateFailureKind::Read
            };
            let quarantine = if failure == StateFailureKind::FutureVersion {
                QuarantineStatus::OriginalRetained
            } else {
                quarantine_retained_read(retained_read, resolver, &origin)
            };
            AuthorityLoad::Recovery {
                fallback,
                authority: RecoveryAuthority {
                    origin,
                    source_path,
                    failure,
                    quarantine,
                },
            }
        }
        other => {
            let fallback = other.into_value();
            let quarantine = quarantine_retained_read(retained_read, resolver, &origin);
            AuthorityLoad::Recovery {
                fallback,
                authority: RecoveryAuthority {
                    origin,
                    source_path,
                    failure: StateFailureKind::Read,
                    quarantine,
                },
            }
        }
    }
}

pub(crate) fn quarantine_retained_read(
    bytes: Option<&[u8]>,
    resolver: &XdgResolver,
    origin: &StateOrigin,
) -> QuarantineStatus {
    let Some(bytes) = bytes else {
        return QuarantineStatus::OriginalRetained;
    };
    match quarantine_bytes(bytes, resolver, origin) {
        Ok((path, durable)) => QuarantineStatus::Preserved { path, durable },
        Err(_) => QuarantineStatus::Failed,
    }
}

fn quarantine_bytes(
    bytes: &[u8],
    resolver: &XdgResolver,
    origin: &StateOrigin,
) -> Result<(PathBuf, bool), String> {
    let state_path = ensure_state_path(resolver)?;
    let parent = state_path
        .parent()
        .ok_or_else(|| "Reader state has no private parent directory".to_string())?;
    let recovery_dir = ensure_redirect_root(&parent.join("recovery"))?;

    let stem = match origin {
        StateOrigin::Library => "library".to_string(),
        StateOrigin::Annotations { source_path } => {
            annotations_hashed_name(source_path).replace(".json", "")
        }
    };
    let hash = bytes.iter().fold(1469598103934665603u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    });
    for collision in 0..1_024u32 {
        let suffix = if collision == 0 {
            String::new()
        } else {
            format!("-{collision}")
        };
        let path = recovery_dir.join(format!("{stem}-{hash:016x}{suffix}.corrupt"));
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if existing_recovery_matches(&path, &metadata, bytes) {
                let file_synced = enzui_iced_core::atomic_state::open_state_file_nofollow(&path)
                    .and_then(|file| file.sync_all())
                    .is_ok();
                let parents_synced = sync_recovery_parents(parent, &recovery_dir);
                return Ok((path, file_synced && parents_synced));
            }
            continue;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        file.write_all(bytes).map_err(|error| error.to_string())?;
        let file_synced = file.sync_all().is_ok();
        let parents_synced = sync_recovery_parents(parent, &recovery_dir);
        return Ok((path, file_synced && parents_synced));
    }
    Err("No collision-free recovery name was available".into())
}

fn existing_recovery_matches(path: &Path, metadata: &fs::Metadata, expected: &[u8]) -> bool {
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != u64::try_from(expected.len()).unwrap_or(u64::MAX)
    {
        return false;
    }
    let Ok(file) = enzui_iced_core::atomic_state::open_state_file_nofollow(path) else {
        return false;
    };
    let mut existing = Vec::with_capacity(expected.len());
    file.take(
        u64::try_from(expected.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    )
    .read_to_end(&mut existing)
    .is_ok()
        && existing == expected
}

fn sync_recovery_parents(state_dir: &Path, recovery_dir: &Path) -> bool {
    #[cfg(test)]
    if quarantine_fault::take_parent_sync_failure() {
        return false;
    }
    let recovery_synced = fs::File::open(recovery_dir)
        .and_then(|directory| directory.sync_all())
        .is_ok();
    let state_synced = fs::File::open(state_dir)
        .and_then(|directory| directory.sync_all())
        .is_ok();
    let state_parent_synced = state_dir.parent().is_none_or(|parent| {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .is_ok()
    });
    recovery_synced && state_synced && state_parent_synced
}

#[cfg(test)]
pub(crate) fn serialize_state_pretty(state: &AppState) -> Result<String, String> {
    record_serialization();
    bounded_json::validate_library(state)?;
    let body = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Cannot serialise state: {error}"))?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX) > bounded_json::MAX_LIBRARY_JSON_BYTES {
        return Err("serialized state exceeds the byte limit".into());
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) fn write_serialized_state_with_resolver(
    serialized: &str,
    resolver: &XdgResolver,
) -> Result<(), String> {
    if u64::try_from(serialized.len()).unwrap_or(u64::MAX) > bounded_json::MAX_LIBRARY_JSON_BYTES {
        return Err("serialized state exceeds the byte limit".into());
    }
    let path = ensure_state_path(resolver)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create state dir: {e}"))?;
    }
    publish_raw(&path, serialized.as_bytes()).map_err(|e| format!("Cannot write state: {e}"))
}

/// Persist the library state using an explicit XDG resolver.
pub fn save_state_with_resolver(state: &AppState, resolver: &XdgResolver) -> Result<(), String> {
    let (store, outcome) = load_state_authority(resolver);
    if !matches!(
        outcome,
        AuthorityLoad::Missing(_) | AuthorityLoad::Loaded(_)
    ) {
        return Err("Reader state replacement is not authorized".into());
    }
    store
        .save_blocking(state)
        .map_err(|error| format!("Cannot write state: {error}"))
}

/// Persist the library state using the current production environment.
pub fn save_state(state: &AppState) -> Result<(), String> {
    save_state_with_resolver(state, &XdgResolver::from_env())
}

#[cfg(test)]
pub(crate) mod quarantine_fault {
    thread_local! {
        static FAIL_PARENT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub struct ParentSyncFailure;

    pub fn arm_parent_sync_failure() -> ParentSyncFailure {
        FAIL_PARENT_SYNC.with(|slot| slot.set(true));
        ParentSyncFailure
    }

    pub(super) fn take_parent_sync_failure() -> bool {
        FAIL_PARENT_SYNC.with(|slot| slot.replace(false))
    }

    impl Drop for ParentSyncFailure {
        fn drop(&mut self) {
            FAIL_PARENT_SYNC.with(|slot| slot.set(false));
        }
    }
}
