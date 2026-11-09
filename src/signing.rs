//! The sign-a-PDF flow's state and its pure parts: the fonts on offer, the
//! preview, and where a click on a page lands in PDF coordinates.
//!
//! The PDF work itself is [`crate::plumbing::pdf_sign`].

use std::path::PathBuf;
use std::sync::Arc;

use crate::plumbing::pdf_sign::{self, Placement};

/// How wide a placed signature is on the page, in PDF points (2.5 inches).
pub const SIGNATURE_WIDTH_PTS: f32 = 180.0;

// The preview's text height in pixels: twice what the dialog shows, so the
// signature stays sharp when it follows the pointer over a zoomed page.
const PREVIEW_PX: f32 = 120.0;

// The rendered height for the PDF: tall enough to stay crisp when printed.
const STAMP_PX: f32 = 220.0;

/// A font the signature can be written in.
#[derive(Debug, Clone, PartialEq)]
pub struct FontChoice {
    pub family: String,
    pub source: FontSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FontSource {
    Bundled,
    File { path: PathBuf, index: u32 },
}

/// What the sign dialog and the placement step hold.
#[derive(Debug, Clone)]
pub struct SignDraft {
    pub name: String,
    pub fonts: Vec<FontChoice>,
    pub family: String,
    /// The chosen face's bytes and index, read once when it is chosen.
    font: Option<(Arc<Vec<u8>>, u32)>,
    pub preview: Option<iced::widget::image::Handle>,
    pub preview_size: (u32, u32),
    pub certificate: Option<PathBuf>,
    pub password: String,
    pub password_revealed: bool,
    pub error: Option<String>,
    /// Waiting for a click on a page.
    pub placing: bool,
    /// The pointer over a page while placing: the page and a point in the
    /// page's displayed pixels.
    pub pointer: Option<(usize, iced::Point, iced::Size)>,
    /// Where the click put it, while the destination is chosen and written.
    pub placed: Option<Placement>,
}

impl SignDraft {
    pub fn new(fonts: Vec<FontChoice>) -> Self {
        let mut draft = Self {
            name: String::new(),
            fonts,
            family: pdf_sign::BUNDLED_FAMILY.to_string(),
            font: None,
            preview: None,
            preview_size: (0, 0),
            certificate: None,
            password: String::new(),
            password_revealed: false,
            error: None,
            placing: false,
            pointer: None,
            placed: None,
        };
        draft.choose_family(pdf_sign::BUNDLED_FAMILY.to_string());
        draft
    }

    /// Switch to `family`, reading its face, and redraw the preview.
    pub fn choose_family(&mut self, family: String) {
        let Some(choice) = self.fonts.iter().find(|f| f.family == family).cloned() else {
            return;
        };
        self.font = match &choice.source {
            FontSource::Bundled => Some((Arc::new(pdf_sign::BUNDLED_FONT.to_vec()), 0)),
            FontSource::File { path, index } => std::fs::read(path)
                .ok()
                .map(|bytes| (Arc::new(bytes), *index)),
        };
        self.family = family;
        self.error = self
            .font
            .is_none()
            .then(|| "That font could not be read.".to_string());
        self.refresh_preview();
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
        self.refresh_preview();
    }

    fn refresh_preview(&mut self) {
        self.preview = None;
        self.preview_size = (0, 0);
        if let Some(image) = self.render(PREVIEW_PX) {
            self.preview_size = (image.width, image.height);
            self.preview = Some(iced::widget::image::Handle::from_rgba(
                image.width,
                image.height,
                image.rgba,
            ));
        }
    }

    fn render(&self, px: f32) -> Option<pdf_sign::SignatureImage> {
        let (bytes, index) = self.font.as_ref()?;
        pdf_sign::render_signature(&self.name, bytes, *index, px).ok()
    }

    /// The image the PDF gets.
    pub fn stamp_image(&self) -> Option<pdf_sign::SignatureImage> {
        self.render(STAMP_PX)
    }

