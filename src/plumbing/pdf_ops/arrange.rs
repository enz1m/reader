//! Arrangement publication and page extraction.

use super::*;

#[derive(Clone, Debug)]
pub struct ArrangementPublication {
    pub page_count: usize,
    pub annotation_snapshot: PdfAnnotationSnapshot,
}

struct ArrangementSource {
    path: PathBuf,
    pages: Vec<lopdf::ObjectId>,
}

fn inherited_page_value(
    document: &Document,
    mut object_id: lopdf::ObjectId,
    key: &[u8],
) -> Option<Object> {
    loop {
        let dictionary = document.get_object(object_id).ok()?.as_dict().ok()?;
        if let Ok(value) = dictionary.get(key) {
            return Some(value.clone());
        }
        object_id = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
}

fn stabilize_native_annotation_ids(document: &mut Document, page_id: lopdf::ObjectId) {
    for object_id in annotation_refs(document, page_id) {
        let stable_id = document
            .get_dictionary(object_id)
            .ok()
            .map(|dictionary| native_id(dictionary, object_id));
        let Some(stable_id) = stable_id else {
            continue;
        };
        if let Ok(dictionary) = document.get_dictionary_mut(object_id)
            && !dictionary.has(b"NM")
        {
            dictionary.set(
                "NM",
                Object::String(stable_id.into_bytes(), StringFormat::Literal),
            );
        }
    }
}

fn flatten_page_inheritance(document: &mut Document, page_id: lopdf::ObjectId) {
    for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
        let already_direct = document
            .get_object(page_id)
            .ok()
            .and_then(|object| object.as_dict().ok())
            .is_some_and(|dictionary| dictionary.has(key));
        if already_direct {
            continue;
        }
        let Some(value) = inherited_page_value(document, page_id, key) else {
            continue;
        };
        if let Ok(dictionary) = document
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
        {
            dictionary.set(key, value);
        }
    }
}

fn source_for_entry<'a>(
    entry: &'a crate::pages::PageEntry,
    original: &'a Path,
) -> (&'a Path, usize) {
    match &entry.source {
        crate::pages::PageSource::Original(index) => (original, *index),
        crate::pages::PageSource::Merged { file, index, .. } => (Path::new(file), *index),
    }
}

fn arrangement_error(detail: impl Into<String>) -> PdfWriteError {
    PdfWriteError::from(detail.into())
}

pub fn publish_arrangement(
    original: &Path,
    expected_original_fingerprint: &str,
    entries: &[crate::pages::PageEntry],
    destination: &Path,
) -> Result<ArrangementPublication, PdfWriteError> {
    publish_arrangement_with_custody(
        original,
        expected_original_fingerprint,
        entries,
        destination,
        None,
    )
}

