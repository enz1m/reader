//! PDF metadata: dates, fields, and reads.

use super::*;

/// A PDF date with immutable source bytes and a separately owned user draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfDateField {
    value: String,
    pub(super) original: Option<(Vec<u8>, StringFormat)>,
    edited: bool,
}

impl PdfDateField {
    fn from_raw(bytes: &[u8], format: StringFormat) -> Self {
        Self {
            value: decode_pdf_string(bytes),
            original: Some((bytes.to_vec(), format)),
            edited: false,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn edit(&mut self, value: String) {
        self.value = value;
        self.edited = true;
    }

    pub fn is_edited(&self) -> bool {
        self.edited
    }

    pub fn requests_removal(&self) -> bool {
        self.edited && self.value.is_empty() && self.original.is_some()
    }

    pub fn original_bytes(&self) -> Option<&[u8]> {
        self.original.as_ref().map(|(bytes, _)| bytes.as_slice())
    }

    pub fn original_format(&self) -> Option<StringFormat> {
        self.original.as_ref().map(|(_, format)| *format)
    }

    pub fn warning(&self) -> Option<&'static str> {
        if self.edited {
            if self.value.is_empty() {
                return None;
            }
            return validate_pdf_date(&self.value)
                .err()
                .map(PdfDateError::message);
        }
        let (bytes, _) = self.original.as_ref()?;
        let Ok(raw) = std::str::from_utf8(bytes) else {
            return Some(MALFORMED_PDF_DATE_WARNING);
        };
        validate_pdf_date(raw)
            .err()
            .map(|_| MALFORMED_PDF_DATE_WARNING)
    }

    fn is_present_or_added(&self) -> bool {
        self.original.is_some() || (self.edited && !self.value.is_empty())
    }
}

/// Editable `/Info` values plus the source-owned creation and modification dates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub creation_date: PdfDateField,
    pub modification_date: PdfDateField,
}

impl PdfMetadata {
    /// True when every editable field is blank. Drives the metadata dialog's
    /// "nothing to clear" disabled state.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.author.is_empty()
            && self.subject.is_empty()
            && self.keywords.is_empty()
            && self.creator.is_empty()
            && self.producer.is_empty()
            && !self.creation_date.is_present_or_added()
            && !self.modification_date.is_present_or_added()
    }

    pub fn date_removals_requested(&self) -> MetadataDateRemoval {
        MetadataDateRemoval {
            creation: self.creation_date.requests_removal(),
            modification: self.modification_date.requests_removal(),
        }
    }

    pub fn has_invalid_date_edit(&self) -> bool {
        [&self.creation_date, &self.modification_date]
            .into_iter()
            .any(|date| date.is_edited() && !date.value().is_empty() && date.warning().is_some())
    }
}

/// Exact date fields whose empty drafts received removal confirmation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataDateRemoval {
    pub creation: bool,
    pub modification: bool,
}

impl MetadataDateRemoval {
    pub fn is_empty(self) -> bool {
        !self.creation && !self.modification
    }

    pub(super) fn permits(self, field: MetadataField) -> bool {
        match field {
            MetadataField::CreationDate => self.creation,
            MetadataField::ModificationDate => self.modification,
            _ => false,
        }
    }
}

/// User-facing syntax accepted by `validate_pdf_date`.
pub const PDF_DATE_FORMAT_HELP: &str =
    "Use D:YYYY, then optional MM, DD, HH, mm, SS; a full date may end in Z or +HH'mm' / -HH'mm'.";
pub const MALFORMED_PDF_DATE_WARNING: &str =
    "This existing PDF date is malformed. Its raw bytes will be preserved unless you replace it.";
pub const PDF_DATE_REMOVAL_NOTICE: &str = "Saving an empty date removes it after confirmation.";

/// Precision explicitly represented by a validated PDF date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfDatePrecision {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    ZonedSecond,
}

/// Validation failure for an explicitly edited PDF date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfDateError {
    Prefix,
    Precision,
    Digits,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Timezone,
}

