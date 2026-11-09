//! Annotation and bookmark seeds: bounded visitors for marks.

use super::super::{JSON_STATE_VERSION, default_version};
use super::*;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

struct AnnotationSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for AnnotationSeed<'_> {
    type Value = Annotation;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(AnnotationVisitor(self.0))
    }
}

struct AnnotationVisitor<'a>(&'a mut Budget);

impl<'de> Visitor<'de> for AnnotationVisitor<'_> {
    type Value = Annotation;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded annotation")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        self.0.charge::<A::Error>(size_of::<Annotation>())?;
        let mut id = None;
        let mut page = None;
        let mut kind = None;
        let mut text = None;
        let mut value = Annotation {
            id: 0,
            page: 0,
            kind: String::new(),
            text: String::new(),
            color: String::new(),
            created: String::new(),
            position: String::new(),
            style: String::new(),
            note: String::new(),
            pdf_binding: None,
            quads: Vec::new(),
        };
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Id => id = Some(map.next_value()?),
                Field::Page => page = Some(map.next_value()?),
                Field::Kind => {
                    kind = Some(next_string(
                        &mut map,
                        self.0,
                        MAX_IDENTIFIER_BYTES,
                        "annotation kind",
                    )?)
                }
                Field::Text => {
                    text = Some(next_string(
                        &mut map,
                        self.0,
                        MAX_TEXT_BYTES,
                        "annotation text",
                    )?)
                }
                Field::Color => {
                    value.color =
                        next_string(&mut map, self.0, MAX_IDENTIFIER_BYTES, "annotation color")?
                }
                Field::Created => {
                    value.created = next_string(
                        &mut map,
                        self.0,
                        MAX_TIMESTAMP_BYTES,
                        "annotation timestamp",
                    )?
                }
                Field::Position => {
                    value.position = next_string(
                        &mut map,
                        self.0,
                        MAX_SHORT_TEXT_BYTES,
                        "annotation position",
                    )?
                }
                Field::Style => {
                    value.style =
                        next_string(&mut map, self.0, MAX_IDENTIFIER_BYTES, "annotation style")?
                }
                Field::Note => {
                    value.note = next_string(&mut map, self.0, MAX_TEXT_BYTES, "annotation note")?
                }
                Field::PdfBinding => {
                    value.pdf_binding = map.next_value_seed(OptionalBindingSeed(self.0))?
                }
                Field::Quads => {
                    value.quads = map.next_value_seed(BoundedSequence {
                        budget: &mut *self.0,
                        max: MAX_HIGHLIGHT_QUADS,
                        name: "highlight quad",
                        marker: std::marker::PhantomData,
                    })?
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        value.id = id.ok_or_else(|| de::Error::missing_field("id"))?;
        value.page = page.ok_or_else(|| de::Error::missing_field("page"))?;
        value.kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
        value.text = text.ok_or_else(|| de::Error::missing_field("text"))?;
        Ok(value)
    }
}

struct BookmarkSeed<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for BookmarkSeed<'_> {
    type Value = BookmarkRecord;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BookmarkVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for BookmarkVisitor<'_> {
            type Value = BookmarkRecord;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded bookmark")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                self.0.charge::<A::Error>(size_of::<BookmarkRecord>())?;
                let mut page = None;
                let mut value = BookmarkRecord {
                    page: 0,
                    label: String::new(),
                    created: String::new(),
                };
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::Page => page = Some(map.next_value()?),
                        Field::Label => {
                            value.label =
                                next_string(&mut map, self.0, MAX_TEXT_BYTES, "bookmark label")?
                        }
                        Field::Created => {
                            value.created = next_string(
                                &mut map,
                                self.0,
                                MAX_TIMESTAMP_BYTES,
                                "bookmark timestamp",
                            )?
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                value.page = page.ok_or_else(|| de::Error::missing_field("page"))?;
                Ok(value)
            }
        }

        deserializer.deserialize_map(BookmarkVisitor(self.0))
    }
}

