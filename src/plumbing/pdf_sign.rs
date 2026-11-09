//! Typed-name image stamping and detached PKCS #7 signing with PKCS #12 certificates.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

/// Great Vibes, bundled under the SIL Open Font License (see
/// `assets/fonts/OFL.txt`): a handwriting face every install has.
pub const BUNDLED_FONT: &[u8] = include_bytes!("../../assets/fonts/GreatVibes-Regular.ttf");

/// The bundled font's family name.
pub const BUNDLED_FAMILY: &str = "Great Vibes";

// Ink for the written signature: a dark blue, like a pen.
const INK: [u8; 3] = [0x1b, 0x2a, 0x6b];

// Bytes reserved for the certificate signature. A PKCS #7 signature with a
// short certificate chain fits comfortably; a larger one is refused rather
// than written past its space.
const SIGNATURE_SPACE: usize = 16 * 1024;

// Width of each reserved `/ByteRange` number.
const RANGE_DIGITS: usize = 10;

#[derive(Debug)]
pub enum SignError {
    Font,
    EmptyName,
    Pdf(String),
    NoPage(usize),
    Certificate,
    Signing(String),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Font => write!(f, "The font could not be read."),
            Self::EmptyName => write!(f, "There is no name to write."),
            Self::Pdf(e) => write!(f, "The PDF could not be read: {e}"),
            Self::NoPage(page) => write!(f, "Page {} does not exist.", page + 1),
            Self::Certificate => write!(
                f,
                "The certificate could not be opened; check the file and its password."
            ),
            Self::Signing(e) => write!(f, "The signature could not be made: {e}"),
        }
    }
}

impl std::error::Error for SignError {}

/// A rendered signature: straight (not premultiplied) RGBA, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Draw `name` with face `index` of the font in `font_bytes`, `px` pixels
/// tall, trimmed to its ink with a small margin.
pub fn render_signature(
    name: &str,
    font_bytes: &[u8],
    index: u32,
    px: f32,
) -> Result<SignatureImage, SignError> {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};

    let name = name.trim();
    if name.is_empty() {
        return Err(SignError::EmptyName);
    }
    let font = FontRef::try_from_slice_and_index(font_bytes, index).map_err(|_| SignError::Font)?;
    let scaled = font.as_scaled(PxScale::from(px));

    // Lay the glyphs out on one baseline.
    let mut caret = 0.0f32;
    let mut previous = None;
    let mut outlined = Vec::new();
    for ch in name.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev) = previous {
            caret += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(px), point(caret, scaled.ascent()));
        caret += scaled.h_advance(id);
        previous = Some(id);
        if let Some(outline) = font.outline_glyph(glyph) {
            outlined.push(outline);
        }
    }
    if outlined.is_empty() {
        return Err(SignError::EmptyName);
    }

    // Script faces swash past their advance box, so the canvas is the union
    // of the inked boxes, not the advances.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for g in &outlined {
        let b = g.px_bounds();
        x0 = x0.min(b.min.x);
        y0 = y0.min(b.min.y);
        x1 = x1.max(b.max.x);
        y1 = y1.max(b.max.y);
    }
    let margin = (px * 0.08).ceil();
    let width = ((x1 - x0) + 2.0 * margin).ceil().max(1.0) as u32;
    let height = ((y1 - y0) + 2.0 * margin).ceil().max(1.0) as u32;
    let mut alpha = vec![0f32; (width * height) as usize];
    for g in &outlined {
        let b = g.px_bounds();
        let ox = (b.min.x - x0 + margin) as i64;
        let oy = (b.min.y - y0 + margin) as i64;
        g.draw(|x, y, coverage| {
            let (px_x, px_y) = (ox + i64::from(x), oy + i64::from(y));
            if px_x >= 0 && px_y >= 0 && (px_x as u32) < width && (px_y as u32) < height {
                let i = (px_y as u32 * width + px_x as u32) as usize;
                alpha[i] = (alpha[i] + coverage).min(1.0);
            }
        });
    }
    let mut rgba = Vec::with_capacity(alpha.len() * 4);
    for a in alpha {
        rgba.extend_from_slice(&INK);
        rgba.push((a * 255.0).round() as u8);
    }
    Ok(SignatureImage {
        width,
        height,
        rgba,
    })
}

