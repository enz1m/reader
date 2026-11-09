//! Blocking PDF rasterisation and text extraction through Poppler and Cairo.
//!
//! Call on a blocking worker. Raster pixels are straight RGBA; selectable glyphs
//! use top-left page coordinates in points.

use crate::message::{PageBitmap, PageThumbnail};
use crate::raster::PageExtent;

// Target pixel width for each page raster.
const RENDER_WIDTH: f64 = 1000.0;
const INITIAL_BAND_PAGES: usize = 4;

/// Target width in device pixels for a "Continue reading" thumbnail: about 2× the 56×72
/// tile for HiDPI crispness.
pub const THUMBNAIL_WIDTH: f64 = 112.0;
pub const THUMBNAIL_MAX_HEIGHT: f64 = 160.0;

// Poppler documents hold this lock for their lifetime to protect process-wide LCMS state.
static POPPLER: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct PopplerGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn poppler_lock() -> PopplerGuard {
    let lock = POPPLER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering;
        let in_flight = POPPLER_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
        POPPLER_MAX_IN_FLIGHT.fetch_max(in_flight, Ordering::SeqCst);
    }
    PopplerGuard { _lock: lock }
}

impl Drop for PopplerGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        POPPLER_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

// Poppler sections running now, and the most ever seen at once in this
// process, read by the serialisation gate.
#[cfg(test)]
static POPPLER_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static POPPLER_MAX_IN_FLIGHT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Paint `page` with its annotations except sticky notes and their popups.
//
// Poppler draws a sticky note as an icon over its words; the reading view draws its own
// clickable margin marker instead.
fn render_page(page: &poppler::Page, cairo: &cairo::Context) {
    let annotations = poppler::RenderAnnotsFlags::ALL
        - poppler::RenderAnnotsFlags::TEXT
        - poppler::RenderAnnotsFlags::POPUP;
    page.render_full(cairo, false, annotations);
}

/// Render the FIRST page of a PDF to a small RGBA thumbnail.
///
/// Returns `None` on any failure, so the card keeps its generic glyph. Only page 0 is
/// rendered, at [`THUMBNAIL_WIDTH`].
pub fn render_first_page_thumbnail(path: &str) -> Option<PageThumbnail> {
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let doc = poppler::Document::from_file(&uri, None).ok()?;
    if doc.n_pages() < 1 {
        return None;
    }
    let page = doc.page(0)?;
    let (w_pts, h_pts) = page.size();
    if w_pts <= 0.0 || h_pts <= 0.0 {
        return None;
    }
    let scale = (THUMBNAIL_WIDTH / w_pts).min(THUMBNAIL_MAX_HEIGHT / h_pts);
    let pw = (w_pts * scale).round().max(1.0) as i32;
    let ph = (h_pts * scale).round().max(1.0) as i32;

    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, pw, ph).ok()?;
    {
        let ctx = cairo::Context::new(&surface).ok()?;
        // Poppler renders transparent otherwise.
        ctx.set_source_rgb(1.0, 1.0, 1.0);
        let _ = ctx.paint();
        ctx.scale(scale, scale);
        render_page(&page, &ctx);
    }
    let stride = surface.stride() as usize;
    let width = surface.width() as usize;
    let height = surface.height() as usize;
    let rgba = surface
        .data()
        .ok()
        .map(|data| argb32_to_rgba(&data, stride, width, height))?;

    Some(PageThumbnail {
        rgba,
        width: width as u32,
        height: height as u32,
    })
}

