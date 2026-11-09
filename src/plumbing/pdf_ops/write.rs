//! PDF writes: save, metadata, and faults.

use super::*;

/// A PDF write refusal retaining whether publication occurred.
#[derive(Clone, Debug)]
pub struct PdfWriteError {
    published: bool,
    completed_outputs: usize,
    detail: String,
    already_exists: bool,
}

impl PdfWriteError {
    /// Whether this operation reached publication for at least one output.
    pub fn was_published(&self) -> bool {
        self.published || self.completed_outputs != 0
    }

    /// Number of completed files left by a partially successful split.
    pub fn completed_outputs(&self) -> usize {
        self.completed_outputs
    }

    /// True when a no-clobber publication was refused because a file already
    /// exists at the candidate path. The collision ladder in
    /// `publish_no_clobber_sibling` tests this to advance to the next
    /// numeric suffix without truncating the pre-existing sibling.
    pub fn is_already_exists(&self) -> bool {
        self.already_exists
    }

    pub(super) fn with_completed_outputs(mut self, completed_outputs: usize) -> Self {
        self.completed_outputs = completed_outputs;
        self
    }

    pub fn contains(&self, text: &str) -> bool {
        self.to_string().contains(text)
    }
}

impl From<String> for PdfWriteError {
    fn from(detail: String) -> Self {
        Self {
            published: false,
            completed_outputs: 0,
            detail,
            already_exists: false,
        }
    }
}

impl PdfWriteError {
    pub(crate) fn published(detail: String) -> Self {
        Self {
            published: true,
            completed_outputs: 0,
            detail,
            already_exists: false,
        }
    }
}

impl std::fmt::Display for PdfWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.completed_outputs != 0 {
            return write!(
                f,
                "Publication occurred for {} split file(s); current identity or durability may be uncertain: {}",
                self.completed_outputs, self.detail
            );
        }
        if self.published {
            write!(
                f,
                "PDF publication occurred, but current identity or durability could not be confirmed: {}",
                self.detail
            )
        } else {
            f.write_str(&self.detail)
        }
    }
}

impl std::error::Error for PdfWriteError {}

pub(super) fn save_document(
    document: &mut Document,
    path: &Path,
    policy: enzui_iced_core::user_file::SavePolicy,
) -> Result<(), PdfWriteError> {
    save_document_result(document, path, policy)
}

