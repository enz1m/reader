//! Native annotations: read, write, and delete.

use super::*;

/// The `/Subtype` of a PDF annotation this app writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfAnnotationKind {
    /// A text-markup highlight over a rectangle (`PDF Highlight`).
    Highlight,
    /// A sticky note anchored at a point (`Add comment on page`).
    Comment,
}

impl PdfAnnotationKind {
    /// The PDF `/Subtype` name from the spec's annotation table, so other readers find
    /// the annotation.
    pub fn subtype(self) -> &'static str {
        match self {
            PdfAnnotationKind::Highlight => "Highlight",
            PdfAnnotationKind::Comment => "Text",
        }
    }
}

/// A rectangle in PDF user space (points, origin bottom-left), the space of an
/// annotation's `/Rect`.
///
/// Distinct from the normalised [`crate::plumbing::persistence::AnnotationPosition`]:
/// converting needs the page's MediaBox, and a normalised rect here would produce a
/// 1×1-point highlight in the corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PdfRect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        PdfRect { x0, y0, x1, y1 }
    }

    // The `/Rect` array, normalised so `x0 <= x1` and `y0 <= y1`. A viewer
    // given an inverted rect draws nothing, so a drag that went right-to-left
    // would silently produce an invisible highlight.
    fn to_array(self) -> Vec<Object> {
        let (x0, x1) = if self.x0 <= self.x1 {
            (self.x0, self.x1)
        } else {
            (self.x1, self.x0)
        };
        let (y0, y1) = if self.y0 <= self.y1 {
            (self.y0, self.y1)
        } else {
            (self.y1, self.y0)
        };
        vec![
            Object::Real(x0),
            Object::Real(y0),
            Object::Real(x1),
            Object::Real(y1),
        ]
    }

    // The `/QuadPoints` array a `/Highlight` needs: the four corners in the
    // spec's order (upper-left, upper-right, lower-left, lower-right).
    //
    // Most viewers paint a `/Highlight`'s quads, not its `/Rect`, so omitting them makes
    // the highlight invisible.
    fn to_quad_points(self) -> Vec<Object> {
        let (x0, x1) = if self.x0 <= self.x1 {
            (self.x0, self.x1)
        } else {
            (self.x1, self.x0)
        };
        let (y0, y1) = if self.y0 <= self.y1 {
            (self.y0, self.y1)
        } else {
            (self.y1, self.y0)
        };
        vec![
            Object::Real(x0),
            Object::Real(y1),
            Object::Real(x1),
            Object::Real(y1),
            Object::Real(x0),
            Object::Real(y0),
            Object::Real(x1),
            Object::Real(y0),
        ]
    }
}

// The RGB triple a highlight colour name maps to, as PDF `/C` reals.
//
// The names are the app's palette (`yellow | orange | red | magenta | aqua | lime`); an
// unknown name falls back to yellow, since an invisible highlight is worse.
pub(crate) fn highlight_rgb(color: &str) -> [f32; 3] {
    match color {
        "orange" => [1.0, 0.65, 0.0],
        "red" => [1.0, 0.32, 0.32],
        "magenta" => [1.0, 0.4, 0.8],
        "aqua" => [0.4, 0.9, 1.0],
        "lime" => [0.6, 1.0, 0.4],
        _ => [1.0, 0.94, 0.4],
    }
}

/// Write a PDF annotation into page `page_index` (0-based) of the file at
/// `path`.
///
/// * `Highlight` — a `/Highlight` text-markup annotation over `rect`, with
///   `/QuadPoints`, `/C` from `color` and `/Contents` from `contents`.
/// * `Comment` — a `/Text` sticky note anchored at `rect`'s corner, whose
///   `/Contents` is the comment body.
///
/// The annotation is appended to the page's `/Annots`, creating it if needed, so
/// annotations from other tools are preserved.
pub fn write_annotation(
    path: &Path,
    page_index: usize,
    kind: PdfAnnotationKind,
    rect: PdfRect,
    contents: &str,
    color: &str,
) -> Result<(), PdfWriteError> {
    write_annotation_with_id(path, page_index, kind, rect, contents, color, None)
}

