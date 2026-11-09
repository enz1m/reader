//! Annotation and bookmark mutations.

use super::*;

/// Insert an already-constructed annotation, preserving its id + timestamp.
/// Used by the "undo delete" flow — the annotation retains its original id
/// so any references remain valid.
pub fn restore_annotation(file: &mut AnnotationsFile, ann: Annotation) {
    // Ensure next_id stays ahead of any restored id.
    if ann.id >= file.next_id {
        file.next_id = ann.id + 1;
    }
    // Guard against duplicate id (idempotent restore).
    if file.annotations.iter().any(|a| a.id == ann.id) {
        return;
    }
    file.annotations.push(ann);
}

/// Update the `note` body attached to a highlight annotation. Returns whether
/// the annotation existed.
pub fn update_annotation_note(file: &mut AnnotationsFile, id: u64, note: &str) -> bool {
    for a in file.annotations.iter_mut() {
        if a.id == id {
            a.note = note.to_string();
            return true;
        }
    }
    false
}

/// Update the `style` of a highlight annotation.
pub fn update_annotation_style(file: &mut AnnotationsFile, id: u64, style: &str) -> bool {
    for a in file.annotations.iter_mut() {
        if a.id == id {
            a.style = style.to_string();
            return true;
        }
    }
    false
}

pub fn update_annotation_color(file: &mut AnnotationsFile, id: u64, color: &str) -> bool {
    for annotation in file.annotations.iter_mut() {
        if annotation.id == id {
            annotation.color = color.to_string();
            return true;
        }
    }
    false
}

pub fn remove_annotation(file: &mut AnnotationsFile, id: u64) -> bool {
    let before = file.annotations.len();
    file.annotations.retain(|a| a.id != id);
    file.annotations.len() != before
}

pub fn update_annotation_text(file: &mut AnnotationsFile, id: u64, text: &str) -> bool {
    for a in file.annotations.iter_mut() {
        if a.id == id {
            a.text = text.to_string();
            return true;
        }
    }
    false
}

/// Toggle a bookmark for `page`. If already bookmarked, removes it and returns
/// `false`. If not bookmarked, adds it and returns `true`.
pub fn toggle_bookmark(file: &mut AnnotationsFile, page: usize, label: &str) -> bool {
    if let Some(idx) = file.bookmarks.iter().position(|b| b.page == page) {
        file.bookmarks.remove(idx);
        false
    } else {
        file.bookmarks.push(BookmarkRecord {
            page,
            label: label.to_string(),
            created: current_timestamp(),
        });
        file.bookmarks.sort_by_key(|b| b.page);
        true
    }
}

/// Check whether `page` is bookmarked.
pub fn is_bookmarked(file: &AnnotationsFile, page: usize) -> bool {
    file.bookmarks.iter().any(|b| b.page == page)
}

/// Remove a bookmark by page. Returns `true` if found.
pub fn remove_bookmark(file: &mut AnnotationsFile, page: usize) -> bool {
    let before = file.bookmarks.len();
    file.bookmarks.retain(|b| b.page != page);
    file.bookmarks.len() != before
}
