//! Blocking PDF structure edits through atomic user-file publication.
//!
//! Empty text fields remain PDF strings. Removing dates and clearing metadata are
//! separate explicit intents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};

use enzui_iced_core::user_file::{
    CreateDocumentError, DocumentAuthority, DocumentSnapshot, LostCreateStage, ReadError,
    ReplaceCustody, StagedBytes,
};
use lopdf::{Document, Object, ObjectId, StringFormat, dictionary};

/// Retained descriptors a Save As arrangement publication hands to the reducer when
/// `DocumentAuthority::create_new` returned a non-`NotPublished` error. Kept on `App`
/// because the authority types are not [`Clone`]; the model carries a
/// [`crate::model::ArrangementUncertainPublication`] summary.
#[derive(Debug)]
pub enum ArrangementUncertainAuthority {
    /// [`CreateDocumentError::PublishedButNotDurable`]: the published inode is retained so
    /// reconciliation can call [`DocumentAuthority::read_bounded`] without reopening the
    /// path.
    Durable(DocumentAuthority),
    /// [`CreateDocumentError::Lost`]: the staged descriptor is retained with the
    /// framework's [`StagedBytes`] naming, so the reducer can report where the bytes are
    /// visible without claiming the destination name denotes them.
    Lost {
        staged_handle: LostCreateStage,
        staged: StagedBytes,
    },
}

/// Public summary of an [`ArrangementUncertainAuthority`] variant for the
/// `Model`-side banner / close-veto message. Deliberately a plain
/// [`Clone`] enum: it never carries a retained descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrangementUncertainKind {
    /// Retained authority is a [`DocumentAuthority`] for the exact published
    /// inode (`PublishedButNotDurable`).
    Durable,
    /// Retained descriptor is a [`LostCreateStage`] and the framework
    /// reported these staged-bytes coordinates.
    Lost { staged: String },
}

/// One-line rendering of [`StagedBytes`] suitable for a
/// [`crate::model::ArrangementUncertainPublication::message`] user banner.
pub fn staged_bytes_description(staged: &StagedBytes) -> String {
    match staged {
        StagedBytes::Discarded => "staged bytes discarded".to_string(),
        StagedBytes::AtDestination => "staged bytes at the destination name".to_string(),
        StagedBytes::AtPath(path) => format!("staged bytes named {}", path.display()),
        StagedBytes::Unnamed => "staged bytes named nowhere".to_string(),
        _ => "staged bytes in an unrecognised state".to_string(),
    }
}

/// Shared retention slot for the arrangement Save As custody handoff: the reducer keeps
/// one [`Arc`] and moves another into the worker, which writes the authority once on
/// `PublishedButNotDurable` / `Lost`; the reducer drains it on
/// [`crate::message::Msg::PageArrangementPublished`].
pub type ArrangementUncertainSlot = Arc<Mutex<Option<ArrangementUncertainAuthority>>>;

/// Page-count-bounded cap on retained extract/split output authorities.
///
/// A run that would retain more is refused before opening the source, so one split cannot
/// drain the file-descriptor budget. Larger than the per-page
/// `MAX_SIBLING_COLLISION_TRIES`, since this cap is per run.
pub const MAX_RETAINED_EXTRACT_HANDLES: usize = 256;

/// Move-only, page-cap-bounded retention for an extract or split
/// publication run.
///
/// Each successful [`DocumentAuthority::create_new`] pushes onto [`Self::completed`]; a
/// failing output's `PublishedButNotDurable` or `Lost` outcome installs
/// [`Self::uncertain`]. On success the reducer drops the record to release descriptors;
/// on failure it moves to `crate::app::App::extract_publication_debt` for close-drain and
/// Dismiss. Not [`Clone`]: the handles are unique kernel resources.
#[derive(Debug, Default)]
pub struct ExtractPublicationDebt {
    /// Per-output retained authorities in output order. Bounded by
    /// the source's page count and by [`MAX_RETAINED_EXTRACT_HANDLES`].
    pub completed: Vec<DocumentAuthority>,
    /// Retained custody for the failing output, when the framework
    /// returned a variant of [`CreateDocumentError`] that carries a
    /// descriptor.
    pub uncertain: Option<ArrangementUncertainAuthority>,
}

/// Public summary of an [`ExtractPublicationDebt`] failure for the
/// `Model`-side banner and close-veto message.
///
/// Counts and a classification only; retained pathnames never reach shell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractUncertainKind {
    /// The failing output was retained as a [`DocumentAuthority`]
    /// (`PublishedButNotDurable`).
    Durable,
    /// The failing output was retained as a
    /// [`LostCreateStage`] with the framework's `StagedBytes` naming.
    Lost { staged: String },
    /// The failing output was not published; only earlier outputs (if
    /// any) reached disk.
    NotPublished,
}

