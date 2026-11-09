use std::collections::{HashMap, HashSet};
use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId};

pub const MAX_OUTLINE_ENTRIES: usize = 2_048;
pub const MAX_OUTLINE_DEPTH: usize = 32;
const MAX_NAME_TREE_NODES: usize = 2_048;
const MAX_OUTLINE_TITLE_CHARS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfOutlineEntry {
    pub title: String,
    pub page: usize,
    pub level: usize,
}

#[derive(Clone, Debug, Default)]
pub struct PdfNavigationData {
    pub page_count: usize,
    pub text: Vec<crate::pdf::PdfPageText>,
    pub text_truncated: bool,
    pub outline: Vec<PdfOutlineEntry>,
    pub outline_truncated: bool,
    pub outline_error: Option<String>,
}

pub fn load(path: &Path, password: Option<&str>) -> Result<PdfNavigationData, String> {
    let text = crate::pdf::searchable_page_text(&path.to_string_lossy(), password)?;
    let (outline, outline_error) = match read_outline(path, password) {
        Ok(outline) => (outline, None),
        Err(error) => (OutlineResult::default(), Some(error)),
    };
    Ok(PdfNavigationData {
        page_count: text.page_count,
        text: text.pages,
        text_truncated: text.truncated,
        outline: outline.entries,
        outline_truncated: outline.truncated,
        outline_error,
    })
}

#[derive(Default)]
struct OutlineResult {
    entries: Vec<PdfOutlineEntry>,
    truncated: bool,
}

fn read_outline(path: &Path, password: Option<&str>) -> Result<OutlineResult, String> {
    let mut document =
        Document::load(path).map_err(|error| format!("Could not read PDF outline: {error}"))?;
    if document.is_encrypted() {
        let Some(password) = password else {
            return Err("The PDF outline is encrypted.".to_string());
        };
        document
            .decrypt(password)
            .map_err(|_| "The PDF outline could not be unlocked.".to_string())?;
    }
    let pages: HashMap<ObjectId, usize> = document
        .get_pages()
        .into_iter()
        .map(|(number, id)| (id, number.saturating_sub(1) as usize))
        .collect();
    let catalog = document.catalog().map_err(|error| error.to_string())?;
    let names = named_destinations(&document, catalog);
    let Some(first) = dictionary_value(&document, catalog, b"Outlines")
        .and_then(|object| object_dictionary(&document, &object))
        .and_then(|dictionary| dictionary.get(b"First").ok().cloned())
    else {
        return Ok(OutlineResult::default());
    };
    let mut result = OutlineResult::default();
    let mut visited = HashSet::new();
    visit_outline_chain(
        &document,
        first,
        0,
        &pages,
        &names,
        &mut visited,
        &mut result,
    );
    Ok(result)
}

fn visit_outline_chain(
    document: &Document,
    first: Object,
    level: usize,
    pages: &HashMap<ObjectId, usize>,
    names: &HashMap<Vec<u8>, Object>,
    visited: &mut HashSet<ObjectId>,
    result: &mut OutlineResult,
) {
    if level >= MAX_OUTLINE_DEPTH {
        result.truncated = true;
        return;
    }
    let mut current = Some(first);
    while let Some(object) = current.take() {
        if result.entries.len() >= MAX_OUTLINE_ENTRIES || visited.len() >= MAX_OUTLINE_ENTRIES {
            result.truncated = true;
            return;
        }
        if let Object::Reference(id) = &object
            && !visited.insert(*id)
        {
            result.truncated = true;
            return;
        }
        let Some(dictionary) = object_dictionary(document, &object) else {
            return;
        };
        if let Some(title) = dictionary
            .get(b"Title")
            .ok()
            .and_then(|value| object_bytes(document, value))
            .map(|bytes| decode_pdf_text(&bytes))
            .filter(|title| !title.trim().is_empty())
            && let Some(page) = outline_page(document, &dictionary, pages, names)
        {
            result.entries.push(PdfOutlineEntry {
                title: title.chars().take(MAX_OUTLINE_TITLE_CHARS).collect(),
                page,
                level,
            });
        }
        if let Ok(child) = dictionary.get(b"First") {
            visit_outline_chain(
                document,
                child.clone(),
                level + 1,
                pages,
                names,
                visited,
                result,
            );
        }
        current = dictionary.get(b"Next").ok().cloned();
    }
}

