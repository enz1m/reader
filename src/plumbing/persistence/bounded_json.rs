use super::{
    Annotation, AnnotationsFile, BookmarkRecord, EpubProgressAnchor, PdfAnnotationBinding,
    ProgressRecord,
};
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use std::collections::HashMap;
use std::fmt;
use std::mem::size_of;

pub(super) const MAX_LIBRARY_RECORDS: usize = 4_096;
pub(super) const MAX_ANNOTATIONS: usize = 4_096;
pub(super) const MAX_BOOKMARKS: usize = 4_096;
pub(super) const MAX_PATH_BYTES: usize = 4 * 1_024;
pub(super) const MAX_TEXT_BYTES: usize = 64 * 1_024;
pub(super) const MAX_LIBRARY_RETAINED_BYTES: usize = 4 * 1_024 * 1_024;
pub(super) const MAX_ANNOTATION_RETAINED_BYTES: usize = 8 * 1_024 * 1_024;

// Six bytes cover JSON's longest escape for one retained byte; the per-record
// allowance covers field names, punctuation, and fixed-width scalar values.
pub(super) const MAX_LIBRARY_JSON_BYTES: u64 = 6 * 4 * 1_024 * 1_024 + 4_096 * 1_024 + 1_024;
pub(super) const MAX_ANNOTATION_JSON_BYTES: u64 =
    6 * 8 * 1_024 * 1_024 + (4_096 + 4_096) * 1_024 + 1_024;

pub(super) const MAX_FIELD_NAME_BYTES: usize = 32;
const MAX_IDENTIFIER_BYTES: usize = 1_024;
const MAX_SHORT_TEXT_BYTES: usize = 4 * 1_024;
const MAX_TIMESTAMP_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DecodeFailureKind {
    Parse,
    FutureVersion,
    Limit,
}

#[derive(Debug)]
pub(super) struct DecodeFailure {
    pub(super) kind: DecodeFailureKind,
    pub(super) diagnostic: String,
}

struct Budget {
    retained: usize,
    limit: usize,
}

impl Budget {
    pub(crate) fn new(limit: usize) -> Self {
        Self { retained: 0, limit }
    }

    fn charge_plain(&mut self, bytes: usize) -> Result<(), String> {
        let retained = self
            .retained
            .checked_add(bytes)
            .ok_or_else(|| "retained-byte accounting overflow".to_string())?;
        if retained > self.limit {
            return Err("persisted aggregate retained-byte limit exceeded".into());
        }
        self.retained = retained;
        Ok(())
    }

    fn charge<E: de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.charge_plain(bytes).map_err(E::custom)
    }
}

fn validate_string(
    budget: &mut Budget,
    value: &str,
    max: usize,
    field: &str,
) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{field} exceeds its byte limit"));
    }
    budget.charge_plain(value.len())
}

struct BoundedString<'a> {
    budget: &'a mut Budget,
    max: usize,
    field: &'static str,
}

impl<'de> DeserializeSeed<'de> for BoundedString<'_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StringVisitor<'a>(BoundedString<'a>);

        impl<'de> Visitor<'de> for StringVisitor<'_> {
            type Value = String;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a {}-byte {}", self.0.max, self.0.field)
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.retain(value)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.retain(value)
            }

            fn visit_string<E>(mut self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.check(value.len())?;
                Ok(value)
            }
        }

        impl StringVisitor<'_> {
            fn check<E: de::Error>(&mut self, len: usize) -> Result<(), E> {
                if len > self.0.max {
                    return Err(E::custom(format_args!(
                        "{} exceeds its byte limit",
                        self.0.field
                    )));
                }
                self.0.budget.charge::<E>(len)
            }

            fn retain<E: de::Error>(mut self, value: &str) -> Result<String, E> {
                self.check(value.len())?;
                Ok(value.to_owned())
            }
        }

        deserializer.deserialize_str(StringVisitor(self))
    }
}