// Save `document` under `policy` at `path` and surface the typed
// [`PdfWriteError`] whose [`PdfWriteError::is_already_exists`] flag the
// no-clobber collision ladder tests to advance suffixes.
pub(super) fn save_document_result(
    document: &mut Document,
    path: &Path,
    policy: enzui_iced_core::user_file::SavePolicy,
) -> Result<(), PdfWriteError> {
    #[cfg(test)]
    if let Some(detail) = save_fault::take_failure_for_this_call() {
        return Err(PdfWriteError {
            published: false,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    enzui_iced_core::user_file::save_with(path, policy, |output| {
        let mut output = output;
        document
            .save_to(&mut output)
            .map_err(|error| std::io::Error::other(error.to_string()))
    })
    .map_err(|error| {
        let already_exists = error.io_error().kind() == std::io::ErrorKind::AlreadyExists;
        PdfWriteError {
            published: error.was_published(),
            completed_outputs: 0,
            detail: error.to_string(),
            already_exists,
        }
    })?;
    #[cfg(test)]
    if let Some(detail) = save_fault::take_published_failure() {
        return Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    Ok(())
}

/// Write `meta` into the PDF at `path` without authorizing date removal.
///
/// Ordinary fields cleared to `""` remain present as empty PDF strings. Date
/// fields retain their original object unless edited; an edited empty date is
/// refused until the caller supplies explicit removal authority.
pub fn write_metadata(path: &Path, meta: &PdfMetadata) -> Result<(), PdfWriteError> {
    write_metadata_with_date_removal(path, meta, MetadataDateRemoval::default()).map(|_| ())
}

/// Publishes metadata with exact date-removal authority and returns the reopened state.
pub fn write_metadata_with_date_removal(
    path: &Path,
    meta: &PdfMetadata,
    confirmed_removal: MetadataDateRemoval,
) -> Result<PdfMetadata, PdfWriteError> {
    validate_date_write(
        &meta.creation_date,
        MetadataField::CreationDate,
        confirmed_removal,
    )?;
    validate_date_write(
        &meta.modification_date,
        MetadataField::ModificationDate,
        confirmed_removal,
    )?;
    let SourceAuthority {
        mut authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(path)?;
    if doc.is_encrypted() {
        return Err(PdfWriteError::from(format!(
            "{} is encrypted; unlock it before editing metadata.",
            path.display()
        )));
    }
    let source_metadata = read_metadata_from(&doc);
    let id = match info_id(&doc) {
        Some(id) => id,
        None => {
            let id = doc.add_object(Object::Dictionary(lopdf::Dictionary::new()));
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
    };
    let dict = doc
        .get_dictionary_mut(id)
        .map_err(|e| format!("Malformed /Info dictionary: {e}"))?;
    for field in MetadataField::TEXT_FIELDS {
        dict.set(
            String::from_utf8_lossy(field.pdf_key()).into_owned(),
            Object::String(encode_pdf_string(field.get(meta)), StringFormat::Literal),
        );
    }
    apply_date_write(
        dict,
        MetadataField::CreationDate,
        &meta.creation_date,
        confirmed_removal,
    );
    apply_date_write(
        dict,
        MetadataField::ModificationDate,
        &meta.modification_date,
        confirmed_removal,
    );
    let serialized = serialize_document_bounded(&mut doc, "metadata write")?;
    drop(doc);
    conditional_replace_authority(&mut authority, path, &serialized)?;
    let reopened_bytes = read_retained_bytes(&authority, "metadata reopen")?;
    let reopened = Document::load_mem(&reopened_bytes).map_err(|error| PdfWriteError {
        published: true,
        completed_outputs: 0,
        detail: format!("published metadata could not be reopened: {error}"),
        already_exists: false,
    })?;
    let reopened_meta = read_metadata_from(&reopened);
    verify_metadata_write(meta, &source_metadata, &reopened_meta)?;
    Ok(reopened_meta)
}

// Serialize `document` into a bounded buffer whose size is refused above
// [`PDF_MUTATION_MAX_BYTES`]. `operation_label` is inlined into the refusal
// message so the reducer's `PdfWriteFailure::user_summary` names the flow.
pub(super) fn serialize_document_bounded(
    document: &mut Document,
    operation_label: &str,
) -> Result<Vec<u8>, PdfWriteError> {
    let mut serialized: Vec<u8> = Vec::with_capacity(PDF_MUTATION_MAX_BYTES / 8);
    document
        .save_to(&mut serialized)
        .map_err(|error| PdfWriteError::from(format!("{operation_label}: {error}")))?;
    if serialized.len() > PDF_MUTATION_MAX_BYTES {
        return Err(PdfWriteError::from(format!(
            "serialized {operation_label} ({} bytes) exceeds the {PDF_MUTATION_MAX_BYTES}-byte cap.",
            serialized.len()
        )));
    }
    Ok(serialized)
}

// Conditionally replace the retained document with `bytes`; classify the
// three custody outcomes into the module's [`PdfWriteError`] shape so a
// caller sees `was_published()` on `Rotated` / `Lost` custody, and an
// unchanged destination on `Unchanged` custody.
pub(super) fn conditional_replace_authority(
    authority: &mut DocumentAuthority,
    path: &Path,
    bytes: &[u8],
) -> Result<(), PdfWriteError> {
    #[cfg(test)]
    if let Some(detail) = save_fault::take_failure_for_this_call() {
        return Err(PdfWriteError {
            published: false,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    let result = authority.replace(bytes).map_err(|error| PdfWriteError {
        published: matches!(
            error.custody(),
            ReplaceCustody::Rotated | ReplaceCustody::Lost
        ) || error.was_published(),
        completed_outputs: 0,
        detail: format!("{}: {error}", path.display()),
        already_exists: error.io_error().kind() == std::io::ErrorKind::AlreadyExists,
    });
    #[cfg(test)]
    if let Some(detail) = save_fault::take_published_failure() {
        return Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    result.map(|_| ())
}

// Publish `bytes` at `destination` via the framework's retained
// [`DocumentAuthority::create_new`], so any pre-existing regular file at
// the pathname refuses the write with a typed
// [`CreateDocumentError::NotPublished`] carrying
// [`std::io::ErrorKind::AlreadyExists`] rather than being clobbered, and
// so the exact published inode is retained by descriptor without any
// pathname reopen. The returned authority is the same regular-file
// generation the caller reads through for its reopen verification.
//
// The three `CreateDocumentError` variants map as the acceptance rails
// require: `NotPublished` is a visible prepublication refusal; the caller
// may treat `AlreadyExists` as a collision retry signal and never as
// uncertainty. `PublishedButNotDurable` and `Lost` are surfaced through
// `custody_slot` (when the caller provides one) so the reducer can retain
// the exact descriptor without a pathname reopen after `create_new`.
pub(super) fn create_new_bytes(
    destination: &Path,
    bytes: &[u8],
    custody_slot: Option<&ArrangementUncertainSlot>,
) -> Result<DocumentAuthority, PdfWriteError> {
    #[cfg(test)]
    if let Some(detail) = save_fault::take_failure_for_this_call() {
        return Err(PdfWriteError {
            published: false,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    let authority = match DocumentAuthority::create_new(destination, bytes) {
        Ok(authority) => authority,
        Err(CreateDocumentError::NotPublished { source, .. }) => {
            let already_exists = source.kind() == std::io::ErrorKind::AlreadyExists;
            return Err(PdfWriteError {
                published: false,
                completed_outputs: 0,
                detail: format!("{}: {source}", destination.display()),
                already_exists,
            });
        }
        Err(CreateDocumentError::PublishedButNotDurable {
            authority, source, ..
        }) => {
            if let Some(slot) = custody_slot
                && let Ok(mut guard) = slot.lock()
            {
                let _ = guard.replace(ArrangementUncertainAuthority::Durable(authority));
            }
            return Err(PdfWriteError {
                published: true,
                completed_outputs: 0,
                detail: format!(
                    "{}: PublishedButNotDurable: {source}",
                    destination.display()
                ),
                already_exists: false,
            });
        }
        Err(CreateDocumentError::Lost {
            staged_handle,
            staged,
            source,
            ..
        }) => {
            let staged_desc = staged_bytes_description(&staged);
            if let Some(slot) = custody_slot
                && let Ok(mut guard) = slot.lock()
            {
                let _ = guard.replace(ArrangementUncertainAuthority::Lost {
                    staged_handle,
                    staged,
                });
            }
            return Err(PdfWriteError {
                published: true,
                completed_outputs: 0,
                detail: format!(
                    "{}: Lost custody: {source}, {staged_desc}",
                    destination.display()
                ),
                already_exists: false,
            });
        }
        _ => {
            return Err(PdfWriteError {
                published: false,
                completed_outputs: 0,
                detail: format!("{}: unrecognised publication error", destination.display()),
                already_exists: false,
            });
        }
    };
    #[cfg(test)]
    if let Some(detail) = save_fault::take_published_failure() {
        if let Some(slot) = custody_slot
            && let Ok(mut guard) = slot.lock()
        {
            let _ = guard.replace(ArrangementUncertainAuthority::Durable(authority));
        }
        return Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail,
            already_exists: false,
        });
    }
    Ok(authority)
}

// Read the retained rotated bytes for a post-publication verification.
// `operation_label` is quoted into the uncertainty message so a caller
// distinguishes a metadata reopen from an arrangement reopen.
pub(super) fn read_retained_bytes(
    authority: &DocumentAuthority,
    operation_label: &str,
) -> Result<Vec<u8>, PdfWriteError> {
    match authority.read_bounded(PDF_MUTATION_MAX_BYTES) {
        Ok(DocumentSnapshot::Current {
            bytes,
            overflow: false,
            ..
        }) => Ok(bytes),
        Ok(DocumentSnapshot::Current { overflow: true, .. }) => Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail: format!("{operation_label} exceeded the mutation cap."),
            already_exists: false,
        }),
        Ok(DocumentSnapshot::Lost) => Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail: format!("{operation_label} lost retained custody."),
            already_exists: false,
        }),
        Err(error) => Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail: format!("{operation_label}: {error}"),
            already_exists: false,
        }),
    }
}

fn validate_date_write(
    date: &PdfDateField,
    field: MetadataField,
    confirmed_removal: MetadataDateRemoval,
) -> Result<(), PdfWriteError> {
    if !date.is_edited() {
        return Ok(());
    }
    if date.value().is_empty() {
        if date.original.is_none() || confirmed_removal.permits(field) {
            return Ok(());
        }
        return Err("Removing a PDF date requires confirmation"
            .to_string()
            .into());
    }
    validate_pdf_date(date.value())
        .map(|_| ())
        .map_err(|error| PdfWriteError::from(error.message().to_string()))
}

fn apply_date_write(
    dict: &mut lopdf::Dictionary,
    field: MetadataField,
    date: &PdfDateField,
    confirmed_removal: MetadataDateRemoval,
) {
    if !date.is_edited() {
        return;
    }
    if date.value().is_empty() {
        if date.original.is_some() && confirmed_removal.permits(field) {
            dict.remove(field.pdf_key());
        }
    } else {
        dict.set(
            String::from_utf8_lossy(field.pdf_key()).into_owned(),
            Object::String(date.value().as_bytes().to_vec(), StringFormat::Literal),
        );
    }
}

fn verify_metadata_write(
    requested: &PdfMetadata,
    source: &PdfMetadata,
    reopened: &PdfMetadata,
) -> Result<(), PdfWriteError> {
    let text_matches = MetadataField::TEXT_FIELDS
        .iter()
        .all(|field| field.get(requested) == field.get(reopened));
    let dates_match = [
        (
            &requested.creation_date,
            &source.creation_date,
            &reopened.creation_date,
        ),
        (
            &requested.modification_date,
            &source.modification_date,
            &reopened.modification_date,
        ),
    ]
    .into_iter()
    .all(|(requested, source, reopened)| {
        if !requested.is_edited() {
            return source.original == reopened.original;
        }
        if requested.value().is_empty() {
            return reopened.original.is_none();
        }
        reopened.original_bytes() == Some(requested.value().as_bytes())
    });
    if text_matches && dates_match {
        return Ok(());
    }
    Err(PdfWriteError {
        published: true,
        completed_outputs: 0,
        detail: "published metadata did not match the requested fields".into(),
        already_exists: false,
    })
}

/// Remove the whole `/Info` dictionary (`Clear all metadata`, for privacy).
///
/// Unlike writing blank fields, this leaves no metadata key at all. The reopen check reads
/// the retained rotated bytes and refuses if `/Info` reappears.
pub fn clear_metadata(path: &Path) -> Result<(), PdfWriteError> {
    let SourceAuthority {
        mut authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(path)?;
    if doc.is_encrypted() {
        return Err(PdfWriteError::from(format!(
            "{} is encrypted; unlock it before clearing metadata.",
            path.display()
        )));
    }
    if let Some(id) = info_id(&doc) {
        doc.objects.remove(&id);
    }
    doc.trailer.remove(b"Info");
    let serialized = serialize_document_bounded(&mut doc, "clear metadata")?;
    drop(doc);
    conditional_replace_authority(&mut authority, path, &serialized)?;
    let reopened_bytes = read_retained_bytes(&authority, "clear metadata reopen")?;
    let reopened = Document::load_mem(&reopened_bytes).map_err(|error| PdfWriteError {
        published: true,
        completed_outputs: 0,
        detail: format!("cleared metadata could not be reopened: {error}"),
        already_exists: false,
    })?;
    if info_id(&reopened).is_some() {
        return Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail: "cleared metadata reopened with /Info still present.".into(),
            already_exists: false,
        });
    }
    Ok(())
}

/// Protect the PDF at `path` with `password`, in place: AES-128 (the PDF 1.6
/// standard security handler), with `password` as both the open and the
/// owner password, so every reader asks for it and none is left with a
/// weaker way in. The published file is reopened and must refuse to open
/// without the password and open with it.
///
/// Not AES-256: poppler and other readers cannot decrypt lopdf's PDF 2.0 handler streams.
pub fn set_password(path: &Path, password: &str) -> Result<(), PdfWriteError> {
    use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};

    if password.is_empty() {
        return Err(PdfWriteError::from(
            "A password cannot be empty.".to_string(),
        ));
    }
    let SourceAuthority {
        mut authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(path)?;
    if doc.is_encrypted() || doc.trailer.get(b"Encrypt").is_ok() {
        return Err(PdfWriteError::from(format!(
            "{} already has a password; remove it first.",
            path.display()
        )));
    }
    // The file identifier is part of the key; a document without one gets a
    // fresh one.
    if doc.trailer.get(b"ID").is_err() {
        let id = Object::String(random_bytes::<16>()?.to_vec(), StringFormat::Hexadecimal);
        doc.trailer.set("ID", Object::Array(vec![id.clone(), id]));
    }
    let filter: std::sync::Arc<dyn CryptFilter> = std::sync::Arc::new(Aes128CryptFilter);
    let snapshot = doc.clone();
    let state = EncryptionState::try_from(EncryptionVersion::V4 {
        document: &snapshot,
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: password,
        user_password: password,
        permissions: Permissions::all(),
    })
    .map_err(|error| PdfWriteError::from(format!("Could not set up encryption: {error}")))?;
    doc.encrypt(&state)
        .map_err(|error| PdfWriteError::from(format!("Could not encrypt the PDF: {error}")))?;
    let serialized = serialize_document_bounded(&mut doc, "password protection")?;
    drop(doc);
    conditional_replace_authority(&mut authority, path, &serialized)?;
    let reopened = read_retained_bytes(&authority, "password protection reopen")?;
    let published = |detail: String| PdfWriteError {
        published: true,
        completed_outputs: 0,
        detail,
        already_exists: false,
    };
    let mut check = Document::load_mem(&reopened)
        .map_err(|error| published(format!("the protected PDF could not be reopened: {error}")))?;
    if !check.is_encrypted() {
        return Err(published("the published PDF is not encrypted".to_string()));
    }
    check.decrypt(password).map_err(|error| {
        published(format!(
            "the protected PDF does not open with its password: {error}"
        ))
    })?;
    Ok(())
}

/// Remove the password from the PDF at `path`, in place, given the current
/// `password`. A wrong password changes nothing.
pub fn remove_password(path: &Path, password: &str) -> Result<(), PdfWriteError> {
    let SourceAuthority {
        mut authority,
        bytes,
        document,
    } = open_source_authority(path)?;
    if !document.is_encrypted() {
        return Err(PdfWriteError::from(format!(
            "{} has no password to remove.",
            path.display()
        )));
    }
    drop(document);
    // Loaded again with the password, so the parser reads the object
    // streams and cross-references as plain objects from the start.
    let options = lopdf::LoadOptions {
        password: Some(password.to_owned()),
        ..lopdf::LoadOptions::default()
    };
    let mut doc = Document::load_mem_with_options(&bytes, options)
        .map_err(|_| PdfWriteError::from("That password is not right.".to_string()))?;
    if doc.is_encrypted() {
        return Err(PdfWriteError::from(
            "That password is not right.".to_string(),
        ));
    }
    doc.trailer.remove(b"Encrypt");
    let serialized = serialize_document_bounded(&mut doc, "password removal")?;
    drop(doc);
    conditional_replace_authority(&mut authority, path, &serialized)?;
    let reopened = read_retained_bytes(&authority, "password removal reopen")?;
    let check = Document::load_mem(&reopened).map_err(|error| PdfWriteError {
        published: true,
        completed_outputs: 0,
        detail: format!("the unprotected PDF could not be reopened: {error}"),
        already_exists: false,
    })?;
    if check.is_encrypted() {
        return Err(PdfWriteError {
            published: true,
            completed_outputs: 0,
            detail: "the published PDF still has a password".to_string(),
            already_exists: false,
        });
    }
    Ok(())
}

// `N` fresh bytes from the kernel's random source.
fn random_bytes<const N: usize>() -> Result<[u8; N], PdfWriteError> {
    use std::io::Read;
    let mut bytes = [0u8; N];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| {
            PdfWriteError::from(format!("Could not make a file identifier: {error}"))
        })?;
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod save_fault {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static FAIL_CALL: Cell<Option<usize>> = const { Cell::new(None) };
        static FAIL_DETAIL: RefCell<Option<String>> = const { RefCell::new(None) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
        static PUBLISHED_DETAIL: RefCell<Option<String>> = const { RefCell::new(None) };
        static PUBLISHED_FAIL_CALL: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub fn fail_with_detail(detail: impl Into<String>) {
        fail_on_call_with_detail(1, detail);
    }

    fn fail_on_call_with_detail(call: usize, detail: impl Into<String>) {
        CALLS.with(|calls| calls.set(0));
        FAIL_CALL.with(|slot| slot.set(Some(call)));
        FAIL_DETAIL.with(|slot| *slot.borrow_mut() = Some(detail.into()));
    }

    pub fn arm_published_failure_with_detail(detail: impl Into<String>) {
        // Clear any leftover call-guarded arm from another test on a
        // reused test thread so the unconditional variant reliably
        // fires on the next consumer.
        PUBLISHED_FAIL_CALL.with(|slot| slot.set(None));
        PUBLISHED_DETAIL.with(|slot| *slot.borrow_mut() = Some(detail.into()));
    }

    // Reset the thread-local call counter so reused test threads cannot shift the selected publication fault.
    pub fn arm_published_failure_on_call(call: usize, detail: impl Into<String>) {
        CALLS.with(|calls| calls.set(0));
        PUBLISHED_FAIL_CALL.with(|slot| slot.set(Some(call)));
        PUBLISHED_DETAIL.with(|slot| *slot.borrow_mut() = Some(detail.into()));
    }

    pub(super) fn take_failure_for_this_call() -> Option<String> {
        let call = CALLS.with(|calls| {
            let call = calls.get() + 1;
            calls.set(call);
            call
        });
        let should_fail = FAIL_CALL.with(|slot| slot.get() == Some(call));
        if !should_fail {
            return None;
        }
        FAIL_CALL.with(|slot| slot.set(None));
        FAIL_DETAIL.with(|slot| slot.borrow_mut().take())
    }

    pub(in crate::plumbing::pdf_ops) fn take_published_failure() -> Option<String> {
        // A guarded fault fires only on its armed call; without a guard it fires on the
        // next consumer.
        let armed_call = PUBLISHED_FAIL_CALL.with(|slot| slot.get());
        if let Some(call) = armed_call {
            let current = CALLS.with(|calls| calls.get());
            if current != call {
                return None;
            }
            PUBLISHED_FAIL_CALL.with(|slot| slot.set(None));
        }
        PUBLISHED_DETAIL.with(|slot| slot.borrow_mut().take())
    }

    /// Every armed fault and the call tally, lifted off this thread.
    ///
    /// The faults are thread-local, and the writes they target run on the
    /// blocking owner's worker: `App::dispatch_blocking` carries the state to
    /// the worker for the job and back afterwards, so an arm made on the test
    /// thread fires on the call it names wherever that call runs.
    #[derive(Debug, Default)]
    pub struct Carried {
        fail_call: Option<usize>,
        fail_detail: Option<String>,
        calls: usize,
        published_detail: Option<String>,
        published_fail_call: Option<usize>,
    }

    // Fault state travels with the job because async execution can change threads.
    pub fn carry() -> Carried {
        Carried {
            fail_call: FAIL_CALL.with(|slot| slot.replace(None)),
            fail_detail: FAIL_DETAIL.with(|slot| slot.borrow_mut().take()),
            calls: CALLS.with(|calls| calls.replace(0)),
            published_detail: PUBLISHED_DETAIL.with(|slot| slot.borrow_mut().take()),
            published_fail_call: PUBLISHED_FAIL_CALL.with(|slot| slot.replace(None)),
        }
    }

    impl Carried {
        // Fault state travels with the job because async execution can change threads.
        pub fn install(self) {
            FAIL_CALL.with(|slot| slot.set(self.fail_call));
            FAIL_DETAIL.with(|slot| *slot.borrow_mut() = self.fail_detail);
            CALLS.with(|calls| calls.set(self.calls));
            PUBLISHED_DETAIL.with(|slot| *slot.borrow_mut() = self.published_detail);
            PUBLISHED_FAIL_CALL.with(|slot| slot.set(self.published_fail_call));
        }
    }
}
