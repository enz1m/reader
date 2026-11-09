//! Shared undo/redo history for page and annotation mutations.
//!
//! New edits clear redo. History is bounded, and restored entries retain their identities.

use crate::pages::{PageArrangement, PageEntry};
use crate::plumbing::persistence::{
    Annotation, AnnotationsFile, BookmarkRecord, remove_annotation, remove_bookmark,
    restore_annotation, toggle_bookmark, update_annotation_text,
};

/// The mutable document state an undo or redo applies to.
///
/// One borrow rather than two arguments, so a call site cannot supply the annotation
/// half and forget the page half.
pub struct UndoTarget<'a> {
    pub annotations: &'a mut AnnotationsFile,
    pub pages: &'a mut PageArrangement,
}

impl<'a> UndoTarget<'a> {
    pub fn new(annotations: &'a mut AnnotationsFile, pages: &'a mut PageArrangement) -> Self {
        Self { annotations, pages }
    }
}

/// One user-visible mutation the app can undo or redo.
///
/// Each variant carries exactly the state needed to reverse the mutation: a destructive
/// op carries the whole record, an add only what redo needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoableAction {
    /// A new annotation was added; `id` was assigned by
    /// `persistence::add_annotation` / `add_annotation_full`.
    /// Undo: remove by id. Redo: re-add the full [`Annotation`] via
    /// `restore_annotation`, so the id and timestamp round-trip.
    AddAnnotation {
        /// The annotation that was added, captured so redo can restore the
        /// exact same id / timestamp.
        annotation: Annotation,
    },
    /// An annotation was removed. `annotation` is the full record captured
    /// before removal, restored intact on undo.
    RemoveAnnotation {
        annotation: Annotation,
    },
    /// An annotation's `text` was overwritten (inline `Edit Note`).
    UpdateAnnotationText {
        id: u64,
        previous: String,
        /// The new text after the edit, reapplied on redo.
        new: String,
    },
    UpdateAnnotationNote {
        id: u64,
        previous: String,
        new: String,
    },
    UpdateAnnotationColor {
        id: u64,
        previous: String,
        new: String,
    },
    /// A bookmark was added on `page`. Undo removes it; redo re-adds it.
    /// The full record is captured so redo restores the label + created stamp.
    AddBookmark {
        record: BookmarkRecord,
    },
    /// A bookmark was removed; the full record is captured.
    RemoveBookmark {
        record: BookmarkRecord,
    },

    MovePage {
        id: u64,
        from: usize,
        to: usize,
    },
    /// A page was deleted; the whole entry and its index are captured so undo restores
    /// that page at that position.
    DeletePage {
        index: usize,
        entry: PageEntry,
    },
    /// Pages from another PDF were appended. Undo removes exactly these ids.
    MergePages {
        file: String,
        entries: Vec<PageEntry>,
    },
    /// One arranger page was rotated.
    ///
    /// Both angles are captured, and undo restores `previous` absolutely, so a step of any
    /// size undoes correctly.
    RotatePage {
        id: u64,
        previous: crate::viewport::PageRotation,
        new: crate::viewport::PageRotation,
    },
}

impl UndoableAction {
    /// A short, user-facing label ("Add highlight", "Delete note", …).
    /// Used by the `"Undo <label>"` / `"Redo <label>"` tooltips.
    pub fn label(&self) -> &'static str {
        match self {
            UndoableAction::AddAnnotation { annotation } => match annotation.kind.as_str() {
                "highlight" => "highlight",
                "note" => "note",
                _ => "annotation",
            },
            UndoableAction::RemoveAnnotation { annotation } => match annotation.kind.as_str() {
                "highlight" => "highlight removal",
                "note" => "note removal",
                _ => "annotation removal",
            },
            UndoableAction::UpdateAnnotationText { .. }
            | UndoableAction::UpdateAnnotationNote { .. } => "edit",
            UndoableAction::UpdateAnnotationColor { .. } => "color change",
            UndoableAction::AddBookmark { .. } => "bookmark",
            UndoableAction::RemoveBookmark { .. } => "bookmark removal",
            UndoableAction::MovePage { .. } => "page move",
            UndoableAction::DeletePage { .. } => "page deletion",
            UndoableAction::MergePages { .. } => "merge",
            UndoableAction::RotatePage { .. } => "page rotation",
        }
    }
}