pub fn render_page_thumbnails(
    path: &str,
    password: Option<&str>,
    indices: &[usize],
) -> Result<Vec<PageBitmap>, String> {
    if indices.len() > crate::raster::THUMBNAIL_BATCH_LIMIT {
        return Err("Too many PDF thumbnails were requested at once.".to_string());
    }
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let document = poppler::Document::from_file(&uri, password)
        .map_err(|error| open_error_summary("render_page_thumbnails.open", &error))?;
    let page_count = document.n_pages().max(0) as usize;
    let mut thumbnails = Vec::with_capacity(indices.len());
    for &index in indices {
        if index >= page_count {
            continue;
        }
        let Some(page) = document.page(index as i32) else {
            continue;
        };
        let (width_points, height_points) = page.size();
        if width_points <= 0.0 || height_points <= 0.0 {
            continue;
        }
        let scale = (THUMBNAIL_WIDTH / width_points).min(THUMBNAIL_MAX_HEIGHT / height_points);
        let width = (width_points * scale).round().max(1.0) as i32;
        let height = (height_points * scale).round().max(1.0) as i32;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
            .map_err(|error| backend_error_summary("render_page_thumbnails.surface", &error))?;
        {
            let context = cairo::Context::new(&surface)
                .map_err(|error| backend_error_summary("render_page_thumbnails.context", &error))?;
            context.set_source_rgb(1.0, 1.0, 1.0);
            let _ = context.paint();
            context.scale(scale, scale);
            render_page(&page, &context);
        }
        let stride = surface.stride() as usize;
        let pixel_width = surface.width() as usize;
        let pixel_height = surface.height() as usize;
        let rgba = surface
            .data()
            .map_err(|error| backend_error_summary("render_page_thumbnails.data", &error))
            .map(|data| argb32_to_rgba(&data, stride, pixel_width, pixel_height))?;
        thumbnails.push(PageBitmap {
            index,
            rgba,
            width: pixel_width as u32,
            height: pixel_height as u32,
        });
    }
    Ok(thumbnails)
}

/// The result of a PDF render pass: the bitmaps of the pages asked for, plus the geometry
/// of every page. `extents` is `total_pages` long, so the pane can scroll to a page whose
/// raster has not arrived.
pub struct PdfRender {
    pub pages: Vec<PageBitmap>,
    pub total_pages: usize,
    /// The on-screen extent of every page in raster device pixels, in document order, so
    /// a page's scroll offset does not move when it rasterises.
    pub extents: Vec<PageExtent>,
    /// The selectable text of each rendered page, parallel to `pages`; a drag picks lines
    /// from it for the annotation's `/Rect` and `/Contents`.
    pub text: Vec<PdfPageText>,
}

/// One line of selectable text on a PDF page, with poppler's rectangle in PDF user space
/// (points, origin bottom-left), the space an annotation's `/Rect` uses.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfTextLine {
    pub text: String,
    /// `[x0, y0, x1, y1]`, normalised so `x0 <= x1` and `y0 <= y1`.
    pub rect: [f32; 4],
}

/// The selectable text of a single page, plus the page's own size in points.
///
/// The size maps a bottom-left PDF rectangle onto a top-left widget box; deriving it from
/// the raster would bake in the render scale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfPageText {
    pub width_pts: f32,
    pub height_pts: f32,
    pub lines: Vec<PdfTextLine>,
    /// Every character with its own box, in reading order, in points with a top-left
    /// origin, so a selection can start mid-word. Filled for rendered pages only.
    pub glyphs: enzui_iced_core::glyph_selection::GlyphText,
}

impl PdfPageText {
    /// Replace this page's text with `text`, keeping a glyph layer already
    /// here when `text` has none (the whole-document text pass carries no
    /// boxes), and synthesizing one from the lines when neither does.
    pub fn absorb(&mut self, text: PdfPageText) {
        self.absorb_search_text(text);
        if self.glyphs.is_empty() {
            self.glyphs = self.selectable_glyphs().into_owned();
        }
    }

    /// Merge scanned text without expanding unseen pages into per-character boxes.
    ///
    /// Existing rendered glyphs remain authoritative when the scan supplies no boxes.
    pub fn absorb_search_text(&mut self, mut text: PdfPageText) {
        if text.glyphs.is_empty() {
            text.glyphs = std::mem::take(&mut self.glyphs);
        }
        *self = text;
    }