/// Write one annotation with an optional stable PDF `/NM` identity.
pub fn write_annotation_with_id(
    path: &Path,
    page_index: usize,
    kind: PdfAnnotationKind,
    rect: PdfRect,
    contents: &str,
    color: &str,
    native_id: Option<&str>,
) -> Result<(), PdfWriteError> {
    let SourceAuthority {
        mut authority,
        bytes: _admission_bytes,
        document: mut doc,
    } = open_source_authority(path)?;
    if doc.is_encrypted() {
        return Err(PdfWriteError::from(format!(
            "{} is encrypted; unlock it before writing annotations.",
            path.display()
        )));
    }
    add_annotation_to_document(
        &mut doc,
        page_index,
        kind,
        rect,
        &[],
        contents,
        color,
        native_id,
    )?;
    let serialized = serialize_document_bounded(&mut doc, "annotation write")?;
    drop(doc);
    conditional_replace_authority(&mut authority, path, &serialized)
}

// Mutate `doc` in place by appending one annotation, so [`apply_annotation_mutation`]
// checks identity, mutates and publishes on one loaded document.
//
// `quads` are a highlight's per-line boxes; empty means one box, `rect`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_annotation_to_document(
    doc: &mut Document,
    page_index: usize,
    kind: PdfAnnotationKind,
    rect: PdfRect,
    quads: &[PdfRect],
    contents: &str,
    color: &str,
    native_id: Option<&str>,
) -> Result<(), PdfWriteError> {
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    let page_id = *pages.get(&((page_index + 1) as u32)).ok_or_else(|| {
        format!(
            "Page {} does not exist (the document has {} pages).",
            page_index + 1,
            pages.len()
        )
    })?;

    let mut annot = lopdf::Dictionary::new();
    annot.set("Type", Object::Name(b"Annot".to_vec()));
    annot.set("Subtype", Object::Name(kind.subtype().as_bytes().to_vec()));
    annot.set("Rect", Object::Array(rect.to_array()));
    annot.set(
        "Contents",
        Object::String(encode_pdf_string(contents), StringFormat::Literal),
    );
    // `/T` is the annotation's author line, shown in every viewer's comment
    // pane. Without it the note is attributed to nobody.
    annot.set(
        "T",
        Object::String(b"Reader".to_vec(), StringFormat::Literal),
    );
    let native_id = native_id
        .map(str::to_owned)
        .unwrap_or_else(|| format!("enzui-{page_index}-{}", doc.max_id.saturating_add(1)));
    annot.set(
        "NM",
        Object::String(native_id.into_bytes(), StringFormat::Literal),
    );
    match kind {
        PdfAnnotationKind::Highlight => {
            // One quad per selected line, so a selection that starts or ends
            // mid-line marks only its own words.
            let points = if quads.is_empty() {
                rect.to_quad_points()
            } else {
                quads
                    .iter()
                    .flat_map(|quad| quad.to_quad_points())
                    .collect()
            };
            annot.set("QuadPoints", Object::Array(points));
            let [r, g, b] = highlight_rgb(color);
            annot.set(
                "C",
                Object::Array(vec![Object::Real(r), Object::Real(g), Object::Real(b)]),
            );
        }
        PdfAnnotationKind::Comment => {
            // A closed sticky note with the standard `Comment` icon.
            annot.set("Name", Object::Name(b"Comment".to_vec()));
            annot.set("Open", Object::Boolean(false));
        }
    }
    let annot_id = doc.add_object(Object::Dictionary(annot));

    // Append, never replace: an existing `/Annots` may carry another tool's
    // annotations, and overwriting it deletes them silently.
    let existing: Vec<Object> = match doc.get_dictionary(page_id).ok().map(|d| d.get(b"Annots")) {
        Some(Ok(Object::Array(a))) => a.clone(),
        Some(Ok(Object::Reference(id))) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut updated = existing;
    updated.push(Object::Reference(annot_id));
    doc.get_dictionary_mut(page_id)
        .map_err(|e| format!("Malformed page dictionary: {e}"))?
        .set("Annots", Object::Array(updated));
    Ok(())
}

/// A PDF page's immutable content identity and coordinate box.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageIdentity {
    pub fingerprint: String,
    pub rotation_degrees: i32,
    pub x0: f32,
    pub y0: f32,
    pub width: f32,
    pub height: f32,
}

/// The native annotations and stable content identity read from one PDF.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfAnnotationSnapshot {
    pub document_fingerprint: String,
    pub pages: Vec<PdfPageIdentity>,
    pub annotations: Vec<StoredAnnotation>,
}

const MAX_PAGE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_NATIVE_ANNOTATIONS: usize = 10_000;
const MAX_ANNOTATION_TEXT_BYTES: usize = 64 * 1024;