#[derive(Clone, Copy)]
enum Field {
    Version,
    Files,
    Page,
    Total,
    EpubFontSize,
    EpubLineSpacing,
    EpubDarkTheme,
    EpubTheme,
    LastOpened,
    Title,
    Author,
    Format,
    EpubViewMode,
    EpubAnchor,
    ReadingTimeSecs,
    NativeId,
    DocumentFingerprint,
    PageFingerprint,
    NativeFingerprint,
    TextFingerprint,
    Id,
    Kind,
    Text,
    Color,
    Created,
    Position,
    Style,
    Note,
    PdfBinding,
    Quads,
    SourcePath,
    Annotations,
    NextId,
    Bookmarks,
    Label,
    Unknown,
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Field;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Reader persistence field")
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(value)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value.len() > MAX_FIELD_NAME_BYTES {
                    return Err(E::custom("persistence field name exceeds its byte limit"));
                }
                Ok(match value {
                    "version" => Field::Version,
                    "files" => Field::Files,
                    "page" => Field::Page,
                    "total" => Field::Total,
                    "epub_font_size" => Field::EpubFontSize,
                    "epub_line_spacing" => Field::EpubLineSpacing,
                    "epub_dark_theme" => Field::EpubDarkTheme,
                    "epub_theme" => Field::EpubTheme,
                    "last_opened" => Field::LastOpened,
                    "title" => Field::Title,
                    "author" => Field::Author,
                    "format" => Field::Format,
                    "epub_view_mode" => Field::EpubViewMode,
                    "epub_anchor" => Field::EpubAnchor,
                    "reading_time_secs" => Field::ReadingTimeSecs,
                    "native_id" => Field::NativeId,
                    "document_fingerprint" => Field::DocumentFingerprint,
                    "page_fingerprint" => Field::PageFingerprint,
                    "native_fingerprint" => Field::NativeFingerprint,
                    "text_fingerprint" => Field::TextFingerprint,
                    "id" => Field::Id,
                    "kind" => Field::Kind,
                    "text" => Field::Text,
                    "color" => Field::Color,
                    "created" => Field::Created,
                    "position" => Field::Position,
                    "style" => Field::Style,
                    "note" => Field::Note,
                    "pdf_binding" => Field::PdfBinding,
                    "quads" => Field::Quads,
                    "source_path" => Field::SourcePath,
                    "annotations" => Field::Annotations,
                    "next_id" => Field::NextId,
                    "bookmarks" => Field::Bookmarks,
                    "label" => Field::Label,
                    _ => Field::Unknown,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct OptionalBoundedString<'a>(BoundedString<'a>);

impl<'de> DeserializeSeed<'de> for OptionalBoundedString<'_> {
    type Value = Option<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptionalVisitor<'a>(BoundedString<'a>);

        impl<'de> Visitor<'de> for OptionalVisitor<'_> {
            type Value = Option<String>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded string or null")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                self.0.deserialize(deserializer).map(Some)
            }
        }

        deserializer.deserialize_option(OptionalVisitor(self.0))
    }
}

fn next_string<'de, A>(
    map: &mut A,
    budget: &mut Budget,
    max: usize,
    field: &'static str,
) -> Result<String, A::Error>
where
    A: MapAccess<'de>,
{
    map.next_value_seed(BoundedString { budget, max, field })
}

fn next_optional_string<'de, A>(
    map: &mut A,
    budget: &mut Budget,
    max: usize,
    field: &'static str,
) -> Result<Option<String>, A::Error>
where
    A: MapAccess<'de>,
{
    map.next_value_seed(OptionalBoundedString(BoundedString { budget, max, field }))
}

struct ProgressSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for ProgressSeed<'_> {
    type Value = ProgressRecord;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ProgressVisitor(self.0))
    }
}

struct ProgressVisitor<'a>(&'a mut Budget);