    /// The page's selectable glyphs: poppler's boxes when available, otherwise each line's
    /// characters spread evenly across its rectangle.
    pub fn selectable_glyphs(
        &self,
    ) -> std::borrow::Cow<'_, enzui_iced_core::glyph_selection::GlyphText> {
        use enzui_iced_core::glyph_selection::{Glyph, GlyphText};
        if !self.glyphs.is_empty() {
            return std::borrow::Cow::Borrowed(&self.glyphs);
        }
        let mut glyphs = Vec::new();
        for (line, text) in self.lines.iter().enumerate() {
            let [x0, y0, x1, y1] = text.rect;
            let count = text.text.chars().count().max(1) as f32;
            let advance = (x1 - x0) / count;
            let top = self.height_pts - y1;
            for (i, ch) in text.text.chars().enumerate() {
                glyphs.push(Glyph::new(
                    ch,
                    [x0 + advance * i as f32, top, advance, y1 - y0],
                    line as u32,
                ));
            }
        }
        std::borrow::Cow::Owned(GlyphText::new(glyphs))
    }
}

// How many characters of one page are selectable, so OCR noise cannot make the text
// layer cost more than the raster.
const MAX_PAGE_GLYPHS: usize = 50_000;

/// The page's characters and their boxes, from poppler's text layout.
///
/// `poppler_page_get_text_layout` reports one rectangle per character of
/// `poppler_page_get_text`, newlines included, in points with a top-left origin. Newlines
/// become line breaks and are not selectable.
#[allow(unsafe_code)] // reviewed poppler_page_get_text_layout FFI (inventoried).
fn page_glyphs(page: &poppler::Page) -> enzui_iced_core::glyph_selection::GlyphText {
    use enzui_iced_core::glyph_selection::{Glyph, GlyphText};
    use glib::translate::ToGlibPtr;
    let Some(text) = page.text() else {
        return GlyphText::default();
    };
    let mut rects: *mut poppler::ffi::PopplerRectangle = std::ptr::null_mut();
    let mut count: std::os::raw::c_uint = 0;
    // SAFETY: `page` is a live PopplerPage for the call; on success poppler
    // returns an owned array of `count` rectangles, freed after copying.
    let ok = unsafe {
        poppler::ffi::poppler_page_get_text_layout(page.to_glib_none().0, &mut rects, &mut count)
    };
    if ok == glib::ffi::GFALSE || rects.is_null() {
        return GlyphText::default();
    }
    // SAFETY: poppler returned `count` initialised rectangles at `rects`.
    let boxes = unsafe { std::slice::from_raw_parts(rects, count as usize) };
    let mut glyphs = Vec::new();
    let mut line = 0u32;
    for (ch, r) in text.chars().zip(boxes) {
        if ch == '\n' {
            line = line.saturating_add(1);
            continue;
        }
        if glyphs.len() >= MAX_PAGE_GLYPHS {
            break;
        }
        let (x0, x1) = (r.x1.min(r.x2), r.x1.max(r.x2));
        let (y0, y1) = (r.y1.min(r.y2), r.y1.max(r.y2));
        glyphs.push(Glyph::new(
            ch,
            [x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32],
            line,
        ));
    }
    // SAFETY: the array was allocated by poppler with g_malloc and is ours.
    unsafe { glib::ffi::g_free(rects.cast()) };
    GlyphText::new(glyphs)
}

// How many text lines are collected per page.
//
// Each line costs one `poppler_page_find_text` pass, so a noisy page cannot make opening
// a search storm; longer pages are selectable down to this many lines.
const MAX_TEXT_LINES: usize = 80;
const MAX_TEXT_LINE_CHARS: usize = 4_096;
pub const MAX_PDF_SEARCH_PAGES: usize = 4_096;
pub const MAX_PDF_SEARCH_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct PdfSearchText {
    pub pages: Vec<PdfPageText>,
    pub page_count: usize,
    pub truncated: bool,
}

// Collect the selectable text lines of `page`.
//
// Pairs poppler's page text with `find_text` rectangles. A line poppler cannot locate is
// skipped rather than given a guessed rectangle.
fn page_text_lines(page: &poppler::Page) -> PdfPageText {
    let (w_pts, h_pts) = page.size();
    let mut out = PdfPageText {
        width_pts: w_pts as f32,
        height_pts: h_pts as f32,
        ..PdfPageText::default()
    };
    let Some(text) = page.text() else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let bounded_line = line.chars().take(MAX_TEXT_LINE_CHARS).collect::<String>();
        if bounded_line.is_empty() {
            continue;
        }
        if out.lines.len() >= MAX_TEXT_LINES {
            break;
        }
        let Some(rect) = page.find_text(&bounded_line).into_iter().next() else {
            continue;
        };
        let (x0, x1) = (rect.x1().min(rect.x2()), rect.x1().max(rect.x2()));
        let (y0, y1) = (rect.y1().min(rect.y2()), rect.y1().max(rect.y2()));
        out.lines.push(PdfTextLine {
            text: bounded_line,
            rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
        });
    }
    out
}