/// A PDF annotation read back off a page.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAnnotation {
    pub page: usize,
    pub native_id: String,
    pub subtype: String,
    pub contents: String,
    pub color: String,
    pub rect: Option<PdfRect>,
    /// Whether the annotation carries `/QuadPoints`. A `/Highlight` without
    /// them is invisible in most viewers, so this is not cosmetic detail.
    pub has_quad_points: bool,
    pub fingerprint: String,
}

fn number(object: &Object) -> Option<f32> {
    match object {
        Object::Real(value) => Some(*value),
        Object::Integer(value) => Some(*value as f32),
        _ => None,
    }
}

fn rect_from(object: &Object) -> Option<PdfRect> {
    let array = object.as_array().ok()?;
    if array.len() != 4 {
        return None;
    }
    Some(PdfRect::new(
        number(&array[0])?,
        number(&array[1])?,
        number(&array[2])?,
        number(&array[3])?,
    ))
}

fn page_box(doc: &Document, page_id: ObjectId) -> Option<PdfRect> {
    let mut current = Some(page_id);
    for _ in 0..32 {
        let Some(id) = current else {
            break;
        };
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        for key in [b"CropBox".as_slice(), b"MediaBox".as_slice()] {
            if let Ok(object) = dict.get(key) {
                let object = doc
                    .dereference(object)
                    .map(|(_, value)| value)
                    .unwrap_or(object);
                if let Some(rect) = rect_from(object) {
                    return Some(rect);
                }
            }
        }
        current = dict
            .get(b"Parent")
            .ok()
            .and_then(|value| value.as_reference().ok());
    }
    None
}

fn page_rotation(doc: &Document, page_id: ObjectId) -> i32 {
    let mut current = Some(page_id);
    for _ in 0..32 {
        let Some(id) = current else {
            break;
        };
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        if let Ok(Object::Integer(rotation)) = dict.get(b"Rotate") {
            return (*rotation as i32).rem_euclid(360);
        }
        current = dict
            .get(b"Parent")
            .ok()
            .and_then(|value| value.as_reference().ok());
    }
    0
}

fn hash_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

// This identity is persisted, so its explicit FNV bytes must stay stable.
pub(crate) fn text_fingerprint(text: &str) -> String {
    hash_parts([text.as_bytes()])
}

pub(crate) fn annotation_fingerprint(
    subtype: &str,
    contents: &str,
    color: &str,
    rect: Option<PdfRect>,
) -> String {
    let rect = rect
        .map(|rect| {
            format!(
                "{:08x},{:08x},{:08x},{:08x}",
                rect.x0.to_bits(),
                rect.y0.to_bits(),
                rect.x1.to_bits(),
                rect.y1.to_bits()
            )
        })
        .unwrap_or_default();
    hash_parts([
        subtype.as_bytes(),
        contents.as_bytes(),
        color.as_bytes(),
        rect.as_bytes(),
    ])
}

fn annotation_color(dict: &lopdf::Dictionary) -> String {
    let Ok(Object::Array(values)) = dict.get(b"C") else {
        return String::new();
    };
    if values.len() != 3 {
        return String::new();
    }
    let Some(rgb) = number(&values[0])
        .zip(number(&values[1]))
        .zip(number(&values[2]))
        .map(|((r, g), b)| [r, g, b])
    else {
        return String::new();
    };
    let palette = [
        ("yellow", highlight_rgb("yellow")),
        ("orange", highlight_rgb("orange")),
        ("red", highlight_rgb("red")),
        ("magenta", highlight_rgb("magenta")),
        ("aqua", highlight_rgb("aqua")),
        ("lime", highlight_rgb("lime")),
    ];
    palette
        .into_iter()
        .min_by(|(_, left), (_, right)| {
            let distance = |candidate: &[f32; 3]| {
                (candidate[0] - rgb[0]).powi(2)
                    + (candidate[1] - rgb[1]).powi(2)
                    + (candidate[2] - rgb[2]).powi(2)
            };
            distance(left).total_cmp(&distance(right))
        })
        .map(|(name, _)| name.to_string())
        .unwrap_or_default()
}