fn outline_page(
    document: &Document,
    item: &Dictionary,
    pages: &HashMap<ObjectId, usize>,
    names: &HashMap<Vec<u8>, Object>,
) -> Option<usize> {
    let destination = if let Ok(destination) = item.get(b"Dest") {
        destination.clone()
    } else {
        let action = dictionary_value(document, item, b"A")?;
        let action = object_dictionary(document, &action)?;
        if object_bytes(document, action.get(b"S").ok()?)?.as_slice() != b"GoTo" {
            return None;
        }
        action.get(b"D").ok()?.clone()
    };
    destination_page(document, &destination, pages, names, 0)
}

fn destination_page(
    document: &Document,
    destination: &Object,
    pages: &HashMap<ObjectId, usize>,
    names: &HashMap<Vec<u8>, Object>,
    depth: usize,
) -> Option<usize> {
    if depth >= 8 {
        return None;
    }
    match destination {
        Object::Array(values) => match values.first()? {
            Object::Reference(id) => pages.get(id).copied(),
            Object::Integer(page) => usize::try_from(*page)
                .ok()
                .filter(|page| *page < pages.len()),
            other => destination_page(document, other, pages, names, depth + 1),
        },
        Object::Name(name) | Object::String(name, _) => names
            .get(name)
            .and_then(|value| destination_page(document, value, pages, names, depth + 1)),
        Object::Reference(id) => document
            .get_object(*id)
            .ok()
            .and_then(|value| destination_page(document, value, pages, names, depth + 1)),
        Object::Dictionary(dictionary) => dictionary
            .get(b"D")
            .ok()
            .and_then(|value| destination_page(document, value, pages, names, depth + 1)),
        _ => None,
    }
}

fn named_destinations(document: &Document, catalog: &Dictionary) -> HashMap<Vec<u8>, Object> {
    let mut destinations = HashMap::new();
    if let Some(dictionary) = dictionary_value(document, catalog, b"Dests")
        .and_then(|object| object_dictionary(document, &object))
    {
        for (name, value) in dictionary.iter().take(MAX_NAME_TREE_NODES) {
            destinations.insert(name.clone(), value.clone());
        }
    }
    if let Some(names) = dictionary_value(document, catalog, b"Names")
        .and_then(|object| object_dictionary(document, &object))
        && let Some(root) = dictionary_value(document, &names, b"Dests")
    {
        let mut stack = vec![root];
        let mut visited = HashSet::new();
        while let Some(node) = stack.pop() {
            if visited.len() >= MAX_NAME_TREE_NODES {
                break;
            }
            if let Object::Reference(id) = &node
                && !visited.insert(*id)
            {
                continue;
            }
            let Some(dictionary) = object_dictionary(document, &node) else {
                continue;
            };
            if let Ok(Object::Array(values)) = dictionary.get(b"Names") {
                for pair in values.as_chunks::<2>().0 {
                    if destinations.len() >= MAX_NAME_TREE_NODES {
                        break;
                    }
                    if let Some(name) = object_bytes(document, &pair[0]) {
                        destinations.insert(name, pair[1].clone());
                    }
                }
            }
            if let Ok(Object::Array(kids)) = dictionary.get(b"Kids") {
                let remaining = MAX_NAME_TREE_NODES.saturating_sub(stack.len());
                stack.extend(kids.iter().rev().take(remaining).cloned());
            }
        }
    }
    destinations
}

fn dictionary_value(document: &Document, dictionary: &Dictionary, key: &[u8]) -> Option<Object> {
    let value = dictionary.get(key).ok()?;
    match value {
        Object::Reference(id) => document.get_object(*id).ok().cloned(),
        value => Some(value.clone()),
    }
}

fn object_dictionary(document: &Document, object: &Object) -> Option<Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary.clone()),
        Object::Reference(id) => document.get_object(*id).ok()?.as_dict().ok().cloned(),
        _ => None,
    }
}

fn object_bytes(document: &Document, object: &Object) -> Option<Vec<u8>> {
    object_bytes_with_depth(document, object, 0)
}

fn object_bytes_with_depth(document: &Document, object: &Object, depth: usize) -> Option<Vec<u8>> {
    if depth >= 8 {
        return None;
    }
    match object {
        Object::Name(bytes) | Object::String(bytes, _) => Some(
            bytes
                .iter()
                .take(MAX_OUTLINE_TITLE_CHARS * 4)
                .copied()
                .collect(),
        ),
        Object::Reference(id) => {
            object_bytes_with_depth(document, document.get_object(*id).ok()?, depth + 1)
        }
        _ => None,
    }
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|byte| *byte as char).collect()
    }
}
