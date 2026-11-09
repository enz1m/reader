//! Virtual-list track provider.

use super::*;

// One virtual-list item: a bounded horizontal track of one or two books.
// Indices point back into the persistent projection, so realising a viewport
// clones only this tiny descriptor rather than cloning its book records.
#[derive(Debug, Clone)]
pub(crate) struct LibraryTrack {
    pub(crate) start: usize,
    pub(crate) len: usize,
    key: SequenceKey,
}

// Responsive adapter over the persistent, sorted projection. Grouping lives
// at the view edge because it is layout policy; filtering/sorting remains the
// model's one source of truth. The adapter allocates nothing per shelf item.
pub(crate) struct LibraryTracks<'a> {
    pub(crate) books: &'a [RecentBook],
    pub(crate) columns: usize,
}

impl LibraryTracks<'_> {
    pub(crate) fn track_count(&self) -> usize {
        // Pair from the trailing edge. If one book is inserted at the front of
        // an even shelf, it forms a leading singleton and every existing pair
        // keeps its key/child state (notably keyboard focus).
        let leading = self.books.len() % self.columns;
        usize::from(leading > 0) + (self.books.len() - leading) / self.columns
    }

    pub(crate) fn bounds(&self, index: usize) -> Option<(usize, usize)> {
        let leading = self.books.len() % self.columns;
        if leading > 0 {
            if index == 0 {
                return Some((0, leading));
            }
            let start = leading.checked_add((index - 1).checked_mul(self.columns)?)?;
            (start < self.books.len()).then_some((start, self.columns))
        } else {
            let start = index.checked_mul(self.columns)?;
            (start < self.books.len()).then_some((start, self.columns))
        }
    }

    fn track(&self, index: u64) -> Option<LibraryTrack> {
        let (start, len) = self.bounds(usize::try_from(index).ok()?)?;
        // A track is logically the exact ordered group it contains. Including
        // every path avoids handing retained state to a different pair after a
        // filter/sort/mutation; the book controls themselves retain path ids.
        let mut key = String::from("reader.library.track");
        for book in &self.books[start..start + len] {
            key.push('\0');
            key.push_str(&book.path);
        }
        Some(LibraryTrack {
            start,
            len,
            key: SequenceKey::new(key),
        })
    }
}

impl ItemProvider<LibraryTrack> for LibraryTracks<'_> {
    fn len(&self) -> u64 {
        self.track_count() as u64
    }

    fn item(&self, index: u64) -> Option<LibraryTrack> {
        self.track(index)
    }
}

impl KeyedProvider<LibraryTrack> for LibraryTracks<'_> {
    fn key(&self, index: u64) -> Option<SequenceKey> {
        self.track(index).map(|track| track.key)
    }
}