pub(super) fn annotation_refs(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Ok(annots) = page.get(b"Annots") else {
        return Vec::new();
    };
    let annots = doc
        .dereference(annots)
        .map(|(_, object)| object)
        .unwrap_or(annots);
    annots
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_reference().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn native_id(dict: &lopdf::Dictionary, object_id: ObjectId) -> String {
    match dict.get(b"NM") {
        Ok(Object::String(bytes, _))
            if !bytes.is_empty() && bytes.len() <= MAX_ANNOTATION_TEXT_BYTES =>
        {
            decode_pdf_string(bytes)
        }
        _ => format!("object-{}-{}", object_id.0, object_id.1),
    }
}

fn stored_annotation(
    page: usize,
    object_id: ObjectId,
    dict: &lopdf::Dictionary,
) -> Result<StoredAnnotation, String> {
    let subtype = dict
        .get(b"Subtype")
        .ok()
        .and_then(|object| object.as_name().ok())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default();
    let contents = match dict.get(b"Contents") {
        Ok(Object::String(bytes, _)) if bytes.len() <= MAX_ANNOTATION_TEXT_BYTES => {
            decode_pdf_string(bytes)
        }
        Ok(Object::String(_, _)) => {
            return Err("A native PDF annotation is too large to import.".to_string());
        }
        _ => String::new(),
    };
    let rect = dict.get(b"Rect").ok().and_then(rect_from);
    let color = annotation_color(dict);
    let native_id = native_id(dict, object_id);
    let fingerprint = annotation_fingerprint(&subtype, &contents, &color, rect);
    Ok(StoredAnnotation {
        page,
        native_id,
        subtype,
        contents,
        color,
        rect,
        has_quad_points: matches!(dict.get(b"QuadPoints"), Ok(Object::Array(a)) if a.len() >= 8),
        fingerprint,
    })
}

/// Read every supported native annotation and the page-content identity used
/// to validate its sidecar projection after a PDF changes.
pub fn read_document_annotations(path: &Path) -> Result<PdfAnnotationSnapshot, String> {
    let doc = load(path)?;
    snapshot_from_document(&doc)
}

/// Parse `bytes` in memory and compute the snapshot without touching a pathname. `None`
/// when parsing fails or snapshot limits refuse. The uncertain-publication classifier
/// fingerprints retained lost-custody bytes with it.
pub fn snapshot_from_bytes(bytes: &[u8]) -> Option<PdfAnnotationSnapshot> {
    Document::load_mem(bytes)
        .ok()
        .and_then(|document| snapshot_from_document(&document).ok())
}

// Compute a snapshot from an already-loaded document, so identity, mutation and
// verification read the same in-memory bytes.
pub(crate) fn snapshot_from_document(doc: &Document) -> Result<PdfAnnotationSnapshot, String> {
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    let mut identities = Vec::with_capacity(pages.len());
    let mut annotations = Vec::new();
    for (page, page_id) in pages.values().copied().enumerate() {
        let page_box = page_box(doc, page_id).unwrap_or(PdfRect::new(0.0, 0.0, 0.0, 0.0));
        let box_key = format!(
            "{:08x},{:08x},{:08x},{:08x}",
            page_box.x0.to_bits(),
            page_box.y0.to_bits(),
            page_box.x1.to_bits(),
            page_box.y1.to_bits()
        );
        let rotation_degrees = page_rotation(doc, page_id);
        let rotation_key = rotation_degrees.to_string();
        let content = doc
            .get_page_content_with_limit(page_id, MAX_PAGE_CONTENT_BYTES)
            .map_err(|_| "A PDF page is too large to validate annotations.".to_string())?;
        let extracted_text = doc
            .extract_text_with_limit(&[(page + 1) as u32], MAX_PAGE_CONTENT_BYTES)
            .map_err(|_| "A PDF page is too large to validate annotations.".to_string())?;
        let fingerprint = hash_parts([
            box_key.as_bytes(),
            rotation_key.as_bytes(),
            content.as_slice(),
            extracted_text.as_bytes(),
        ]);
        identities.push(PdfPageIdentity {
            fingerprint,
            rotation_degrees,
            x0: page_box.x0,
            y0: page_box.y0,
            width: (page_box.x1 - page_box.x0).abs(),
            height: (page_box.y1 - page_box.y0).abs(),
        });
        for object_id in annotation_refs(doc, page_id) {
            let Ok(dict) = doc.get_dictionary(object_id) else {
                continue;
            };
            let stored = stored_annotation(page, object_id, dict)?;
            if annotations.len() >= MAX_NATIVE_ANNOTATIONS {
                return Err("The PDF contains too many annotations to import.".to_string());
            }
            annotations.push(stored);
        }
    }
    let document_fingerprint = hash_parts(
        identities
            .iter()
            .map(|identity| identity.fingerprint.as_bytes()),
    );
    Ok(PdfAnnotationSnapshot {
        document_fingerprint,
        pages: identities,
        annotations,
    })
}

/// Read the annotations on page `page_index` (0-based) of the PDF at `path`.
pub fn read_annotations(path: &Path, page_index: usize) -> Result<Vec<StoredAnnotation>, String> {
    Ok(read_document_annotations(path)?
        .annotations
        .into_iter()
        .filter(|annotation| annotation.page == page_index)
        .collect())
}

fn annotation_object_id(
    doc: &Document,
    page_id: ObjectId,
    wanted_native_id: &str,
) -> Option<ObjectId> {
    annotation_refs(doc, page_id).into_iter().find(|object_id| {
        doc.get_dictionary(*object_id)
            .map(|dict| native_id(dict, *object_id) == wanted_native_id)
            .unwrap_or(false)
    })
}

/// Update the authored contents and/or colour of one native annotation.
pub fn update_annotation(
    path: &Path,
    page_index: usize,
    wanted_native_id: &str,
    contents: Option<&str>,
    color: Option<&str>,
) -> Result<(), PdfWriteError> {
    let mut doc = load(path)?;
    update_annotation_in_document(&mut doc, page_index, wanted_native_id, contents, color)?;
    save_document(
        &mut doc,
        path,
        enzui_iced_core::user_file::SavePolicy::ReplaceExisting,
    )
}

pub(crate) fn update_annotation_in_document(
    doc: &mut Document,
    page_index: usize,
    wanted_native_id: &str,
    contents: Option<&str>,
    color: Option<&str>,
) -> Result<(), PdfWriteError> {
    let pages = doc.get_pages();
    let page_id = *pages
        .get(&((page_index + 1) as u32))
        .ok_or_else(|| format!("Page {} does not exist.", page_index + 1))?;
    let object_id = annotation_object_id(doc, page_id, wanted_native_id)
        .ok_or_else(|| "The native PDF annotation no longer exists.".to_string())?;
    let dict = doc
        .get_dictionary_mut(object_id)
        .map_err(|error| format!("Malformed annotation dictionary: {error}"))?;
    if let Some(contents) = contents {
        dict.set(
            "Contents",
            Object::String(encode_pdf_string(contents), StringFormat::Literal),
        );
    }
    if let Some(color) = color {
        let [r, g, b] = highlight_rgb(color);
        dict.set(
            "C",
            Object::Array(vec![Object::Real(r), Object::Real(g), Object::Real(b)]),
        );
    }
    Ok(())
}

/// Delete one native annotation by its stable `/NM` or object identity.
pub fn delete_annotation(
    path: &Path,
    page_index: usize,
    wanted_native_id: &str,
) -> Result<(), PdfWriteError> {
    let mut doc = load(path)?;
    delete_annotation_from_document(&mut doc, page_index, wanted_native_id)?;
    save_document(
        &mut doc,
        path,
        enzui_iced_core::user_file::SavePolicy::ReplaceExisting,
    )
}

pub(crate) fn delete_annotation_from_document(
    doc: &mut Document,
    page_index: usize,
    wanted_native_id: &str,
) -> Result<(), PdfWriteError> {
    let pages = doc.get_pages();
    let page_id = *pages
        .get(&((page_index + 1) as u32))
        .ok_or_else(|| format!("Page {} does not exist.", page_index + 1))?;
    let object_id = annotation_object_id(doc, page_id, wanted_native_id)
        .ok_or_else(|| "The native PDF annotation no longer exists.".to_string())?;

    let annots = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Annots").ok())
        .cloned()
        .ok_or_else(|| "The native PDF annotation list no longer exists.".to_string())?;
    match annots {
        Object::Reference(array_id) => {
            let array = doc
                .get_object_mut(array_id)
                .and_then(Object::as_array_mut)
                .map_err(|error| format!("Malformed annotation list: {error}"))?;
            array.retain(|object| object.as_reference().ok() != Some(object_id));
        }
        Object::Array(_) => {
            let array = doc
                .get_dictionary_mut(page_id)
                .and_then(|page| page.get_mut(b"Annots"))
                .and_then(Object::as_array_mut)
                .map_err(|error| format!("Malformed annotation list: {error}"))?;
            array.retain(|object| object.as_reference().ok() != Some(object_id));
        }
        _ => return Err("Malformed annotation list.".to_string().into()),
    }
    Ok(())
}
