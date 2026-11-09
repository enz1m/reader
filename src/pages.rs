//! Stable page identities, source references and arranger order.
//!
//! Page identities survive moves, deletion and undo.

/// Where a page in the arrangement came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageSource {
    /// Page `index` (0-based) of the document that is currently open. The
    /// view resolves a rendered thumbnail from `Model::pages[index]`.
    Original(usize),
    /// Page `index` (0-based) of another PDF merged in from `file`. There is
    /// no rendered bitmap for these, so the view draws a labelled placeholder
    /// rather than pretending it has a thumbnail.
    Merged {
        file: String,
        index: usize,
        document_fingerprint: String,
    },
}

/// One page in the arranger, carrying a **stable id** that survives reorder,
/// delete and undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEntry {
    /// Stable identity, never reused within one arrangement, so reorder keys and undo
    /// records stay valid.
    pub id: u64,
    pub source: PageSource,
    /// This page's own rotation, independent of every other page and of the reading
    /// pane's global [`crate::viewport::PageRotation`].
    pub rotation: crate::viewport::PageRotation,
    /// The user-facing row label, fixed at construction so it does not renumber when the
    /// page moves.
    pub label: String,
}

impl PageEntry {
    /// The key handed to `enz_reorder_list`, prefixed so it cannot collide with another
    /// keyed collection in the same drag host.
    pub fn key(&self) -> String {
        format!("page-{}", self.id)
    }
}

/// Recover a [`PageEntry::id`] from the key an `enz_reorder_list`
/// [`DropRequest`] carries back.
///
/// A drop publishes only stable ids, so this is the one place an id is parsed back.
///
/// [`DropRequest`]: enzui_iced_core::drag::DropRequest
pub fn id_from_key(key: &str) -> Option<u64> {
    key.strip_prefix("page-")?.parse().ok()
}

/// The ordered list of pages the arranger edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageArrangement {
    entries: Vec<PageEntry>,
    next_id: u64,
}

impl PageArrangement {
    /// Build the identity arrangement for a document with `page_count` pages.
    pub fn from_page_count(page_count: usize) -> Self {
        let mut out = Self::default();
        for i in 0..page_count {
            let id = out.next_id;
            out.next_id += 1;
            out.entries.push(PageEntry {
                id,
                source: PageSource::Original(i),
                label: format!("Page {}", i + 1),
                rotation: crate::viewport::PageRotation::NONE,
            });
        }
        out
    }