impl PdfDateError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Prefix => "PDF dates must start with D:.",
            Self::Precision => "Use year, month, day, hour, minute, or second precision.",
            Self::Digits => "PDF date components must use ASCII digits.",
            Self::Year => "Year must be between 0001 and 9999.",
            Self::Month => "Month must be between 01 and 12.",
            Self::Day => "Day is outside the selected month.",
            Self::Hour => "Hour must be between 00 and 23.",
            Self::Minute => "Minute must be between 00 and 59.",
            Self::Second => "Second must be between 00 and 59.",
            Self::Timezone => "Timezone must be Z or +HH'mm' / -HH'mm' after seconds.",
        }
    }
}

/// Which metadata field an edit message names.
///
/// One typed field avoids near-identical reducer branches where a copy-paste slip writes
/// the wrong field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataField {
    Title,
    Author,
    Subject,
    Keywords,
    Creator,
    Producer,
    CreationDate,
    ModificationDate,
}

impl MetadataField {
    /// Every editable field, in dialog order. A stable array: the metadata
    /// dialog iterates it, so adding a field here adds a row there.
    pub const ALL: &'static [MetadataField] = &[
        MetadataField::Title,
        MetadataField::Author,
        MetadataField::Subject,
        MetadataField::Keywords,
        MetadataField::Creator,
        MetadataField::Producer,
        MetadataField::CreationDate,
        MetadataField::ModificationDate,
    ];

    pub(super) const TEXT_FIELDS: &'static [MetadataField] = &[
        MetadataField::Title,
        MetadataField::Author,
        MetadataField::Subject,
        MetadataField::Keywords,
        MetadataField::Creator,
        MetadataField::Producer,
    ];

    /// The row title the dialog shows (`Document title`, `Author name`, …).
    /// See `view::metadata_dialog`.
    pub fn label(self) -> &'static str {
        match self {
            MetadataField::Title => "Document title",
            MetadataField::Author => "Author name",
            MetadataField::Subject => "Document subject",
            MetadataField::Keywords => "Keywords",
            MetadataField::Creator => "Creator",
            MetadataField::Producer => "Producer",
            MetadataField::CreationDate => "Creation date",
            MetadataField::ModificationDate => "Modification date",
        }
    }

    /// The field's placeholder. `Comma-separated keywords` is a PLACEHOLDER,
    /// not a label — it describes the expected format of the value, and the
    /// row's title stays `Keywords`.
    pub fn placeholder(self) -> &'static str {
        match self {
            MetadataField::Title => "Document title",
            MetadataField::Author => "Author name",
            MetadataField::Subject => "Document subject",
            MetadataField::Keywords => "Comma-separated keywords",
            MetadataField::Creator => "Creating application",
            MetadataField::Producer => "PDF producer",
            MetadataField::CreationDate | MetadataField::ModificationDate => "D:YYYYMMDDHHmmSSZ",
        }
    }

    /// Read this field out of `meta`.
    pub fn get(self, meta: &PdfMetadata) -> &str {
        match self {
            MetadataField::Title => &meta.title,
            MetadataField::Author => &meta.author,
            MetadataField::Subject => &meta.subject,
            MetadataField::Keywords => &meta.keywords,
            MetadataField::Creator => &meta.creator,
            MetadataField::Producer => &meta.producer,
            MetadataField::CreationDate => meta.creation_date.value(),
            MetadataField::ModificationDate => meta.modification_date.value(),
        }
    }

    /// Write this field into `meta`. `value` is stored verbatim — including
    /// the empty string, which is a real user intent (see the module docs).
    pub fn set(self, meta: &mut PdfMetadata, value: String) {
        match self {
            MetadataField::Title => meta.title = value,
            MetadataField::Author => meta.author = value,
            MetadataField::Subject => meta.subject = value,
            MetadataField::Keywords => meta.keywords = value,
            MetadataField::Creator => meta.creator = value,
            MetadataField::Producer => meta.producer = value,
            MetadataField::CreationDate => meta.creation_date.edit(value),
            MetadataField::ModificationDate => meta.modification_date.edit(value),
        }
    }

    // The `/Info` dictionary key this field maps to.
    pub(super) fn pdf_key(self) -> &'static [u8] {
        match self {
            MetadataField::Title => b"Title",
            MetadataField::Author => b"Author",
            MetadataField::Subject => b"Subject",
            MetadataField::Keywords => b"Keywords",
            MetadataField::Creator => b"Creator",
            MetadataField::Producer => b"Producer",
            MetadataField::CreationDate => b"CreationDate",
            MetadataField::ModificationDate => b"ModDate",
        }
    }
}