/// The raster size in device pixels a page of `w_pts × h_pts` produces.
///
/// The one place the render scale is applied, so [`page_extents`] and [`render_pages`]
/// cannot disagree by a rounded pixel.
pub fn raster_size(w_pts: f64, h_pts: f64) -> PageExtent {
    let scale = RENDER_WIDTH / w_pts;
    PageExtent {
        width: (w_pts * scale).round().max(1.0) as u32,
        height: (h_pts * scale).round().max(1.0) as u32,
    }
}

/// The extent of every page of `path`, without rasterising any of them.
///
/// Poppler parses the page tree and reports sizes; nothing is rasterised. A page poppler
/// cannot produce, or one with a degenerate size, falls back to US Letter, since a
/// zero-height page would shift every offset below it.
pub fn page_extents(path: &str, password: Option<&str>) -> Result<Vec<PageExtent>, String> {
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let doc = poppler::Document::from_file(&uri, password)
        .map_err(|error| open_error_summary("page_extents.open", &error))?;
    let n = doc.n_pages().max(0) as usize;
    Ok((0..n).map(|i| extent_of(&doc, i)).collect())
}

// One page's extent, with the degenerate-page fallback in a single place.
fn extent_of(doc: &poppler::Document, index: usize) -> PageExtent {
    const LETTER: (f64, f64) = (612.0, 792.0);
    let (w, h) = doc
        .page(index as i32)
        .map(|p| p.size())
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .unwrap_or(LETTER);
    raster_size(w, h)
}

/// The selectable text of every page of `path`.
///
/// A re-render after a password unlock takes the same route as the first open.
pub fn searchable_page_text(path: &str, password: Option<&str>) -> Result<PdfSearchText, String> {
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let doc = poppler::Document::from_file(&uri, password)
        .map_err(|error| open_error_summary("searchable_page_text.open", &error))?;
    let page_count = doc.n_pages().max(0) as usize;
    let retained = page_count.min(MAX_PDF_SEARCH_PAGES);
    let mut pages = Vec::with_capacity(retained);
    let mut bytes = 0usize;
    let mut truncated = retained < page_count;
    for index in 0..retained {
        let mut text = doc
            .page(index as i32)
            .map(|page| page_text_lines(&page))
            .unwrap_or_default();
        let mut budget_exhausted = false;
        text.lines.retain(|line| {
            let next = bytes.saturating_add(line.text.len());
            if next > MAX_PDF_SEARCH_TEXT_BYTES {
                truncated = true;
                budget_exhausted = true;
                false
            } else {
                bytes = next;
                true
            }
        });
        pages.push(text);
        if budget_exhausted || bytes >= MAX_PDF_SEARCH_TEXT_BYTES {
            truncated = index + 1 < page_count || truncated;
            break;
        }
    }
    Ok(PdfSearchText {
        pages,
        page_count,
        truncated,
    })
}

pub fn page_text(path: &str, password: Option<&str>) -> Vec<PdfPageText> {
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let Ok(doc) = poppler::Document::from_file(&uri, password) else {
        return Vec::new();
    };
    let n = doc.n_pages().max(0) as usize;
    (0..n)
        .map(|i| {
            doc.page(i as i32)
                .map(|p| page_text_lines(&p))
                .unwrap_or_default()
        })
        .collect()
}

/// Open `path` and rasterise only the first band.
///
/// Returns every page's geometry plus the first `INITIAL_BAND_PAGES` rasters; later pages
/// arrive through [`render_pages`]. Encrypted PDFs without `password` return a classified
/// error.
pub fn render_pdf(path: &str, password: Option<&str>) -> Result<PdfRender, String> {
    let indices: Vec<usize> = (0..INITIAL_BAND_PAGES).collect();
    render_pages(path, password, &indices)
}

