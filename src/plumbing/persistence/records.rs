//! Annotation records and serialization counters.

use super::*;

/// Typed annotation position, encoded to/from the sidecar's free-form `position` string.
///
/// Round-trip:
/// - `PdfRect { page: 3, x: 0.1, y: 0.2, w: 0.3, h: 0.04 }` ↔ `"rect:3;0.1,0.2,0.3,0.04"`
///   (the older `"rect:x,y,w,h"` without a page is accepted; the parent annotation's
///   `page` is then authoritative.)
/// - `EpubCfi { chapter: 2, char_start: 100, char_end: 130 }` ↔ `"epubcfi:2/100-130"`
/// - `None` ↔ `""`
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationPosition {
    /// A rectangle in normalised (0..1) page coordinates.
    PdfRect {
        page: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// Reader-local chapter and plain-text character range [start, end); not a standard EPUB CFI.
    EpubCfi {
        chapter: usize,
        char_start: usize,
        char_end: usize,
    },
    /// No specific location.
    None,
}

impl AnnotationPosition {
    /// Serialise to the compact tagged string used in the sidecar.
    pub fn to_stored(&self) -> String {
        match self {
            AnnotationPosition::PdfRect { page, x, y, w, h } => {
                format!("rect:{};{:.4},{:.4},{:.4},{:.4}", page, x, y, w, h)
            }
            AnnotationPosition::EpubCfi {
                chapter,
                char_start,
                char_end,
            } => {
                format!("epubcfi:{}/{}-{}", chapter, char_start, char_end)
            }
            AnnotationPosition::None => String::new(),
        }
    }

    /// Parse a stored position string.
    /// - `rect:{page};x,y,w,h` — page + normalised rect
    /// - `rect:x,y,w,h`: older form without a page; caller supplies it
    /// - `epubcfi:{chapter}/{start}-{end}`
    /// - empty → `None`
    pub fn parse(s: &str, fallback_page: usize) -> Self {
        if s.is_empty() {
            return AnnotationPosition::None;
        }
        if let Some(rest) = s.strip_prefix("rect:") {
            let (page_str, coords_str) = if let Some((p, c)) = rest.split_once(';') {
                (Some(p), c)
            } else {
                (None, rest)
            };
            let page = match page_str {
                Some(p) => p.trim().parse::<usize>().unwrap_or(fallback_page),
                None => fallback_page,
            };
            let parts: Vec<&str> = coords_str.split(',').collect();
            if parts.len() != 4 {
                return AnnotationPosition::None;
            }
            let (x, y, w, h) = match (
                parts[0].trim().parse::<f32>(),
                parts[1].trim().parse::<f32>(),
                parts[2].trim().parse::<f32>(),
                parts[3].trim().parse::<f32>(),
            ) {
                (Ok(x), Ok(y), Ok(w), Ok(h)) => (x, y, w, h),
                _ => return AnnotationPosition::None,
            };
            return AnnotationPosition::PdfRect { page, x, y, w, h };
        }
        if let Some(rest) = s.strip_prefix("epubcfi:") {
            // {chapter}/{start}-{end}
            let (ch_str, rng_str) = match rest.split_once('/') {
                Some((a, b)) => (a, b),
                None => return AnnotationPosition::None,
            };
            let (start_str, end_str) = match rng_str.split_once('-') {
                Some((a, b)) => (a, b),
                None => return AnnotationPosition::None,
            };
            let chapter = ch_str.trim().parse::<usize>().unwrap_or(fallback_page);
            let char_start = start_str.trim().parse::<usize>().unwrap_or(0);
            let char_end = end_str.trim().parse::<usize>().unwrap_or(char_start);
            return AnnotationPosition::EpubCfi {
                chapter,
                char_start,
                char_end,
            };
        }
        AnnotationPosition::None
    }

    /// Parse only the rect coordinates, ignoring the optional page prefix. Returns `None`
    /// for non-rect positions or unparseable strings.
    pub fn parse_rect_coords(s: &str) -> Option<(f32, f32, f32, f32)> {
        match AnnotationPosition::parse(s, 0) {
            AnnotationPosition::PdfRect { x, y, w, h, .. } => Some((x, y, w, h)),
            _ => None,
        }
    }
}

/// One annotation on a page (or chapter, for EPUBs).
///
/// Sidecars without `style` default to `"highlight"` for highlights and `""` for notes;
/// a missing `note` defaults to empty. Native PDF identity is optional.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfAnnotationBinding {
    /// PDF `/NM` identifier, or a deterministic object-reference fallback for
    /// annotations created by another reader without `/NM`.
    pub native_id: String,
    /// Fingerprint of the page-tree content when this binding was established.
    pub document_fingerprint: String,
    /// Fingerprint of the owning page's content stream and page box.
    pub page_fingerprint: String,
    /// Fingerprint of subtype, contents, colour, and absolute PDF rectangle.
    pub native_fingerprint: String,
    /// Fingerprint of the selected document text for highlights. Empty for
    /// point comments and for imported highlights whose text is not loaded yet.
    #[serde(default)]
    pub text_fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Annotation {
    /// Stable id (monotonic within a sidecar).
    pub id: u64,
    /// 0-indexed page (PDF) or chapter (EPUB).
    pub page: usize,
    /// "highlight" or "note". Kept for backward compatibility; new code should
    /// look at `style` first — a highlight with `style: "underline"` is still
    /// `kind: "highlight"`.
    pub kind: String,
    /// User-authored text (for highlights: the quoted snippet; for notes: the
    /// note body itself). Preserved for backward compatibility.
    pub text: String,
    /// Optional colour hint. One of the palette values
    /// ("yellow" | "orange" | "red" | "magenta" | "aqua" | "lime") for
    /// highlights, empty for notes.
    #[serde(default)]
    pub color: String,
    /// ISO-8601 timestamp of creation.
    #[serde(default)]
    pub created: String,
    /// Free-form position hint (see [`AnnotationPosition`]). Empty for
    /// chapter/page-level notes.
    #[serde(default)]
    pub position: String,
    /// Highlight style accepts highlight, underline, strikethrough or squiggly; note records
    /// ignore it.
    #[serde(default)]
    pub style: String,
    /// Optional sticky-note body attached to a highlight, making it a highlight with a
    /// note. Empty for plain highlights and for `kind: "note"`, whose body is `text`.
    #[serde(default)]
    pub note: String,
    /// Identity tying a sidecar row to one native PDF annotation and the page
    /// content it was created against. Older EPUB/PDF/CBZ sidecars omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_binding: Option<PdfAnnotationBinding>,
    /// A PDF highlight's per-line boxes in page space (PDF user space,
    /// hundredths of a point, `[x0, y0, x1, y1]`), so re-creating it on
    /// redo marks the same words. Empty for notes and older sidecars.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quads: Vec<[i32; 4]>,
}

/// A lightweight bookmark: just a page position + optional label.
/// Separate from annotations so they can be managed, displayed, and copied
/// independently.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BookmarkRecord {
    /// 0-indexed page (PDF) or chapter (EPUB).
    pub page: usize,
    /// User-visible label. Defaults to "Page N" / "Chapter N".
    #[serde(default)]
    pub label: String,
    /// ISO-8601 creation timestamp.
    #[serde(default)]
    pub created: String,
}