/// Publish the arrangement and, on Save As, hand any `create_new` retained
/// custody back to the reducer through `custody_slot`.
///
/// Callers that never induce a durability or lost-custody outcome pass `None`.
/// `crate::app::App::begin_arrangement_publication` passes an
/// [`ArrangementUncertainSlot`] that `crate::app::App::finish_arrangement_publication`
/// drains after the worker settles. In-place publication never fills the slot, since the
/// reducer already owns the source authority.
pub fn publish_arrangement_with_custody(
    original: &Path,
    expected_original_fingerprint: &str,
    entries: &[crate::pages::PageEntry],
    destination: &Path,
    custody_slot: Option<ArrangementUncertainSlot>,
) -> Result<ArrangementPublication, PdfWriteError> {
    if entries.is_empty() {
        return Err(arrangement_error("a PDF must contain at least one page"));
    }

    let mut specifications = vec![(
        original.to_path_buf(),
        expected_original_fingerprint.to_string(),
    )];
    for entry in entries {
        if let crate::pages::PageSource::Merged {
            file,
            document_fingerprint,
            ..
        } = &entry.source
        {
            let path = PathBuf::from(file);
            if let Some((_, expected)) = specifications.iter().find(|(known, _)| known == &path) {
                if expected != document_fingerprint {
                    return Err(arrangement_error(format!(
                        "the draft contains conflicting identities for {}",
                        path.display()
                    )));
                }
            } else {
                specifications.push((path, document_fingerprint.clone()));
            }
        }
    }

    // One retained authority per source path: every fingerprint check, page-tree walk and
    // encryption refusal reads its bytes, not a fresh pathname open. The original's
    // authority survives the loop so in-place publication can rotate custody through it.
    let mut output = Document::new();
    let mut sources = Vec::with_capacity(specifications.len());
    let mut original_info = None;
    let mut original_catalog = None;
    let mut original_authority: Option<DocumentAuthority> = None;
    for (path, expected_fingerprint) in specifications {
        let SourceAuthority {
            authority,
            bytes: _admission_bytes,
            mut document,
        } = open_source_authority(&path)?;
        if document.is_encrypted() {
            return Err(arrangement_error(format!(
                "{} is encrypted and cannot be arranged while locked",
                path.display()
            )));
        }
        let identity = snapshot_from_document(&document)
            .map_err(|error| arrangement_error(error.to_string()))?;
        if identity.document_fingerprint != expected_fingerprint {
            return Err(arrangement_error(format!(
                "{} changed after it was added to the draft",
                path.display()
            )));
        }
        let pages_before = document.get_pages();
        for entry in entries {
            let (entry_path, index) = source_for_entry(entry, original);
            if entry_path == path.as_path() {
                let page_id = pages_before
                    .get(&((index + 1) as u32))
                    .copied()
                    .ok_or_else(|| {
                        arrangement_error(format!(
                            "page {} is no longer present in {}",
                            index + 1,
                            path.display()
                        ))
                    })?;
                flatten_page_inheritance(&mut document, page_id);
                stabilize_native_annotation_ids(&mut document, page_id);
            }
        }

        let next_id = output.max_id.saturating_add(1);
        document.renumber_objects_with(next_id);
        let pages = document.get_pages().into_values().collect::<Vec<_>>();
        if path.as_path() == original {
            original_catalog = document.catalog().ok().cloned();
            original_info = document
                .trailer
                .get(b"Info")
                .ok()
                .and_then(|value| value.as_reference().ok());
            output.version = document.version.clone();
            original_authority = Some(authority);
        } else if document.version > output.version {
            output.version = document.version.clone();
        }
        output.max_id = output.max_id.max(document.max_id);
        output.objects.extend(document.objects);
        sources.push(ArrangementSource { path, pages });
    }

    let mut ordered_page_ids = Vec::with_capacity(entries.len());
    for entry in entries {
        let (path, index) = source_for_entry(entry, original);
        let source = sources
            .iter()
            .find(|source| source.path.as_path() == path)
            .ok_or_else(|| {
                arrangement_error(format!("source {} was not loaded", path.display()))
            })?;
        let page_id = source.pages.get(index).copied().ok_or_else(|| {
            arrangement_error(format!(
                "page {} is no longer present in {}",
                index + 1,
                path.display()
            ))
        })?;
        ordered_page_ids.push((page_id, entry.rotation.degrees()));
    }

    let pages_id = output.new_object_id();
    for (page_id, extra_rotation) in &ordered_page_ids {
        let page = output
            .get_object_mut(*page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| arrangement_error(error.to_string()))?;
        let base_rotation = page
            .get(b"Rotate")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(0);
        let rotation = (base_rotation + i64::from(*extra_rotation)).rem_euclid(360);
        page.set("Parent", pages_id);
        page.set("Rotate", rotation);
    }
    output.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => ordered_page_ids
                .iter()
                .map(|(page_id, _)| Object::Reference(*page_id))
                .collect::<Vec<_>>(),
            "Count" => entries.len() as i64,
        }),
    );
    let mut catalog = original_catalog.unwrap_or_else(|| {
        dictionary! {
            "Type" => "Catalog",
        }
    });
    catalog.set("Pages", pages_id);
    let catalog_id = output.add_object(catalog);
    output.trailer.set("Root", catalog_id);
    if let Some(info) = original_info {
        output.trailer.set("Info", info);
    }
    output.max_id = output.objects.keys().map(|(id, _)| *id).max().unwrap_or(0);
    output.prune_objects();
    output.renumber_objects();

    let serialized = serialize_document_bounded(&mut output, "arrangement publication")?;
    drop(output);

    // In place, custody rotates on the original's retained authority. Save As uses
    // [`DocumentAuthority::create_new`]: atomic, strictly no-clobber, and returning the
    // retained inode, so an existing file is a typed refusal. Overwriting an existing
    // destination goes through [`DocumentAuthority::open_existing`] instead.
    let reopened_bytes = if destination == original {
        let Some(mut authority) = original_authority else {
            return Err(arrangement_error(
                "in-place arrangement publication requires the original's retained authority",
            ));
        };
        conditional_replace_authority(&mut authority, destination, &serialized)?;
        read_retained_bytes(&authority, "arrangement in-place reopen")?
    } else {
        let destination_authority =
            create_new_bytes(destination, &serialized, custody_slot.as_ref())?;
        read_retained_bytes(&destination_authority, "arrangement Save As reopen")?
    };
    let reopened = Document::load_mem(&reopened_bytes).map_err(|error| {
        PdfWriteError::published(format!("published PDF could not be reopened: {error}"))
    })?;
    let page_count = reopened.get_pages().len();
    if page_count != entries.len() {
        return Err(PdfWriteError::published(format!(
            "published PDF reopened with {page_count} pages instead of {}",
            entries.len()
        )));
    }
    let annotation_snapshot = snapshot_from_document(&reopened).map_err(|error| {
        PdfWriteError::published(format!(
            "published annotation identities could not be verified: {error}"
        ))
    })?;
    Ok(ArrangementPublication {
        page_count,
        annotation_snapshot,
    })
}