/// Where a signature goes: a 0-based page and a rectangle in PDF points with
/// the origin at the page's bottom-left, as PDF counts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub page: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Paint `image` onto the page at `placement`.
///
/// The page's own content is wrapped in `q`/`Q`, so whatever state it leaves
/// behind cannot move or tint the signature drawn after it.
pub fn stamp(
    doc: &mut Document,
    image: &SignatureImage,
    placement: Placement,
) -> Result<(), SignError> {
    let page_id = page_id(doc, placement.page)?;

    let (rgb, alpha): (Vec<u8>, Vec<u8>) = {
        let mut rgb = Vec::with_capacity(image.rgba.len() / 4 * 3);
        let mut alpha = Vec::with_capacity(image.rgba.len() / 4);
        for px in image.rgba.as_chunks::<4>().0 {
            rgb.extend_from_slice(&px[..3]);
            alpha.push(px[3]);
        }
        (rgb, alpha)
    };
    let mut mask = Stream::new(image_dict(image, b"DeviceGray"), alpha);
    let _ = mask.compress();
    let mask_id = doc.add_object(mask);
    let mut picture_dict = image_dict(image, b"DeviceRGB");
    picture_dict.set("SMask", Object::Reference(mask_id));
    let mut picture = Stream::new(picture_dict, rgb);
    let _ = picture.compress();
    let picture_id = doc.add_object(picture);
    let name = format!("Sig{}", picture_id.0);
    doc.add_xobject(page_id, name.as_bytes(), picture_id)
        .map_err(|e| SignError::Pdf(e.to_string()))?;

    let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let draw = format!(
        "Q\nq\n{} 0 0 {} {} {} cm\n/{} Do\nQ\n",
        placement.width, placement.height, placement.x, placement.y, name
    );
    let close = doc.add_object(Stream::new(Dictionary::new(), draw.into_bytes()));

    let page = doc
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| SignError::Pdf(e.to_string()))?;
    let mut contents: Vec<Object> = match page.get(b"Contents") {
        Ok(Object::Array(items)) => items.clone(),
        Ok(other) => vec![other.clone()],
        Err(_) => Vec::new(),
    };
    contents.insert(0, Object::Reference(open));
    contents.push(Object::Reference(close));
    page.set("Contents", Object::Array(contents));
    Ok(())
}

fn image_dict(image: &SignatureImage, color_space: &[u8]) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    dict.set("Width", i64::from(image.width));
    dict.set("Height", i64::from(image.height));
    dict.set("ColorSpace", Object::Name(color_space.to_vec()));
    dict.set("BitsPerComponent", 8);
    dict
}

fn page_id(doc: &Document, page: usize) -> Result<ObjectId, SignError> {
    doc.get_pages()
        .get(&(page as u32 + 1))
        .copied()
        .ok_or(SignError::NoPage(page))
}

/// Who signs, from a PKCS #12 (`.p12` / `.pfx`) file.
pub struct Signer {
    key: openssl::pkey::PKey<openssl::pkey::Private>,
    cert: openssl::x509::X509,
    chain: openssl::stack::Stack<openssl::x509::X509>,
}

impl Signer {
    /// Open a PKCS #12 file with its password.
    pub fn from_pkcs12(der: &[u8], password: &str) -> Result<Self, SignError> {
        let parsed = openssl::pkcs12::Pkcs12::from_der(der)
            .and_then(|p| p.parse2(password))
            .map_err(|_| SignError::Certificate)?;
        let key = parsed.pkey.ok_or(SignError::Certificate)?;
        let cert = parsed.cert.ok_or(SignError::Certificate)?;
        let mut chain = openssl::stack::Stack::new().map_err(|_| SignError::Certificate)?;
        for ca in parsed.ca.into_iter().flatten() {
            chain.push(ca).map_err(|_| SignError::Certificate)?;
        }
        Ok(Self { key, cert, chain })
    }

    /// The certificate's common name, for the signature's `/Name` and the UI.
    pub fn name(&self) -> String {
        self.cert
            .subject_name()
            .entries_by_nid(openssl::nid::Nid::COMMONNAME)
            .next()
            .and_then(|e| e.data().to_string().ok())
            .unwrap_or_else(|| "Signer".to_string())
    }
}