struct BoundedSequence<'a, T> {
    budget: &'a mut Budget,
    max: usize,
    name: &'static str,
    marker: std::marker::PhantomData<T>,
}

trait SeedValue: Sized {
    fn next<'de, A>(map: &mut A, budget: &mut Budget) -> Result<Option<Self>, A::Error>
    where
        A: SeqAccess<'de>;
}

impl SeedValue for Annotation {
    fn next<'de, A>(map: &mut A, budget: &mut Budget) -> Result<Option<Self>, A::Error>
    where
        A: SeqAccess<'de>,
    {
        map.next_element_seed(AnnotationSeed(budget))
    }
}

// The most per-line boxes one highlight keeps: a highlight spanning more
// lines than a page holds is not a selection anyone made.
const MAX_HIGHLIGHT_QUADS: usize = 512;

impl SeedValue for [i32; 4] {
    fn next<'de, A>(map: &mut A, budget: &mut Budget) -> Result<Option<Self>, A::Error>
    where
        A: SeqAccess<'de>,
    {
        budget.charge::<A::Error>(size_of::<[i32; 4]>())?;
        map.next_element()
    }
}

impl SeedValue for BookmarkRecord {
    fn next<'de, A>(map: &mut A, budget: &mut Budget) -> Result<Option<Self>, A::Error>
    where
        A: SeqAccess<'de>,
    {
        map.next_element_seed(BookmarkSeed(budget))
    }
}

impl<'de, T: SeedValue> DeserializeSeed<'de> for BoundedSequence<'_, T> {
    type Value = Vec<T>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SequenceVisitor<'a, T> {
            budget: &'a mut Budget,
            max: usize,
            name: &'static str,
            marker: std::marker::PhantomData<T>,
        }

        impl<'de, T: SeedValue> Visitor<'de> for SequenceVisitor<'_, T> {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a bounded {} sequence", self.name)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while values.len() < self.max {
                    let Some(value) = T::next(&mut sequence, &mut *self.budget)? else {
                        return Ok(values);
                    };
                    values.push(value);
                }
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(format_args!(
                        "{} count limit exceeded",
                        self.name
                    )));
                }
                Ok(values)
            }
        }

        deserializer.deserialize_seq(SequenceVisitor {
            budget: self.budget,
            max: self.max,
            name: self.name,
            marker: std::marker::PhantomData,
        })
    }
}

pub(super) struct AnnotationsSeed<'a>(pub(super) &'a mut Budget);

impl<'de> DeserializeSeed<'de> for AnnotationsSeed<'_> {
    type Value = AnnotationsFile;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AnnotationsVisitor<'a>(&'a mut Budget);

        impl<'de> Visitor<'de> for AnnotationsVisitor<'_> {
            type Value = AnnotationsFile;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Reader annotation sidecar")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut value = AnnotationsFile {
                    version: default_version(),
                    ..AnnotationsFile::default()
                };
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::Version => {
                            value.version = map.next_value()?;
                            if value.version > JSON_STATE_VERSION {
                                return Err(de::Error::custom(
                                    "future-version: stored annotation schema is newer than this Reader",
                                ));
                            }
                        }
                        Field::SourcePath => {
                            value.source_path = next_string(
                                &mut map,
                                self.0,
                                MAX_PATH_BYTES,
                                "annotation source path",
                            )?
                        }
                        Field::Annotations => {
                            value.annotations = map.next_value_seed(BoundedSequence {
                                budget: &mut *self.0,
                                max: MAX_ANNOTATIONS,
                                name: "annotation",
                                marker: std::marker::PhantomData,
                            })?
                        }
                        Field::NextId => value.next_id = map.next_value()?,
                        Field::Bookmarks => {
                            value.bookmarks = map.next_value_seed(BoundedSequence {
                                budget: &mut *self.0,
                                max: MAX_BOOKMARKS,
                                name: "bookmark",
                                marker: std::marker::PhantomData,
                            })?
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(value)
            }
        }

        deserializer.deserialize_map(AnnotationsVisitor(self.0))
    }
}