/// Bounded undo/redo history against an [`AnnotationsFile`].
///
/// Every user mutation is [`Self::record`]ed, which clears the redo stack.
/// [`Self::undo`] applies the reverse mutation and pushes onto the redo stack;
/// [`Self::redo`] is the mirror image.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<UndoableAction>,
    redo: Vec<UndoableAction>,
}

impl UndoStack {
    /// Cap on either stack: ample for a reading session while keeping memory to a few
    /// kilobytes.
    pub const CAP: usize = 50;

    pub fn new() -> Self {
        Self::default()
    }

    /// True when the undo stack has no entries. Drives the
    /// `Nothing to undo` message and the disabled state on the Undo button.
    pub fn undo_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// True when the redo stack has no entries. Drives the
    /// `Nothing to redo` message and the disabled state on the Redo button.
    pub fn redo_empty(&self) -> bool {
        self.redo.is_empty()
    }

    /// The label the tooltip would show for `"Undo <label>"`. `None` iff empty.
    pub fn peek_undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(UndoableAction::label)
    }
    /// The label the tooltip would show for `"Redo <label>"`. `None` iff empty.
    pub fn peek_redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(UndoableAction::label)
    }

    pub fn peek_undo_action(&self) -> Option<&UndoableAction> {
        self.undo.last()
    }

    pub fn peek_redo_action(&self) -> Option<&UndoableAction> {
        self.redo.last()
    }

    /// Record a fresh user-driven mutation, clearing the redo stack. Bounded at
    /// [`Self::CAP`]; overflow drops the oldest entry.
    pub fn record(&mut self, action: UndoableAction) {
        self.redo.clear();
        self.undo.push(action);
        if self.undo.len() > Self::CAP {
            // `remove(0)` fires only on overflow, shifting about 50 entries.
            self.undo.remove(0);
        }
    }

    /// Undo the most recent action and move it to the redo stack. Returns its label for
    /// `"Undo <label>"` feedback, or `None` without mutating anything when empty.
    pub fn undo(&mut self, target: UndoTarget<'_>) -> Option<&'static str> {
        let action = self.undo.pop()?;
        apply_reverse(&action, target);
        let label = action.label();
        self.push_redo(action);
        Some(label)
    }

    /// Redo the most recently undone action. Applies the forward mutation
    /// to `file` and moves the action back onto the undo stack.
    pub fn redo(&mut self, target: UndoTarget<'_>) -> Option<&'static str> {
        let action = self.redo.pop()?;
        apply_forward(&action, target);
        let label = action.label();
        self.push_undo(action);
        Some(label)
    }

    /// Clear both stacks, on document close and open, so undo cannot reach across
    /// documents.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    #[cfg(test)]
    pub(crate) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    // Cap-checked pushes for `undo()` and `redo()`, which must not clear the opposite
    // stack; only `record` does that.
    fn push_redo(&mut self, action: UndoableAction) {
        self.redo.push(action);
        if self.redo.len() > Self::CAP {
            self.redo.remove(0);
        }
    }
    fn push_undo(&mut self, action: UndoableAction) {
        self.undo.push(action);
        if self.undo.len() > Self::CAP {
            self.undo.remove(0);
        }
    }
}

fn apply_reverse(action: &UndoableAction, target: UndoTarget<'_>) {
    let file = target.annotations;
    let pages = target.pages;
    match action {
        UndoableAction::AddAnnotation { annotation } => {
            remove_annotation(file, annotation.id);
        }
        UndoableAction::RemoveAnnotation { annotation } => {
            restore_annotation(file, annotation.clone());
        }
        UndoableAction::UpdateAnnotationText { id, previous, .. } => {
            update_annotation_text(file, *id, previous);
        }
        UndoableAction::UpdateAnnotationNote { id, previous, .. } => {
            crate::plumbing::persistence::update_annotation_note(file, *id, previous);
        }
        UndoableAction::UpdateAnnotationColor { id, previous, .. } => {
            crate::plumbing::persistence::update_annotation_color(file, *id, previous);
        }
        UndoableAction::AddBookmark { record } => {
            remove_bookmark(file, record.page);
        }
        UndoableAction::RemoveBookmark { record } => {
            // Idempotent against a double restore.
            if !file.bookmarks.iter().any(|b| b.page == record.page) {
                file.bookmarks.push(record.clone());
                file.bookmarks.sort_by_key(|b| b.page);
            }
        }
        // Undo a move from the destination back to the origin: `move_index(to, from)`.
        UndoableAction::MovePage { from, to, .. } => {
            pages.move_index(*to, *from);
        }
        UndoableAction::DeletePage { index, entry } => {
            pages.insert(*index, entry.clone());
        }
        UndoableAction::MergePages { entries, .. } => {
            let ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
            pages.remove_ids(&ids);
        }
        // Restore the previous angle absolutely (see the variant's docs).
        UndoableAction::RotatePage { id, previous, .. } => {
            pages.set_rotation(*id, *previous);
        }
    }
}

fn apply_forward(action: &UndoableAction, target: UndoTarget<'_>) {
    let file = target.annotations;
    let pages = target.pages;
    match action {
        UndoableAction::AddAnnotation { annotation } => {
            restore_annotation(file, annotation.clone());
        }
        UndoableAction::RemoveAnnotation { annotation } => {
            remove_annotation(file, annotation.id);
        }
        UndoableAction::UpdateAnnotationText { id, new, .. } => {
            update_annotation_text(file, *id, new);
        }
        UndoableAction::UpdateAnnotationNote { id, new, .. } => {
            crate::plumbing::persistence::update_annotation_note(file, *id, new);
        }
        UndoableAction::UpdateAnnotationColor { id, new, .. } => {
            crate::plumbing::persistence::update_annotation_color(file, *id, new);
        }
        UndoableAction::AddBookmark { record } => {
            // Idempotent against a double redo.
            if !file.bookmarks.iter().any(|b| b.page == record.page) {
                // `toggle_bookmark` keeps the bookmarks sorted by page.
                toggle_bookmark(file, record.page, &record.label);
            }
        }
        UndoableAction::RemoveBookmark { record } => {
            remove_bookmark(file, record.page);
        }
        UndoableAction::MovePage { from, to, .. } => {
            pages.move_index(*from, *to);
        }
        UndoableAction::DeletePage { entry, .. } => {
            if let Some(id) = pages.position(entry.id).map(|_| entry.id) {
                pages.remove(id);
            }
        }
        UndoableAction::MergePages { entries, .. } => {
            pages.append_entries(entries);
        }
        UndoableAction::RotatePage { id, new, .. } => {
            pages.set_rotation(*id, *new);
        }
    }
}

/// Message shown when the user tries to undo with an empty stack.
///
/// Reachable via `Msg::Undo` when [`UndoStack::undo_empty`] is true.
pub const NOTHING_TO_UNDO: &str = "Nothing to undo";
/// Message shown when the user tries to redo with an empty stack.
pub const NOTHING_TO_REDO: &str = "Nothing to redo";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::PageArrangement;
    use crate::plumbing::persistence::add_annotation;

    #[test]
    fn undo_stack_is_bounded() {
        let mut stack = UndoStack::new();
        let mut file = AnnotationsFile::default();
        for i in 0..UndoStack::CAP + 5 {
            let id = add_annotation(&mut file, 0, "note", &format!("n{i}"), "");
            let ann = file.annotations.last().unwrap().clone();
            stack.record(UndoableAction::AddAnnotation { annotation: ann });
            let _ = id;
        }
        assert_eq!(stack.undo_len(), UndoStack::CAP, "cap must fire");
        stack.undo(UndoTarget::new(&mut file, &mut PageArrangement::default()));
        assert!(!stack.redo_empty());
        stack.record(UndoableAction::AddAnnotation {
            annotation: file.annotations.last().unwrap().clone(),
        });
        assert!(stack.redo_empty());
    }
}