/// Write `doc` with a certificate signature over the whole file, its visible
/// field at `placement`. Returns the signed file's bytes.
pub fn sign_with_certificate(
    mut doc: Document,
    signer: &Signer,
    placement: Placement,
) -> Result<Vec<u8>, SignError> {
    let page_id = page_id(&doc, placement.page)?;

    let mut value = Dictionary::new();
    value.set("Type", Object::Name(b"Sig".to_vec()));
    value.set("Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    value.set("SubFilter", Object::Name(b"adbe.pkcs7.detached".to_vec()));
    // Placeholders, filled once the file's bytes are known.
    let wide = 10i64.pow(RANGE_DIGITS as u32) - 1;
    value.set(
        "ByteRange",
        Object::Array(vec![0.into(), wide.into(), wide.into(), wide.into()]),
    );
    value.set(
        "Contents",
        Object::String(vec![0u8; SIGNATURE_SPACE], StringFormat::Hexadecimal),
    );
    value.set(
        "Name",
        Object::String(signer.name().into_bytes(), StringFormat::Literal),
    );
    value.set(
        "M",
        Object::String(pdf_date_now().into_bytes(), StringFormat::Literal),
    );
    let value_id = doc.add_object(value);

    let mut appearance = Stream::new(
        {
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"XObject".to_vec()));
            d.set("Subtype", Object::Name(b"Form".to_vec()));
            d.set(
                "BBox",
                Object::Array(vec![
                    0.into(),
                    0.into(),
                    placement.width.into(),
                    placement.height.into(),
                ]),
            );
            d
        },
        Vec::new(),
    );
    appearance.allows_compression = false;
    let appearance_id = doc.add_object(appearance);

    let mut widget = Dictionary::new();
    widget.set("Type", Object::Name(b"Annot".to_vec()));
    widget.set("Subtype", Object::Name(b"Widget".to_vec()));
    widget.set("FT", Object::Name(b"Sig".to_vec()));
    widget.set(
        "T",
        Object::String(b"Signature1".to_vec(), StringFormat::Literal),
    );
    widget.set("V", Object::Reference(value_id));
    widget.set("F", 4);
    widget.set("P", Object::Reference(page_id));
    widget.set(
        "Rect",
        Object::Array(vec![
            placement.x.into(),
            placement.y.into(),
            (placement.x + placement.width).into(),
            (placement.y + placement.height).into(),
        ]),
    );
    widget.set("AP", {
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(appearance_id));
        ap
    });
    let widget_id = doc.add_object(widget);

    {
        let page = doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|e| SignError::Pdf(e.to_string()))?;
        let mut annots = match page.get(b"Annots") {
            Ok(Object::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        annots.push(Object::Reference(widget_id));
        page.set("Annots", Object::Array(annots));
    }
    let mut form = Dictionary::new();
    form.set("Fields", Object::Array(vec![Object::Reference(widget_id)]));
    form.set("SigFlags", 3);
    let form_id = doc.add_object(form);
    doc.catalog_mut()
        .map_err(|e| SignError::Pdf(e.to_string()))?
        .set("AcroForm", Object::Reference(form_id));

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| SignError::Pdf(e.to_string()))?;

    // Find the reserved signature space and the range placeholder.
    let hex_zeros = vec![b'0'; SIGNATURE_SPACE * 2];
    let start =
        find(&bytes, &hex_zeros).ok_or_else(|| SignError::Signing("no signature space".into()))?;
    let open = start - 1; // '<'
    let close = start + hex_zeros.len() + 1; // just past '>'
    if bytes.get(open) != Some(&b'<') || bytes.get(close - 1) != Some(&b'>') {
        return Err(SignError::Signing(
            "the signature space is not a hex string".into(),
        ));
    }
    let placeholder = format!("0 {wide} {wide} {wide}");
    let range_at = find(&bytes, placeholder.as_bytes())
        .ok_or_else(|| SignError::Signing("no byte range".into()))?;
    let range = format!(
        "0 {:<w$} {:<w$} {:<w$}",
        open,
        close,
        bytes.len() - close,
        w = RANGE_DIGITS
    );
    debug_assert_eq!(range.len(), placeholder.len());
    bytes[range_at..range_at + placeholder.len()].copy_from_slice(range.as_bytes());

    let mut signed_bytes = Vec::with_capacity(bytes.len());
    signed_bytes.extend_from_slice(&bytes[..open]);
    signed_bytes.extend_from_slice(&bytes[close..]);
    let flags = openssl::pkcs7::Pkcs7Flags::DETACHED | openssl::pkcs7::Pkcs7Flags::BINARY;
    let pkcs7 = openssl::pkcs7::Pkcs7::sign(
        &signer.cert,
        &signer.key,
        &signer.chain,
        &signed_bytes,
        flags,
    )
    .and_then(|p| p.to_der())
    .map_err(|e| SignError::Signing(e.to_string()))?;
    if pkcs7.len() > SIGNATURE_SPACE {
        return Err(SignError::Signing(
            "the certificate chain is too large for the reserved space".into(),
        ));
    }
    let hex: String = pkcs7.iter().map(|b| format!("{b:02X}")).collect();
    bytes[start..start + hex.len()].copy_from_slice(hex.as_bytes());
    Ok(bytes)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// Now as a PDF date, `D:YYYYMMDDHHmmSSZ` in UTC.
fn pdf_date_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

// Howard Hinnant's days-to-civil conversion.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
#[path = "pdf_sign_tests.rs"]
mod tests;