/// Read-only document properties (`Document Properties`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocProperties {
    pub page_count: usize,
    pub file_size_bytes: u64,
    pub pdf_version: String,
    pub is_encrypted: bool,
}

pub(super) fn load(path: &Path) -> Result<Document, String> {
    Document::load(path).map_err(|e| format!("Could not read PDF: {e}"))
}

// Decode a PDF text string. PDF text strings are either PDFDocEncoding
// (≈Latin-1) or UTF-16BE with a byte-order mark; both appear in the wild, so
// assuming UTF-8 mangles half the corpus.
pub(super) fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|&b| b as char).collect()
}

pub(super) fn encode_pdf_string(text: &str) -> Vec<u8> {
    if text.is_ascii() {
        return text.as_bytes().to_vec();
    }
    let mut bytes = vec![0xfe, 0xff];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

/// A valid PDF date of at least day precision, split for a date picker and a
/// time field: the day, the time (zero where the date is less precise) and
/// the zone suffix as written (`Z`, `+02'00'`, or empty).
pub fn pdf_date_parts(
    value: &str,
) -> Option<(
    enzui_iced_core::civil_date::Date,
    enzui_iced_core::time_of_day::TimeOfDay,
    String,
)> {
    validate_pdf_date(value).ok()?;
    let body = value.strip_prefix("D:")?;
    let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> u32 {
        digits.get(range).and_then(|d| d.parse().ok()).unwrap_or(0)
    };
    let date =
        enzui_iced_core::civil_date::Date::new(number(0..4) as i32, number(4..6), number(6..8))?;
    let time = enzui_iced_core::time_of_day::TimeOfDay::new(
        number(8..10),
        number(10..12),
        number(12..14),
    )?;
    let zone = if digits.len() == 14 {
        body[14..].to_string()
    } else {
        String::new()
    };
    Some((date, time, zone))
}

/// The PDF date for a day, a time and a zone suffix (as [`pdf_date_parts`]
/// returns it), at second precision.
pub fn pdf_date_from_parts(
    date: enzui_iced_core::civil_date::Date,
    time: enzui_iced_core::time_of_day::TimeOfDay,
    zone: &str,
) -> String {
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}{zone}",
        date.year(),
        date.month(),
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

pub fn validate_pdf_date(value: &str) -> Result<PdfDatePrecision, PdfDateError> {
    let Some(body) = value.strip_prefix("D:") else {
        return Err(PdfDateError::Prefix);
    };
    if !body.is_ascii() {
        return Err(PdfDateError::Digits);
    }
    let (digits, zoned) = if let Some(digits) = body.strip_suffix('Z') {
        if digits.len() != 14 {
            return Err(PdfDateError::Timezone);
        }
        (digits, true)
    } else if body.len() == 21 && matches!(body.as_bytes().get(14), Some(b'+') | Some(b'-')) {
        let zone = &body[14..];
        if zone.as_bytes().get(3) != Some(&b'\'')
            || zone.as_bytes().get(6) != Some(&b'\'')
            || !zone[1..3].bytes().all(|byte| byte.is_ascii_digit())
            || !zone[4..6].bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(PdfDateError::Timezone);
        }
        let hours = component(zone, 1, 3)?;
        let minutes = component(zone, 4, 6)?;
        if hours > 23 || minutes > 59 {
            return Err(PdfDateError::Timezone);
        }
        (&body[..14], true)
    } else {
        (body, false)
    };
    let precision = match digits.len() {
        4 => PdfDatePrecision::Year,
        6 => PdfDatePrecision::Month,
        8 => PdfDatePrecision::Day,
        10 => PdfDatePrecision::Hour,
        12 => PdfDatePrecision::Minute,
        14 if zoned => PdfDatePrecision::ZonedSecond,
        14 => PdfDatePrecision::Second,
        _ => return Err(PdfDateError::Precision),
    };
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PdfDateError::Digits);
    }
    let year = component(digits, 0, 4)?;
    if year == 0 {
        return Err(PdfDateError::Year);
    }
    let month = (digits.len() >= 6)
        .then(|| component(digits, 4, 6))
        .transpose()?;
    if month.is_some_and(|month| !(1..=12).contains(&month)) {
        return Err(PdfDateError::Month);
    }
    let day = (digits.len() >= 8)
        .then(|| component(digits, 6, 8))
        .transpose()?;
    if let (Some(month), Some(day)) = (month, day)
        && !(1..=days_in_month(year, month)).contains(&day)
    {
        return Err(PdfDateError::Day);
    }
    if digits.len() >= 10 && component(digits, 8, 10)? > 23 {
        return Err(PdfDateError::Hour);
    }
    if digits.len() >= 12 && component(digits, 10, 12)? > 59 {
        return Err(PdfDateError::Minute);
    }
    if digits.len() >= 14 && component(digits, 12, 14)? > 59 {
        return Err(PdfDateError::Second);
    }
    Ok(precision)
}