impl<'de> Visitor<'de> for ProgressVisitor<'_> {
    type Value = ProgressRecord;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded Reader progress record")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        self.0.charge::<A::Error>(size_of::<ProgressRecord>())?;
        let mut value = ProgressRecord::default();
        let mut page = None;
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Page => page = Some(map.next_value()?),
                Field::Total => value.total = map.next_value()?,
                Field::EpubFontSize => value.epub_font_size = map.next_value()?,
                Field::EpubLineSpacing => value.epub_line_spacing = map.next_value()?,
                Field::EpubDarkTheme => value.epub_dark_theme = map.next_value()?,
                Field::EpubTheme => {
                    value.epub_theme =
                        next_optional_string(&mut map, self.0, MAX_SHORT_TEXT_BYTES, "EPUB theme")?
                }
                Field::LastOpened => {
                    value.last_opened = next_string(
                        &mut map,
                        self.0,
                        MAX_TIMESTAMP_BYTES,
                        "last-opened timestamp",
                    )?
                }
                Field::Title => {
                    value.title = next_string(&mut map, self.0, MAX_TEXT_BYTES, "document title")?
                }
                Field::Author => {
                    value.author = next_string(&mut map, self.0, MAX_TEXT_BYTES, "document author")?
                }
                Field::Format => {
                    value.format =
                        next_string(&mut map, self.0, MAX_IDENTIFIER_BYTES, "document format")?
                }
                Field::EpubViewMode => {
                    value.epub_view_mode = next_optional_string(
                        &mut map,
                        self.0,
                        MAX_IDENTIFIER_BYTES,
                        "EPUB view mode",
                    )?
                }
                Field::EpubAnchor => {
                    value.epub_anchor = map.next_value::<Option<EpubProgressAnchor>>()?
                }
                Field::ReadingTimeSecs => value.reading_time_secs = map.next_value()?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        value.page = page.ok_or_else(|| de::Error::missing_field("page"))?;
        Ok(value)
    }
}

struct LibraryFilesSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for LibraryFilesSeed<'_> {
    type Value = HashMap<String, ProgressRecord>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FilesVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for FilesVisitor<'_> {
            type Value = HashMap<String, ProgressRecord>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded Reader library map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut files = HashMap::new();
                while files.len() < MAX_LIBRARY_RECORDS {
                    let Some(path) = map.next_key_seed(BoundedString {
                        budget: &mut *self.0,
                        max: MAX_PATH_BYTES,
                        field: "library path",
                    })?
                    else {
                        return Ok(files);
                    };
                    let record = map.next_value_seed(ProgressSeed(&mut *self.0))?;
                    files.insert(path, record);
                }
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("library record limit exceeded"));
                }
                Ok(files)
            }
        }

        deserializer.deserialize_map(FilesVisitor(self.0))
    }
}

struct AppStateSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for AppStateSeed<'_> {
    type Value = super::AppState;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AppStateVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for AppStateVisitor<'_> {
            type Value = super::AppState;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Reader library state object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut version = super::default_version();
                let mut files = HashMap::new();
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::Version => {
                            version = map.next_value()?;
                            if version > super::JSON_STATE_VERSION {
                                return Err(de::Error::custom(
                                    "future-version: stored library schema is newer than this Reader",
                                ));
                            }
                        }
                        Field::Files => {
                            files = map.next_value_seed(LibraryFilesSeed(&mut *self.0))?
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(super::AppState { version, files })
            }
        }

        deserializer.deserialize_map(AppStateVisitor(self.0))
    }
}

struct BindingSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for BindingSeed<'_> {
    type Value = PdfAnnotationBinding;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BindingVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for BindingVisitor<'_> {
            type Value = PdfAnnotationBinding;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded native PDF annotation binding")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                self.0
                    .charge::<A::Error>(size_of::<PdfAnnotationBinding>())?;
                let mut native_id = None;
                let mut document_fingerprint = None;
                let mut page_fingerprint = None;
                let mut native_fingerprint = None;
                let mut text_fingerprint = String::new();
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::NativeId => {
                            native_id = Some(next_string(
                                &mut map,
                                self.0,
                                MAX_SHORT_TEXT_BYTES,
                                "annotation native id",
                            )?)
                        }
                        Field::DocumentFingerprint => {
                            document_fingerprint = Some(next_string(
                                &mut map,
                                self.0,
                                MAX_SHORT_TEXT_BYTES,
                                "document fingerprint",
                            )?)
                        }
                        Field::PageFingerprint => {
                            page_fingerprint = Some(next_string(
                                &mut map,
                                self.0,
                                MAX_SHORT_TEXT_BYTES,
                                "page fingerprint",
                            )?)
                        }
                        Field::NativeFingerprint => {
                            native_fingerprint = Some(next_string(
                                &mut map,
                                self.0,
                                MAX_SHORT_TEXT_BYTES,
                                "native annotation fingerprint",
                            )?)
                        }
                        Field::TextFingerprint => {
                            text_fingerprint = next_string(
                                &mut map,
                                self.0,
                                MAX_SHORT_TEXT_BYTES,
                                "annotation text fingerprint",
                            )?
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(PdfAnnotationBinding {
                    native_id: native_id.ok_or_else(|| de::Error::missing_field("native_id"))?,
                    document_fingerprint: document_fingerprint
                        .ok_or_else(|| de::Error::missing_field("document_fingerprint"))?,
                    page_fingerprint: page_fingerprint
                        .ok_or_else(|| de::Error::missing_field("page_fingerprint"))?,
                    native_fingerprint: native_fingerprint
                        .ok_or_else(|| de::Error::missing_field("native_fingerprint"))?,
                    text_fingerprint,
                })
            }
        }

        deserializer.deserialize_map(BindingVisitor(self.0))
    }
}