// Bounded acquisition of an existing PDF through the framework's retained document
// authority. Every later read, mutation and reopen in this module addresses that
// descriptor, so substituting the file at the path cannot change what is parsed,
// verified or published. The `Document` is parsed from the retained bytes, so
// [`snapshot_from_document`] fingerprints the retained inode.
struct SourceAuthority {
    authority: DocumentAuthority,
    bytes: Vec<u8>,
    document: Document,
}

fn open_source_authority(path: &Path) -> Result<SourceAuthority, PdfWriteError> {
    let authority = DocumentAuthority::open_existing(path).map_err(|error| {
        PdfWriteError::from(format!("Could not open {}: {error}", path.display()))
    })?;
    let bytes = match authority.read_bounded(PDF_MUTATION_MAX_BYTES) {
        Ok(DocumentSnapshot::Current {
            bytes,
            overflow: false,
            ..
        }) => bytes,
        Ok(DocumentSnapshot::Current { overflow: true, .. }) => {
            return Err(PdfWriteError::from(format!(
                "{} exceeds the {PDF_MUTATION_MAX_BYTES}-byte cap.",
                path.display()
            )));
        }
        Ok(DocumentSnapshot::Lost) => {
            return Err(PdfWriteError::from(format!(
                "the retained authority for {} reported lost custody before the first read.",
                path.display()
            )));
        }
        Err(ReadError::ConcurrentMutation) => {
            return Err(PdfWriteError::from(format!(
                "{} was mutated concurrently during the bounded read.",
                path.display()
            )));
        }
        Err(ReadError::CapOverflow) => {
            return Err(PdfWriteError::from(format!(
                "the {PDF_MUTATION_MAX_BYTES}-byte cap cannot represent an overflow buffer for {}.",
                path.display()
            )));
        }
        Err(ReadError::Io(error)) => {
            return Err(PdfWriteError::from(format!("{}: {error}", path.display())));
        }
    };
    let document = Document::load_mem(&bytes).map_err(|error| {
        PdfWriteError::from(format!("Could not parse {}: {error}", path.display()))
    })?;
    Ok(SourceAuthority {
        authority,
        bytes,
        document,
    })
}

#[path = "pdf_ops/annotations.rs"]
mod annotations;
#[path = "pdf_ops/arrange.rs"]
mod arrange;
#[path = "pdf_ops/metadata.rs"]
mod metadata;
#[path = "pdf_ops/mutations.rs"]
mod mutations;
#[path = "pdf_ops/write.rs"]
mod write;

pub use annotations::{
    PdfAnnotationKind, PdfAnnotationSnapshot, PdfPageIdentity, PdfRect, StoredAnnotation,
    delete_annotation, read_annotations, read_document_annotations, snapshot_from_bytes,
    update_annotation, write_annotation, write_annotation_with_id,
};
use annotations::{
    add_annotation_to_document, annotation_refs, delete_annotation_from_document, native_id,
    update_annotation_in_document,
};
pub(crate) use annotations::{
    annotation_fingerprint, highlight_rgb, snapshot_from_document, text_fingerprint,
};
pub use arrange::{
    ArrangementPublication, extract_destination, extract_page,
    extract_page_no_clobber_with_custody, extract_pages_no_clobber_with_custody,
    publish_arrangement, publish_arrangement_with_custody, split_pdf, split_pdf_with_custody,
};
pub use metadata::{
    DocProperties, MALFORMED_PDF_DATE_WARNING, MetadataDateRemoval, MetadataField,
    PDF_DATE_FORMAT_HELP, PDF_DATE_REMOVAL_NOTICE, PdfDatePrecision, PdfMetadata, format_file_size,
    page_count, pdf_date_from_parts, pdf_date_parts, read_metadata, read_properties,
};
use metadata::{
    PdfDateField, decode_pdf_string, encode_pdf_string, info_id, load, read_metadata_from,
    validate_pdf_date,
};
pub use mutations::{
    PDF_MUTATION_MAX_BYTES, PdfMutationCustody, PdfMutationOp, PdfMutationOutcome,
    PdfMutationRefreshed, PdfMutationRefusal, PdfMutationRequest, apply_annotation_mutation,
};
#[cfg(test)]
pub(crate) use write::save_fault;
pub use write::{
    PdfWriteError, clear_metadata, remove_password, set_password, write_metadata,
    write_metadata_with_date_removal,
};
use write::{
    conditional_replace_authority, create_new_bytes, read_retained_bytes, save_document,
    serialize_document_bounded,
};

#[cfg(test)]
mod date_tests;

#[cfg(test)]
mod tests;