fn component(value: &str, start: usize, end: usize) -> Result<u32, PdfDateError> {
    value
        .get(start..end)
        .filter(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|part| part.parse().ok())
        .ok_or(PdfDateError::Digits)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

// The `/Info` dictionary's object id, if the document has one.
pub(super) fn info_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Info").ok()?.as_reference().ok()
}

/// Read the `/Info` metadata of the PDF at `path`.
///
/// A document with no `/Info` dictionary yields an all-blank [`PdfMetadata`],
/// which is the correct "nothing set" reading — not an error.
pub fn read_metadata(path: &Path) -> Result<PdfMetadata, String> {
    let doc = load(path)?;
    Ok(read_metadata_from(&doc))
}

pub(super) fn read_metadata_from(doc: &Document) -> PdfMetadata {
    let mut meta = PdfMetadata::default();
    let Some(id) = info_id(doc) else {
        return meta;
    };
    let Ok(dict) = doc.get_dictionary(id) else {
        return meta;
    };
    for field in MetadataField::TEXT_FIELDS {
        if let Some((bytes, _)) = info_string(doc, dict, field.pdf_key()) {
            field.set(&mut meta, decode_pdf_string(bytes));
        }
    }
    if let Some((bytes, format)) = info_string(doc, dict, b"CreationDate") {
        meta.creation_date = PdfDateField::from_raw(bytes, format);
    }
    if let Some((bytes, format)) = info_string(doc, dict, b"ModDate") {
        meta.modification_date = PdfDateField::from_raw(bytes, format);
    }
    if let Some((bytes, _)) = info_string(doc, dict, b"Producer") {
        meta.producer = decode_pdf_string(bytes);
    }
    meta
}

fn info_string<'a>(
    doc: &'a Document,
    dict: &'a lopdf::Dictionary,
    key: &[u8],
) -> Option<(&'a [u8], StringFormat)> {
    let object = dict.get(key).ok()?;
    let (_, object) = doc.dereference(object).ok()?;
    match object {
        Object::String(bytes, format) => Some((bytes, *format)),
        _ => None,
    }
}

/// Read the read-only `Document Properties` of the PDF at `path`.
pub fn read_properties(path: &Path) -> Result<DocProperties, String> {
    let doc = load(path)?;
    let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(DocProperties {
        page_count: doc.get_pages().len(),
        file_size_bytes,
        pdf_version: doc.version.clone(),
        is_encrypted: doc.trailer.get(b"Encrypt").is_ok(),
    })
}

/// How many pages the PDF at `path` has. Used when merging another document
/// so the arranger can append the right number of entries.
pub fn page_count(path: &Path) -> Result<usize, String> {
    Ok(load(path)?.get_pages().len())
}

/// Format a byte count with the framework's canonical decimal SI units.
pub fn format_file_size(bytes: u64) -> String {
    enzui_iced_widgets::file_browser::format_bytes(bytes)
}