struct OptionalBindingSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for OptionalBindingSeed<'_> {
    type Value = Option<PdfAnnotationBinding>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptionalBindingVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for OptionalBindingVisitor<'_> {
            type Value = Option<PdfAnnotationBinding>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a native annotation binding or null")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                BindingSeed(self.0).deserialize(deserializer).map(Some)
            }
        }

        deserializer.deserialize_option(OptionalBindingVisitor(self.0))
    }
}

#[path = "bounded_json/annotation_seeds.rs"]
mod annotation_seeds;

use annotation_seeds::AnnotationsSeed;

fn decode_with_seed<'de, T>(
    bytes: &'de [u8],
    seed: impl DeserializeSeed<'de, Value = T>,
) -> Result<T, DecodeFailure> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = seed.deserialize(&mut deserializer).map_err(|error| {
        let diagnostic = error.to_string();
        let kind = if diagnostic.starts_with("future-version:") {
            DecodeFailureKind::FutureVersion
        } else if diagnostic.contains("limit") {
            DecodeFailureKind::Limit
        } else {
            DecodeFailureKind::Parse
        };
        DecodeFailure { kind, diagnostic }
    })?;
    deserializer.end().map_err(|error| DecodeFailure {
        kind: DecodeFailureKind::Parse,
        diagnostic: error.to_string(),
    })?;
    Ok(value)
}

pub(super) fn validate_library(value: &super::AppState) -> Result<(), String> {
    if value.version > super::JSON_STATE_VERSION {
        return Err("library schema version is newer than this Reader".into());
    }
    if value.files.len() > MAX_LIBRARY_RECORDS {
        return Err("library record limit exceeded".into());
    }
    let mut budget = Budget::new(MAX_LIBRARY_RETAINED_BYTES);
    for (path, record) in &value.files {
        validate_string(&mut budget, path, MAX_PATH_BYTES, "library path")?;
        budget.charge_plain(size_of::<ProgressRecord>())?;
        if let Some(theme) = &record.epub_theme {
            validate_string(&mut budget, theme, MAX_SHORT_TEXT_BYTES, "EPUB theme")?;
        }
        validate_string(
            &mut budget,
            &record.last_opened,
            MAX_TIMESTAMP_BYTES,
            "last-opened timestamp",
        )?;
        validate_string(&mut budget, &record.title, MAX_TEXT_BYTES, "document title")?;
        validate_string(
            &mut budget,
            &record.author,
            MAX_TEXT_BYTES,
            "document author",
        )?;
        validate_string(
            &mut budget,
            &record.format,
            MAX_IDENTIFIER_BYTES,
            "document format",
        )?;
        if let Some(view_mode) = &record.epub_view_mode {
            validate_string(
                &mut budget,
                view_mode,
                MAX_IDENTIFIER_BYTES,
                "EPUB view mode",
            )?;
        }
    }
    Ok(())
}

fn validate_binding(budget: &mut Budget, binding: &PdfAnnotationBinding) -> Result<(), String> {
    budget.charge_plain(size_of::<PdfAnnotationBinding>())?;
    for (value, field) in [
        (&binding.native_id, "annotation native id"),
        (&binding.document_fingerprint, "document fingerprint"),
        (&binding.page_fingerprint, "page fingerprint"),
        (&binding.native_fingerprint, "native annotation fingerprint"),
        (&binding.text_fingerprint, "annotation text fingerprint"),
    ] {
        validate_string(budget, value, MAX_SHORT_TEXT_BYTES, field)?;
    }
    Ok(())
}