/// Rasterise exactly the pages in `indices` (0-based, out-of-range ignored).
///
/// Each page renders through poppler→cairo at `RENDER_WIDTH` px with its text collected in
/// the same pass. Every band also carries the page count and all extents, so a document
/// that changed cannot desynchronise the column.
///
/// # Blocking
///
/// Rendering blocks on Poppler and Cairo and must run off the UI thread.
pub fn render_pages(
    path: &str,
    password: Option<&str>,
    indices: &[usize],
) -> Result<PdfRender, String> {
    let uri = path_to_uri(path);
    let _poppler = poppler_lock();
    let doc = poppler::Document::from_file(&uri, password)
        .map_err(|error| open_error_summary("render_pages.open", &error))?;
    let n = doc.n_pages().max(0) as usize;
    let extents: Vec<PageExtent> = (0..n).map(|i| extent_of(&doc, i)).collect();

    let mut out = Vec::with_capacity(indices.len());
    let mut text = Vec::with_capacity(indices.len());
    for &i in indices {
        if i >= n {
            continue;
        }
        let Some(page) = doc.page(i as i32) else {
            continue;
        };
        let (w_pts, h_pts) = page.size();
        if w_pts <= 0.0 || h_pts <= 0.0 {
            continue;
        }
        // Collected with the raster so `pages[i]` and `text[i]` describe the same page.
        let mut page_text = page_text_lines(&page);
        page_text.glyphs = page_glyphs(&page);
        text.push(page_text);
        let scale = RENDER_WIDTH / w_pts;
        let extent = raster_size(w_pts, h_pts);
        let (pw, ph) = (extent.width as i32, extent.height as i32);

        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, pw, ph)
            .map_err(|error| backend_error_summary("render_pages.surface", &error))?;
        {
            let ctx = cairo::Context::new(&surface)
                .map_err(|error| backend_error_summary("render_pages.context", &error))?;
            // Poppler renders transparent otherwise.
            ctx.set_source_rgb(1.0, 1.0, 1.0);
            let _ = ctx.paint();
            ctx.scale(scale, scale);
            render_page(&page, &ctx);
        }
        let stride = surface.stride() as usize;
        let width = surface.width() as usize;
        let height = surface.height() as usize;
        let rgba = surface
            .data()
            .map_err(|error| backend_error_summary("render_pages.data", &error))
            .map(|data| argb32_to_rgba(&data, stride, width, height))?;

        out.push(PageBitmap {
            index: i,
            rgba,
            width: width as u32,
            height: height as u32,
        });
    }
    Ok(PdfRender {
        pages: out,
        total_pages: n,
        extents,
        text,
    })
}

// Convert cairo ARGB32 (premultiplied, native-endian → BGRA on LE) to straight
// RGBA, dropping row padding from `stride`.
fn argb32_to_rgba(data: &[u8], stride: usize, width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let row = &data[y * stride..y * stride + width * 4];
        for x in 0..width {
            let px = &row[x * 4..x * 4 + 4];
            let b = px[0];
            let g = px[1];
            let r = px[2];
            let a = px[3];
            let o = (y * width + x) * 4;
            // Un-premultiply so iced's straight-alpha compositing matches.
            if a == 0 {
                out[o] = 0;
                out[o + 1] = 0;
                out[o + 2] = 0;
                out[o + 3] = 0;
            } else if a == 255 {
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
                out[o + 3] = 255;
            } else {
                let af = a as f32 / 255.0;
                out[o] = (r as f32 / af).round().min(255.0) as u8;
                out[o + 1] = (g as f32 / af).round().min(255.0) as u8;
                out[o + 2] = (b as f32 / af).round().min(255.0) as u8;
                out[o + 3] = a;
            }
        }
    }
    out
}

// poppler wants a `file://` URI, not a bare path.
fn path_to_uri(path: &str) -> String {
    if path.starts_with("file://") {
        return path.to_string();
    }
    // GIO builds the URI from an absolute path, encoding every reserved character (`#`
    // would otherwise become a fragment).
    let abs = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string());
    match glib::filename_to_uri(&abs, None) {
        Ok(uri) => uri.to_string(),
        // Fallback if GIO refuses: space-only encoding.
        Err(_) => format!("file://{}", abs.replace(' ', "%20")),
    }
}

