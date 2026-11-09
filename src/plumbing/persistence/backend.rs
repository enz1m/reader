//! Raw JSON backend and publication.

#[cfg(test)]
use super::reading::run_after_retained_read_hook;
use super::reading::{RawJsonRead, read_raw_json};
use super::records::{ANNOTATIONS_RAW_BYTE_LIMIT, LIBRARY_RAW_BYTE_LIMIT};
use super::*;

enum PublicationAuthority {
    Unbound,
    Bound(enzui_iced_core::state_authority::StateAuthority),
    Disabled,
}

struct DescriptorPublication {
    root: enzui_iced_core::secure_dir::SecureRoot,
    name: OsString,
    identity: Option<(u64, u64)>,
}

#[derive(Clone)]
pub(crate) struct RawJsonBackend {
    path: PathBuf,
    expected_source_path: Option<String>,
    max_bytes: u64,
    raw_byte_limit: RawByteLimit,
    authority: Arc<Mutex<PublicationAuthority>>,
    retained_read: Arc<Mutex<Option<Vec<u8>>>>,
    descriptor_publication: Option<Arc<Mutex<DescriptorPublication>>>,
}

impl RawJsonBackend {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            expected_source_path: None,
            max_bytes: bounded_json::MAX_LIBRARY_JSON_BYTES,
            raw_byte_limit: LIBRARY_RAW_BYTE_LIMIT,
            authority: Arc::new(Mutex::new(PublicationAuthority::Unbound)),
            retained_read: Arc::new(Mutex::new(None)),
            descriptor_publication: None,
        }
    }

    pub(crate) fn for_annotations(path: PathBuf, source_path: &str) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "annotation destination has no parent".to_string())?;
        let name = path
            .file_name()
            .ok_or_else(|| "annotation destination has no file name".to_string())?
            .to_os_string();
        let root = enzui_iced_core::secure_dir::SecureRoot::open(parent)
            .map_err(|error| error.to_string())?;
        let identity = match root.entry_identity(&name) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        Ok(Self {
            path,
            expected_source_path: Some(source_path.to_string()),
            max_bytes: bounded_json::MAX_ANNOTATION_JSON_BYTES,
            raw_byte_limit: ANNOTATIONS_RAW_BYTE_LIMIT,
            authority: Arc::new(Mutex::new(PublicationAuthority::Unbound)),
            retained_read: Arc::new(Mutex::new(None)),
            descriptor_publication: Some(Arc::new(Mutex::new(DescriptorPublication {
                root,
                name,
                identity,
            }))),
        })
    }

    fn read_retained(&self) -> Result<RawJsonRead, std::io::Error> {
        let mut authority = self
            .authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(&*authority, PublicationAuthority::Unbound) {
            let opened =
                enzui_iced_core::state_authority::StateAuthority::open_or_create_tightening(
                    &self.path,
                )?;
            *authority = PublicationAuthority::Bound(opened);
        }
        let PublicationAuthority::Bound(opened) = &*authority else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "state publication authority is disabled",
            ));
        };
        let read = read_raw_json(&self.path, self.max_bytes);
        let retained = opened.identity();
        let matches = match &read {
            RawJsonRead::Missing => retained.is_none(),
            RawJsonRead::Body(_, identity) | RawJsonRead::Refused(_, _, Some(identity)) => {
                identity.matches(retained)
            }
            RawJsonRead::Refused(_, _, None) => false,
        };
        if !matches {
            *authority = PublicationAuthority::Disabled;
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "state identity changed while the retained file was read",
            ));
        }
        Ok(read)
    }

    pub(crate) fn take_retained_read(&self) -> Option<Vec<u8>> {
        self.retained_read
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn disable_authority(&self) {
        *self
            .authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = PublicationAuthority::Disabled;
    }

    fn publish_descriptor_relative(&self, body: &str) -> Result<(), PersistError> {
        let Some(publication) = &self.descriptor_publication else {
            return Err(PersistError::new("descriptor publication is unavailable"));
        };
        let mut publication = publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let observed = match publication.root.entry_identity(&publication.name) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(PersistError::new(error.to_string())),
        };
        if observed != publication.identity {
            self.disable_authority();
            return Err(PersistError::new(
                "annotation destination no longer matches retained custody",
            ));
        }
        let mut stage =
            enzui_iced_core::tempstage::RootTempStage::create(&publication.root, &publication.name)
                .map_err(|error| PersistError::new(error.to_string()))?;
        stage
            .write_all(body.as_bytes())
            .map_err(|error| PersistError::new(error.to_string()))?;
        let identity = stage.identity();
        stage
            .commit()
            .map_err(|error| PersistError::new(error.to_string()))?;
        publication.identity = Some(identity);
        publication
            .root
            .ordinary_directory()
            .and_then(|directory| directory.sync_all())
            .map_err(|error| PersistError::new(error.to_string()))
    }
}

impl PersistBackend for RawJsonBackend {
    fn load_raw(&self, _format_id: &str, _version: u32) -> BackendLoad {
        *self
            .retained_read
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let read = match self.read_retained() {
            Ok(read) => read,
            Err(error) => {
                self.disable_authority();
                return BackendLoad::Refused {
                    reason: StateRefusal::OpenRefused,
                    diagnostic: error.to_string(),
                };
            }
        };
        #[cfg(test)]
        run_after_retained_read_hook();
        match read {
            RawJsonRead::Missing => BackendLoad::Absent,
            RawJsonRead::Body(bytes, _) => {
                *self
                    .retained_read
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(bytes.clone());
                match String::from_utf8(bytes) {
                    Ok(body) => BackendLoad::Loaded(body),
                    Err(_) => BackendLoad::Loaded("invalid UTF-8".into()),
                }
            }
            RawJsonRead::Refused(reason, diagnostic, _) => {
                BackendLoad::Refused { reason, diagnostic }
            }
        }
    }

    fn save_raw(&self, _format_id: &str, _version: u32, body: &str) -> Result<(), PersistError> {
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > self.max_bytes {
            return Err(PersistError::new("serialized state exceeds the byte limit"));
        }
        if self.expected_source_path.as_ref().is_some_and(|expected| {
            bounded_json::annotation_source(body.as_bytes()).is_some_and(|stored| {
                !stored.is_empty() && canonical_key(&stored) != canonical_key(expected)
            })
        }) {
            return Err(PersistError::new(
                "annotation sidecar destination does not match its document identity",
            ));
        }
        if self.descriptor_publication.is_some() {
            return self.publish_descriptor_relative(body);
        }
        let mut authority = self
            .authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let PublicationAuthority::Bound(opened) = &mut *authority else {
            return Err(PersistError::new(
                "state publication authority is unavailable",
            ));
        };
        match opened.publish(body.as_bytes()) {
            Ok(_) => Ok(()),
            Err(error) => {
                use enzui_iced_core::state_authority::PublishCustody;
                use enzui_iced_core::user_file::SaveOperation;
                let disable = error.custody() == PublishCustody::Lost
                    || (error.custody() == PublishCustody::Unchanged
                        && (matches!(
                            error.operation(),
                            SaveOperation::OpenParent
                                | SaveOperation::InspectDestination
                                | SaveOperation::Revalidate
                        ) || (error.operation() == SaveOperation::Publish
                            && error.io_error().kind() == std::io::ErrorKind::AlreadyExists)));
                let diagnostic = error.to_string();
                if disable {
                    *authority = PublicationAuthority::Disabled;
                }
                Err(PersistError::new(diagnostic))
            }
        }
    }

    fn raw_byte_limit(&self) -> RawByteLimit {
        self.raw_byte_limit
    }
}