/// Annotation sidecar file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnnotationsFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub next_id: u64,
    /// Bookmarks, separate from annotations; sidecars without them deserialise empty.
    #[serde(default)]
    pub bookmarks: Vec<BookmarkRecord>,
}

pub(crate) fn default_version() -> u32 {
    1
}

pub(crate) const JSON_STATE_VERSION: u32 = 1;
pub(crate) const LIBRARY_FORMAT_ID: &str = "reader-library-json";
pub(crate) const ANNOTATIONS_FORMAT_ID: &str = "reader-annotations-json";
pub(crate) const LIBRARY_RAW_BYTE_LIMIT: RawByteLimit = match RawByteLimit::for_body(
    LIBRARY_FORMAT_ID,
    JSON_STATE_VERSION,
    bounded_json::MAX_LIBRARY_JSON_BYTES,
) {
    Ok(limit) => limit,
    Err(_) => panic!("Reader library byte limit exceeds the framework ceiling"),
};
pub(crate) const ANNOTATIONS_RAW_BYTE_LIMIT: RawByteLimit = match RawByteLimit::for_body(
    ANNOTATIONS_FORMAT_ID,
    JSON_STATE_VERSION,
    bounded_json::MAX_ANNOTATION_JSON_BYTES,
) {
    Ok(limit) => limit,
    Err(_) => panic!("Reader annotation byte limit exceeds the framework ceiling"),
};