    pub fn entries(&self) -> &[PageEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current index of the entry with `id`, if it is still present.
    pub fn position(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// Rotate the page with `id` one quarter turn, and report the rotation it
    /// now carries.
    ///
    /// `None` for an unknown id, which is a no-op. Four presses return the page to
    /// upright.
    pub fn rotate(&mut self, id: u64, clockwise: bool) -> Option<crate::viewport::PageRotation> {
        let index = self.position(id)?;
        let entry = self.entries.get_mut(index)?;
        entry.rotation = if clockwise {
            entry.rotation.clockwise()
        } else {
            entry.rotation.counter_clockwise()
        };
        Some(entry.rotation)
    }

    /// Set the page with `id` to an explicit rotation, as undo of a rotate does.
    pub fn set_rotation(&mut self, id: u64, rotation: crate::viewport::PageRotation) -> bool {
        match self.position(id).and_then(|i| self.entries.get_mut(i)) {
            Some(entry) => {
                entry.rotation = rotation;
                true
            }
            None => false,
        }
    }

    /// The rotation the page with `id` currently carries, if it is present.
    pub fn rotation_of(&self, id: u64) -> Option<crate::viewport::PageRotation> {
        self.position(id)
            .and_then(|i| self.entries.get(i))
            .map(|e| e.rotation)
    }

    pub fn get(&self, index: usize) -> Option<&PageEntry> {
        self.entries.get(index)
    }

    /// Whether `Move up` is available for the entry at `index`.
    ///
    /// The first row cannot move up; the view disables the control rather than publish a
    /// no-op.
    pub fn can_move_up(&self, index: usize) -> bool {
        index > 0 && index < self.entries.len()
    }

    /// Whether `Move down` is available for the entry at `index`.
    pub fn can_move_down(&self, index: usize) -> bool {
        !self.entries.is_empty() && index + 1 < self.entries.len()
    }

    /// Move the entry with `id` one position earlier.
    ///
    /// Returns the `(from, to)` pair that changed, or `None` for an unknown id or the
    /// first row. A boundary move never wraps.
    pub fn move_up(&mut self, id: u64) -> Option<(usize, usize)> {
        let from = self.position(id)?;
        if from == 0 {
            return None;
        }
        self.entries.swap(from, from - 1);
        Some((from, from - 1))
    }

    /// Move the entry with `id` one position later. Same no-op contract as
    /// [`Self::move_up`] at the last row.
    pub fn move_down(&mut self, id: u64) -> Option<(usize, usize)> {
        let from = self.position(id)?;
        if from + 1 >= self.entries.len() {
            return None;
        }
        self.entries.swap(from, from + 1);
        Some((from, from + 1))
    }

    /// Move the entry with `id` into gap `slot`, the `enz_reorder_list` drop coordinate,
    /// where `slot == len` appends.
    ///
    /// A gap past the moved item shifts left by one once the item is pulled out.
    /// Returns the `(from, to)` pair,
    /// or `None` when nothing moved.
    pub fn move_to_slot(&mut self, id: u64, slot: usize) -> Option<(usize, usize)> {
        let from = self.position(id)?;
        let clamped = slot.min(self.entries.len());
        let to = if clamped > from { clamped - 1 } else { clamped };
        if to == from {
            return None;
        }
        self.move_index(from, to);
        Some((from, to))
    }

    /// Raw index move, used by [`Self::move_to_slot`] and by undo, which knows both
    /// endpoints.
    pub fn move_index(&mut self, from: usize, to: usize) -> bool {
        if from >= self.entries.len() || to >= self.entries.len() || from == to {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        true
    }

    /// Remove the entry with `id`, returning where it was and what it was.
    ///
    /// The whole entry comes back so undo of a delete restores that exact page.
    pub fn remove(&mut self, id: u64) -> Option<(usize, PageEntry)> {
        let index = self.position(id)?;
        let entry = self.entries.remove(index);
        Some((index, entry))
    }

    /// Re-insert `entry` at `index` (undo of a delete). An out-of-range index
    /// appends rather than panicking.
    pub fn insert(&mut self, index: usize, entry: PageEntry) {
        let at = index.min(self.entries.len());
        // Never let a restored id alias a later-allocated one.
        self.next_id = self.next_id.max(entry.id + 1);
        self.entries.insert(at, entry);
    }

    /// Append `count` pages from `file`, returning the entries that were
    /// added so undo can remove exactly those and redo can restore them with
    /// their original ids.
    pub fn append_merged(
        &mut self,
        file: &str,
        count: usize,
        document_fingerprint: &str,
    ) -> Vec<PageEntry> {
        let mut added = Vec::with_capacity(count);
        let display_name = std::path::Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(file);
        for i in 0..count {
            let id = self.next_id;
            self.next_id += 1;
            let entry = PageEntry {
                id,
                source: PageSource::Merged {
                    file: file.to_string(),
                    index: i,
                    document_fingerprint: document_fingerprint.to_string(),
                },
                label: format!("{display_name} — page {}", i + 1),
                rotation: crate::viewport::PageRotation::NONE,
            };
            self.entries.push(entry.clone());
            added.push(entry);
        }
        added
    }

    /// Remove every entry whose id appears in `ids` (undo of a merge).
    /// Returns how many were actually removed.
    pub fn remove_ids(&mut self, ids: &[u64]) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !ids.contains(&e.id));
        before - self.entries.len()
    }

    /// Re-append `entries` verbatim (redo of a merge), preserving ids.
    pub fn append_entries(&mut self, entries: &[PageEntry]) {
        for e in entries {
            self.next_id = self.next_id.max(e.id + 1);
            self.entries.push(e.clone());
        }
    }

    /// The source page indices of every [`PageSource::Original`] entry, in
    /// arrangement order, as consumed when writing the document out.
    pub fn original_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .filter_map(|e| match &e.source {
                PageSource::Original(i) => Some(*i),
                PageSource::Merged { .. } => None,
            })
            .collect()
    }

    /// Whether the draft still equals the source page tree, including each
    /// page's rotation. Drives the arranger's “unchanged” state.
    pub fn is_identity(&self, page_count: usize) -> bool {
        self.entries.len() == page_count
            && self.entries.iter().enumerate().all(|(i, e)| {
                e.source == PageSource::Original(i)
                    && e.rotation == crate::viewport::PageRotation::NONE
            })
    }
}