pub(super) fn validate_annotations(value: &AnnotationsFile) -> Result<(), String> {
    if value.version > super::JSON_STATE_VERSION {
        return Err("annotation schema version is newer than this Reader".into());
    }
    if value.annotations.len() > MAX_ANNOTATIONS {
        return Err("annotation count limit exceeded".into());
    }
    if value.bookmarks.len() > MAX_BOOKMARKS {
        return Err("bookmark count limit exceeded".into());
    }
    let mut budget = Budget::new(MAX_ANNOTATION_RETAINED_BYTES);
    validate_string(
        &mut budget,
        &value.source_path,
        MAX_PATH_BYTES,
        "annotation source path",
    )?;
    for annotation in &value.annotations {
        budget.charge_plain(size_of::<Annotation>())?;
        validate_string(
            &mut budget,
            &annotation.kind,
            MAX_IDENTIFIER_BYTES,
            "annotation kind",
        )?;
        validate_string(
            &mut budget,
            &annotation.text,
            MAX_TEXT_BYTES,
            "annotation text",
        )?;
        validate_string(
            &mut budget,
            &annotation.color,
            MAX_IDENTIFIER_BYTES,
            "annotation color",
        )?;
        validate_string(
            &mut budget,
            &annotation.created,
            MAX_TIMESTAMP_BYTES,
            "annotation timestamp",
        )?;
        validate_string(
            &mut budget,
            &annotation.position,
            MAX_SHORT_TEXT_BYTES,
            "annotation position",
        )?;
        validate_string(
            &mut budget,
            &annotation.style,
            MAX_IDENTIFIER_BYTES,
            "annotation style",
        )?;
        validate_string(
            &mut budget,
            &annotation.note,
            MAX_TEXT_BYTES,
            "annotation note",
        )?;
        if let Some(binding) = &annotation.pdf_binding {
            validate_binding(&mut budget, binding)?;
        }
    }
    for bookmark in &value.bookmarks {
        budget.charge_plain(size_of::<BookmarkRecord>())?;
        validate_string(
            &mut budget,
            &bookmark.label,
            MAX_TEXT_BYTES,
            "bookmark label",
        )?;
        validate_string(
            &mut budget,
            &bookmark.created,
            MAX_TIMESTAMP_BYTES,
            "bookmark timestamp",
        )?;
    }
    Ok(())
}

pub(super) fn decode_library(bytes: &[u8]) -> Result<super::AppState, DecodeFailure> {
    let mut budget = Budget::new(MAX_LIBRARY_RETAINED_BYTES);
    let value = decode_with_seed(bytes, AppStateSeed(&mut budget))?;
    if value.version > super::JSON_STATE_VERSION {
        return Err(DecodeFailure {
            kind: DecodeFailureKind::FutureVersion,
            diagnostic: "stored library schema is newer than this Reader".into(),
        });
    }
    Ok(value)
}

pub(super) fn decode_annotations(bytes: &[u8]) -> Result<AnnotationsFile, DecodeFailure> {
    let mut budget = Budget::new(MAX_ANNOTATION_RETAINED_BYTES);
    let mut value = decode_with_seed(bytes, AnnotationsSeed(&mut budget))?;
    if value.version > super::JSON_STATE_VERSION {
        return Err(DecodeFailure {
            kind: DecodeFailureKind::FutureVersion,
            diagnostic: "stored annotation schema is newer than this Reader".into(),
        });
    }
    if value.next_id == 0 {
        value.next_id = value
            .annotations
            .iter()
            .map(|annotation| annotation.id.saturating_add(1))
            .max()
            .unwrap_or(1);
    }
    if value.version == 0 {
        value.version = super::default_version();
    }
    Ok(value)
}

struct AnnotationSourceSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for AnnotationSourceSeed<'_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SourceVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for SourceVisitor<'_> {
            type Value = String;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an annotation sidecar object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut source_path = String::new();
                while let Some(field) = map.next_key::<Field>()? {
                    if matches!(field, Field::SourcePath) {
                        source_path = next_string(
                            &mut map,
                            self.0,
                            MAX_PATH_BYTES,
                            "annotation source path",
                        )?;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(source_path)
            }
        }

        deserializer.deserialize_map(SourceVisitor(self.0))
    }
}

pub(super) fn annotation_source(body: &[u8]) -> Option<String> {
    let mut budget = Budget::new(MAX_PATH_BYTES);
    decode_with_seed(body, AnnotationSourceSeed(&mut budget)).ok()
}
