//! Cover thumbnails and heading cleanup.

/// A decoded, downscaled EPUB cover: straight RGBA8 plus its dimensions.
///
/// The same shape as [`crate::message::PageThumbnail`], declared here so `plumbing` names
/// no iced type; see [`cover_thumbnail`].
#[derive(Debug, Clone, PartialEq)]
pub struct CoverThumbnail {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Extract the EPUB's declared cover image as a small RGBA thumbnail.
///
/// A reflowable EPUB has no first page until a layout is chosen, so the library uses the
/// cover, as other readers do. Returns `None` for no cover, an unopenable archive, a
/// missing resource or an undecodable image; the card then paints the generic `book`
/// glyph. `target_width` is in device pixels; aspect is preserved and images never
/// scale up.
pub fn cover_thumbnail(path: &str, target_width: u32) -> Option<CoverThumbnail> {
    let mut doc = super::archive::open(path).ok()?;
    // `get_cover` resolves the EPUB3 `cover-image` item and the EPUB2 `<meta name="cover">`
    // from the already-parsed OPF: one zip entry read, no spine walk.
    let (bytes, _mime) = doc.get_cover()?;
    decode_cover_bytes(&bytes, target_width)
}

/// Decode cover image bytes to a downscaled RGBA thumbnail.
///
/// Returns `None` for malformed data or a framework decode-limit refusal.
/// The format is detected from content, including mislabelled cover resources.
pub fn decode_cover_bytes(bytes: &[u8], target_width: u32) -> Option<CoverThumbnail> {
    if bytes.is_empty() || target_width == 0 {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return None;
    }
    let decoded = enzui_resources::decode_static_resized(
        bytes,
        &enzui_iced_core::decode_limits::DecodeLimits::DEFAULT,
        target_width,
        u32::MAX,
        || false,
    )
    .ok()?;
    let iced::advanced::image::Handle::Rgba { pixels, .. } = decoded.handle else {
        return None;
    };
    Some(CoverThumbnail {
        width: decoded.width,
        height: decoded.height,
        rgba: pixels.to_vec(),
    })
}

// Normalize a heading for loose comparison: lowercase, collapsed whitespace, no
// non-alphanumerics, so "CHAPTER I." matches "Chapter I".
fn normalize_heading(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// If the first non-empty line of `plain_text` loosely matches `title`, remove it so the
/// heading is not printed twice. Matching accepts equality, a prefix either way, or two
/// shared leading words.
pub fn strip_redundant_leading_heading(plain_text: &str, title: &str) -> String {
    let title_norm = normalize_heading(title);
    if title_norm.is_empty() {
        return plain_text.to_string();
    }

    // Strip every consecutive matching leading line, including one matching the title's
    // part before a " -- " / " – " book-name suffix, since some EPUBs echo the title twice.
    let lead_norm = title_norm
        .split(" -- ")
        .next()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| title_norm.clone());

    let mut remaining = plain_text.to_string();
    loop {
        // Peek the first non-empty line.
        let mut lines = remaining.lines();
        let mut first_real: Option<String> = None;
        for line in lines.by_ref() {
            if line.trim().is_empty() {
                continue;
            }
            first_real = Some(line.to_string());
            break;
        }
        let Some(line) = first_real else { break };
        let line_norm = normalize_heading(&line);
        if line_norm.is_empty() {
            break;
        }
        let matches =
            headings_match(&line_norm, &title_norm) || headings_match(&line_norm, &lead_norm);
        if !matches {
            break;
        }
        // Drop everything up to and including this matched line.
        let cut = remaining.find(line.as_str());
        let rest = match cut {
            Some(pos) => remaining[pos + line.len()..].to_string(),
            None => break,
        };
        remaining = rest.trim_start_matches(['\n', ' ']).to_string();
    }
    remaining
}

// Whether two normalised headings are close enough to be the same: equal, one a prefix
// of the other, or sharing two leading words.
pub(crate) fn headings_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // Prefix match (either direction).
    if a.starts_with(b) || b.starts_with(a) {
        return true;
    }
    // Shared leading-word overlap ≥2 words.
    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();
    let shared = a_words
        .iter()
        .zip(b_words.iter())
        .take_while(|(x, y)| x == y)
        .count();
    shared >= 2
}