    /// Whether the dialog can move on to placing: a name that renders.
    pub fn ready(&self) -> bool {
        self.preview.is_some()
    }
}

/// Every font the signature can use: the bundled handwriting face first,
/// then each installed handwriting or script family's regular face, by name.
/// A text face makes no signature, so the rest of the system's fonts are
/// left out. Found once per run: classifying reads a few bytes of each font.
pub fn installed_fonts() -> Vec<FontChoice> {
    static FONTS: std::sync::OnceLock<Vec<FontChoice>> = std::sync::OnceLock::new();
    FONTS.get_or_init(scan_handwriting_fonts).clone()
}

fn scan_handwriting_fonts() -> Vec<FontChoice> {
    use iced::advanced::graphics::text::cosmic_text::fontdb::{Source, Style};

    let mut fonts = vec![FontChoice {
        family: pdf_sign::BUNDLED_FAMILY.to_string(),
        source: FontSource::Bundled,
    }];
    let mut system = iced::advanced::graphics::text::font_system()
        .write()
        .expect("font system");
    let mut seen = std::collections::BTreeMap::new();
    for face in system.raw().db().faces() {
        let Some((family, _)) = face.families.first() else {
            continue;
        };
        let path = match &face.source {
            Source::File(path) | Source::SharedFile(path, _) => path.clone(),
            Source::Binary(_) => continue,
        };
        // Prefer the upright regular face of each family.
        let score =
            u32::from(face.style != Style::Normal) * 1000 + u32::from(face.weight.0.abs_diff(400));
        let entry = seen
            .entry(family.clone())
            .or_insert((u32::MAX, path.clone(), 0));
        if score < entry.0 {
            *entry = (score, path, face.index);
        }
    }
    drop(system);
    fonts.extend(
        seen.into_iter()
            .filter(|(family, _)| family != pdf_sign::BUNDLED_FAMILY)
            .filter(|(family, (_, path, index))| {
                looks_handwritten(family) || is_script_face(path, *index).unwrap_or(false)
            })
            .map(|(family, (_, path, index))| FontChoice {
                family,
                source: FontSource::File { path, index },
            }),
    );
    fonts
}

// Whether a family's name says it is handwriting, for faces that leave
// their OS/2 classification blank.
fn looks_handwritten(family: &str) -> bool {
    let family = family.to_lowercase();
    [
        "script",
        "hand",
        "brush",
        "signature",
        "calligraph",
        "cursive",
    ]
    .iter()
    .any(|hint| family.contains(hint))
}

// Whether the face at `path` (face `index` of a collection) classes itself
// as handwriting: PANOSE family "Latin Hand Written" or IBM family class
// "Scripts" in its OS/2 table. Reads the table directory and the start of
// OS/2, never the whole file.
fn is_script_face(path: &std::path::Path, index: u32) -> Option<bool> {
    use std::io::{Read, Seek, SeekFrom};
    fn read_at(file: &mut std::fs::File, offset: u64, buf: &mut [u8]) -> Option<()> {
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(buf).ok()
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0_u8; 12];
    read_at(&mut file, 0, &mut head)?;
    let mut base = 0_u64;
    if &head[..4] == b"ttcf" {
        let mut offset = [0_u8; 4];
        read_at(&mut file, 12 + 4 * u64::from(index), &mut offset)?;
        base = u64::from(u32::from_be_bytes(offset));
        read_at(&mut file, base, &mut head)?;
    }
    let tables = u16::from_be_bytes([head[4], head[5]]);
    let mut directory = vec![0_u8; usize::from(tables) * 16];
    read_at(&mut file, base + 12, &mut directory)?;
    let os2 = directory
        .as_chunks::<16>()
        .0
        .iter()
        .find(|entry| &entry[..4] == b"OS/2")?;
    let offset = u32::from_be_bytes([os2[8], os2[9], os2[10], os2[11]]);
    // sFamilyClass is at 30 (class in its high byte), PANOSE at 32.
    let mut table = [0_u8; 33];
    read_at(&mut file, u64::from(offset), &mut table)?;
    const SCRIPTS_CLASS: u8 = 10;
    const PANOSE_HAND_WRITTEN: u8 = 3;
    Some(table[30] == SCRIPTS_CLASS || table[32] == PANOSE_HAND_WRITTEN)
}

/// Where a signature centred on `point` lands in PDF coordinates, for a page
/// displayed `displayed` pixels big whose page box is `page_pts` points.
///
/// The signature keeps the preview's aspect, `SIGNATURE_WIDTH_PTS` wide, and
/// stays wholly on the page.
pub fn placement_for(
    page: usize,
    point: iced::Point,
    displayed: iced::Size,
    page_pts: (f32, f32),
    image: (u32, u32),
) -> Placement {
    let (w_pts, h_pts) = page_pts;
    let width = SIGNATURE_WIDTH_PTS.min(w_pts);
    let height = width * image.1 as f32 / image.0.max(1) as f32;
    let cx = point.x / displayed.width.max(1.0) * w_pts;
    // PDF counts up from the bottom; the screen counts down from the top.
    let cy = (1.0 - point.y / displayed.height.max(1.0)) * h_pts;
    Placement {
        page,
        x: (cx - width / 2.0).clamp(0.0, (w_pts - width).max(0.0)),
        y: (cy - height / 2.0).clamp(0.0, (h_pts - height).max(0.0)),
        width,
        height,
    }
}

/// Stamp (and, with a certificate, sign) the PDF at `source`, writing the
/// result to `destination`.
pub fn write_signed_copy(
    source: PathBuf,
    destination: PathBuf,
    image: pdf_sign::SignatureImage,
    placement: Placement,
    certificate: Option<(Vec<u8>, String)>,
) -> Result<PathBuf, String> {
    let mut doc = lopdf::Document::load(&source).map_err(|e| e.to_string())?;
    pdf_sign::stamp(&mut doc, &image, placement).map_err(|e| e.to_string())?;
    let bytes = match certificate {
        Some((der, password)) => {
            let signer =
                pdf_sign::Signer::from_pkcs12(&der, &password).map_err(|e| e.to_string())?;
            pdf_sign::sign_with_certificate(doc, &signer, placement).map_err(|e| e.to_string())?
        }
        None => {
            let mut bytes = Vec::new();
            doc.save_to(&mut bytes).map_err(|e| e.to_string())?;
            bytes
        }
    };
    std::fs::write(&destination, bytes).map_err(|e| e.to_string())?;
    Ok(destination)
}