/// Write page `index` (0-based) of the PDF at `src` to a caller-picked `dest`.
///
/// `src` is not modified, and an out-of-range index is an error. Publication uses strict
/// [`DocumentAuthority::create_new`], so an existing file at `dest` is a
/// [`CreateDocumentError::NotPublished`] refusal with
/// [`std::io::ErrorKind::AlreadyExists`]. An overwrite must retain the destination
/// [`DocumentAuthority`] and go through [`DocumentAuthority::replace`]. Menus use
/// `extract_page_no_clobber`.
pub fn extract_page(src: &Path, index: usize, dest: &Path) -> Result<(), PdfWriteError> {
    let SourceAuthority {
        authority: _authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(src)?;
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    let target = (index + 1) as u32;
    if !pages.contains_key(&target) {
        return Err(PdfWriteError::from(format!(
            "Page {} does not exist (the document has {} pages).",
            index + 1,
            pages.len()
        )));
    }
    let others: Vec<u32> = pages.keys().copied().filter(|n| *n != target).collect();
    doc.delete_pages(&others);
    doc.prune_objects();
    doc.renumber_objects();
    let serialized = serialize_document_bounded(&mut doc, "extract publication")?;
    // This entry point has no debt slot, so the authority drops after reopen verification.
    let authority = create_new_bytes(dest, &serialized, None)?;
    let _ = read_retained_bytes(&authority, "extract caller-picked reopen")?;
    Ok(())
}

/// Extract page `index` beside `src` at a no-clobber sibling name, retaining the
/// authority of a successful output on `debt.completed` or the `PublishedButNotDurable` /
/// `Lost` custody of a failing one on `debt.uncertain`.
///
/// Tries [`extract_destination`] (`<stem>-page-<n>.pdf`), then `-2`, `-3`, … up to
/// `MAX_SIBLING_COLLISION_TRIES` under strict [`DocumentAuthority::create_new`], so an
/// existing file is never truncated. Returns the path written.
pub fn extract_page_no_clobber_with_custody(
    src: &Path,
    index: usize,
    total_pages_hint: Option<usize>,
    debt: &mut ExtractPublicationDebt,
) -> Result<std::path::PathBuf, PdfWriteError> {
    let SourceAuthority {
        authority: _authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(src)?;
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    let target = (index + 1) as u32;
    if !pages.contains_key(&target) {
        let n = total_pages_hint.unwrap_or(pages.len());
        return Err(PdfWriteError::from(format!(
            "Page {} does not exist (the document has {n} pages).",
            index + 1
        )));
    }
    let others: Vec<u32> = pages.keys().copied().filter(|n| *n != target).collect();
    doc.delete_pages(&others);
    doc.prune_objects();
    doc.renumber_objects();
    let page = index + 1;
    publish_no_clobber_sibling(
        &mut doc,
        src,
        &format!("page-{page}"),
        &format!("page {page}"),
        debt,
    )
}

/// Extract the pages at `indices` (0-based, any order) into one new PDF
/// beside `src`, in the document's own page order, at a no-clobber sibling
/// name that says which pages it holds: `<stem>-pages-2-to-5.pdf` for a run,
/// `<stem>-pages-1-4-7.pdf` for a few, `<stem>-9-pages.pdf` for many.
/// Custody of the output is retained on `debt` as for
/// [`extract_page_no_clobber_with_custody`].
pub fn extract_pages_no_clobber_with_custody(
    src: &Path,
    indices: &[usize],
    debt: &mut ExtractPublicationDebt,
) -> Result<std::path::PathBuf, PdfWriteError> {
    let mut wanted: Vec<u32> = indices.iter().map(|&index| (index + 1) as u32).collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return Err(PdfWriteError::from(
            "No pages were chosen to extract.".to_string(),
        ));
    }
    let SourceAuthority {
        authority: _authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(src)?;
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    if let Some(missing) = wanted.iter().find(|page| !pages.contains_key(page)) {
        return Err(PdfWriteError::from(format!(
            "Page {missing} does not exist (the document has {} pages).",
            pages.len()
        )));
    }
    let others: Vec<u32> = pages
        .keys()
        .copied()
        .filter(|page| wanted.binary_search(page).is_err())
        .collect();
    doc.delete_pages(&others);
    doc.prune_objects();
    doc.renumber_objects();
    let (label, what) = pages_label(&wanted);
    publish_no_clobber_sibling(&mut doc, src, &label, &what, debt)
}

// The file-name label and the prose for 1-based `pages` (sorted, unique).
pub(super) fn pages_label(pages: &[u32]) -> (String, String) {
    match pages {
        [page] => (format!("page-{page}"), format!("page {page}")),
        [first, .., last] if (last - first) as usize + 1 == pages.len() => (
            format!("pages-{first}-to-{last}"),
            format!("pages {first} to {last}"),
        ),
        _ if pages.len() <= 4 => {
            let list: Vec<String> = pages.iter().map(u32::to_string).collect();
            (
                format!("pages-{}", list.join("-")),
                format!("pages {}", list.join(", ")),
            )
        }
        _ => (
            format!("{}-pages", pages.len()),
            format!("{} pages", pages.len()),
        ),
    }
}

/// The default destination for extracting page `index` of `src`:
/// `<stem>-page-<n>.pdf` beside the source.
///
/// Also the first candidate of the no-clobber flow, so `Extract this page` and
/// `Split into pages` name files alike.
pub fn extract_destination(src: &Path, index: usize) -> std::path::PathBuf {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let dir = src.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("{stem}-page-{}.pdf", index + 1))
}

/// Cap on numeric-suffix retries before a no-clobber sibling publication refuses.
pub const MAX_SIBLING_COLLISION_TRIES: u32 = 100;

// Publish `doc` beside `src` as `<stem>-<label>.pdf`, or `-2`, `-3`, … on a collision.
// `what` names the pages in an error.
//
// Each candidate goes through [`create_new_bytes`]; `AlreadyExists` advances the ladder,
// and other errors propagate with `debt` already holding the failing output. A success
// is reopen-verified through [`read_retained_bytes`] and pushed onto `debt.completed`.
// Refuses after [`MAX_SIBLING_COLLISION_TRIES`] collisions or past
// [`MAX_RETAINED_EXTRACT_HANDLES`] retained outputs.
fn publish_no_clobber_sibling(
    doc: &mut Document,
    src: &Path,
    label: &str,
    what: &str,
    debt: &mut ExtractPublicationDebt,
) -> Result<std::path::PathBuf, PdfWriteError> {
    if debt.completed.len() >= MAX_RETAINED_EXTRACT_HANDLES {
        return Err(PdfWriteError::from(format!(
            "Retained extract handles reached the cap ({MAX_RETAINED_EXTRACT_HANDLES}); further outputs are refused this run.",
        )));
    }
    let serialized = serialize_document_bounded(doc, "extract publication")?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let dir = src.parent().unwrap_or_else(|| Path::new("."));
    let base = dir.join(format!("{stem}-{label}.pdf"));
    let mut suffix: u32 = 1;
    loop {
        let candidate = if suffix == 1 {
            base.clone()
        } else {
            dir.join(format!("{stem}-{label}-{suffix}.pdf"))
        };
        // `create_new_bytes` writes the slot only on `PublishedButNotDurable` / `Lost`, so
        // a collision leaves it untouched.
        let per_call: ArrangementUncertainSlot = Arc::new(Mutex::new(None));
        match create_new_bytes(&candidate, &serialized, Some(&per_call)) {
            Ok(authority) => {
                // Verify through the retained descriptor; on failure the authority moves to
                // `debt.uncertain` and the error propagates.
                match read_retained_bytes(&authority, "extract reopen") {
                    Ok(_) => {
                        debt.completed.push(authority);
                        return Ok(candidate);
                    }
                    Err(reopen_error) => {
                        debt.uncertain = Some(ArrangementUncertainAuthority::Durable(authority));
                        return Err(reopen_error);
                    }
                }
            }
            Err(error) if error.is_already_exists() && !error.was_published() => {
                suffix = suffix.saturating_add(1);
                if suffix > MAX_SIBLING_COLLISION_TRIES {
                    return Err(PdfWriteError::from(format!(
                        "Could not extract {what}: every candidate name from `{}` through the {suffix}th collision is taken.",
                        base.display(),
                    )));
                }
            }
            Err(error) => {
                if let Ok(mut guard) = per_call.lock()
                    && let Some(uncertain) = guard.take()
                {
                    debt.uncertain = Some(uncertain);
                }
                return Err(error);
            }
        }
    }
}

/// `Split into pages` / `Split PDF`: publish every page of `src` as its own no-clobber
/// sibling and return the paths written, in page order.
///
/// For callers that need no custody; the app uses [`split_pdf_with_custody`].
pub fn split_pdf(src: &Path) -> Result<Vec<std::path::PathBuf>, PdfWriteError> {
    let mut debt = ExtractPublicationDebt::default();
    split_pdf_with_custody(src, &mut debt)
}

/// [`split_pdf`] variant that retains each successful output's
/// authority on `debt.completed` and, on a mid-run failure, the
/// failing output's `PublishedButNotDurable` / `Lost` custody on
/// `debt.uncertain`.
///
/// Built on [`extract_page_no_clobber_with_custody`], so split and `Extract this page`
/// agree for every index. A mid-run failure's
/// [`completed_outputs`](PdfWriteError::completed_outputs) counts only files this call
/// published. Refuses when the page count exceeds [`MAX_RETAINED_EXTRACT_HANDLES`].
pub fn split_pdf_with_custody(
    src: &Path,
    debt: &mut ExtractPublicationDebt,
) -> Result<Vec<std::path::PathBuf>, PdfWriteError> {
    // Count pages from one retained source read; each page publication retains its own
    // authority.
    let SourceAuthority { document: doc, .. } = open_source_authority(src)?;
    let n = doc.get_pages().len();
    drop(doc);
    if n == 0 {
        return Err(PdfWriteError::from(
            "This document has no pages to split.".to_string(),
        ));
    }
    if n > MAX_RETAINED_EXTRACT_HANDLES {
        return Err(PdfWriteError::from(format!(
            "Split refused: {n} pages exceed the per-run cap of {MAX_RETAINED_EXTRACT_HANDLES} retained output handles.",
        )));
    }
    let mut written = Vec::with_capacity(n);
    for index in 0..n {
        match extract_page_no_clobber_with_custody(src, index, Some(n), debt) {
            Ok(path) => written.push(path),
            Err(error) => {
                let visible = written.len() + usize::from(error.was_published());
                return Err(error.with_completed_outputs(visible));
            }
        }
    }
    Ok(written)
}