// The kind of failure a PDF open produced, resolved from the typed
// poppler / GLib error rather than from the C library's raw message.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum PdfOpenErrorKind {
    /// Encrypted document with no (or a wrong) password.
    Password,
    /// No file exists at the target path, or a path component is not a directory.
    NotFound,
    /// The file exists but the process cannot read it.
    Permission,
    /// The PDF structure or catalog is damaged.
    Corrupt,
    /// The input is not a supported PDF.
    Unsupported,
    /// A PDF backend or cairo rendering operation failed.
    Backend,
    /// Any other typed or untyped error.
    Unknown,
}

impl PdfOpenErrorKind {
    // A bounded, path-free, control-free user-facing summary.
    pub(crate) const fn user_summary(self) -> &'static str {
        match self {
            Self::Password => PASSWORD_PROTECTED,
            Self::NotFound => "Couldn\u{2019}t find the PDF file.",
            Self::Permission => "Couldn\u{2019}t open the PDF: permission denied.",
            Self::Corrupt => "The PDF is damaged or unreadable.",
            Self::Unsupported => "This file isn\u{2019}t a supported PDF.",
            Self::Backend => "Couldn\u{2019}t render the PDF.",
            Self::Unknown => "Couldn\u{2019}t open the PDF.",
        }
    }
}

// Classify a domain-tagged error returned by `poppler::Document::from_file`.
//
// The raw message is consulted only as a defensive password fallback after
// both known domains miss. It never becomes user-facing prose.
pub(crate) fn classify_open_error(err: &glib::Error) -> PdfOpenErrorKind {
    if let Some(kind) = err.kind::<poppler::Error>() {
        return match kind {
            poppler::Error::Encrypted => PdfOpenErrorKind::Password,
            poppler::Error::Damaged | poppler::Error::BadCatalog => PdfOpenErrorKind::Corrupt,
            poppler::Error::Invalid => PdfOpenErrorKind::Unsupported,
            // Poppler documents OpenFile as a write failure, so here it is a backend error.
            poppler::Error::OpenFile => PdfOpenErrorKind::Backend,
            _ => PdfOpenErrorKind::Unknown,
        };
    }
    if let Some(kind) = err.kind::<glib::FileError>() {
        return match kind {
            glib::FileError::Noent | glib::FileError::Notdir => PdfOpenErrorKind::NotFound,
            glib::FileError::Acces | glib::FileError::Perm => PdfOpenErrorKind::Permission,
            _ => PdfOpenErrorKind::Unknown,
        };
    }

    let lower = err.message().to_lowercase();
    if lower.contains("encrypted") || lower.contains("password") {
        return PdfOpenErrorKind::Password;
    }
    PdfOpenErrorKind::Unknown
}

// Emit raw backend detail through the single diagnostic sink.
//
// Debug formatting escapes line breaks, terminal controls, and bidi payloads
// into one physical log record. Password errors never call this function.
fn log_pdf_diagnostic(boundary: &'static str, diagnostic: &str) {
    eprintln!("[READER_PDF] boundary={boundary} diagnostic={diagnostic:?}");
}

// Translate a Poppler open failure while retaining non-password diagnostics.
pub(crate) fn open_error_summary(boundary: &'static str, error: &glib::Error) -> String {
    let kind = classify_open_error(error);
    if kind != PdfOpenErrorKind::Password {
        log_pdf_diagnostic(boundary, error.message());
    }
    kind.user_summary().to_string()
}

// Translate a cairo failure while retaining its escaped diagnostic.
fn backend_error_summary(boundary: &'static str, error: &impl std::fmt::Display) -> String {
    log_pdf_diagnostic(boundary, &error.to_string());
    PdfOpenErrorKind::Backend.user_summary().to_string()
}

/// Password-error sentinel used by [`is_password_error`] to request secure unlock.
pub const PASSWORD_PROTECTED: &str = "This PDF is password-protected.";

/// Whether an open error means the document is locked, which raises a password prompt
/// rather than an error banner.
pub fn is_password_error(message: &str) -> bool {
    message == PASSWORD_PROTECTED
}

#[cfg(test)]
#[path = "pdf/tests.rs"]
mod tests;
