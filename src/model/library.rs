//! Model annotations, library, EPUB, and metadata.

use super::types::library_sort_key;
use super::*;

impl Model {
    /// Annotations matching the current [`Self::annotation_filter`], in file
    /// order. Case-insensitive substring match over the annotation text, note
    /// body, kind and style. An empty filter returns every annotation.
    ///
    /// Returns borrowed references so the caller (the view) does not clone.
    pub fn filtered_annotations(&self) -> Vec<&crate::plumbing::persistence::Annotation> {
        let needle = self.annotation_filter.trim().to_lowercase();
        self.annotations
            .annotations
            .iter()
            .filter(|a| {
                if needle.is_empty() {
                    return true;
                }
                a.text.to_lowercase().contains(&needle)
                    || a.note.to_lowercase().contains(&needle)
                    || a.kind.to_lowercase().contains(&needle)
                    || a.style.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// The library books that match the current [`Self::library_query`],
    /// ordered by the current [`Self::library_sort`].
    ///
    /// Case-insensitive substring filter over `title` and `author`, then the sort; an
    /// empty query returns every book. Returns references so the view does not clone the
    /// library each frame. [`Self::library`] is already newest-first, so
    /// [`LibrarySort::Recent`] is the identity order.
    pub fn library_books(&self) -> Vec<&RecentBook> {
        let needle = self.library_query.trim().to_lowercase();
        let mut books: Vec<&RecentBook> = self
            .library
            .iter()
            .filter(|b| {
                if needle.is_empty() {
                    return true;
                }
                b.title.to_lowercase().contains(&needle)
                    || b.author.to_lowercase().contains(&needle)
            })
            .collect();
        match self.library_sort {
            // `library` is already newest-first; keep that order verbatim.
            LibrarySort::Recent => {}
            LibrarySort::Title => {
                books.sort_by_key(|a| library_sort_key(&a.title));
            }
            LibrarySort::Author => {
                books.sort_by_key(|a| library_sort_key(&a.author));
            }
        }
        books
    }

    /// Rebuild the retained provider after a library/filter/sort mutation.
    pub fn rebuild_library_projection(&mut self) {
        self.library_projection =
            LibraryProjection::compute(&self.library, &self.library_query, self.library_sort);
        // Invalidate the derived row-height cache. Any change to the
        // realised set of books (or their order/filtering) can change how many
        // lines a title/caption wraps to, so the height table must be reshaped.
        self.library_revision = self.library_revision.wrapping_add(1);
    }

    /// Apply availability results to the recents/library/projection rows in
    /// place, without re-sorting or cloning the whole history (availability is
    /// not a sort key). Bumps [`Self::library_revision`] when a `file_missing`
    /// flag changed, so the row-height cache reshapes for the missing caption,
    /// but leaves [`Self::order_revision`] untouched so an in-flight scan keeps
    /// its pinned ordering.
    pub fn apply_availability_flags(&mut self, updates: &std::collections::HashMap<String, bool>) {
        let mut changed = false;
        for row in self.recents.iter_mut().chain(self.library.iter_mut()) {
            if let Some(&exists) = updates.get(&row.path) {
                let missing = !exists;
                if row.file_missing != missing {
                    row.file_missing = missing;
                    changed = true;
                }
            }
        }
        changed |= self.library_projection.apply_availability(updates);
        if changed {
            self.library_revision = self.library_revision.wrapping_add(1);
        }
    }

    /// Apply durable bookmark counts without changing library order.
    pub fn apply_bookmark_counts(&mut self, updates: &std::collections::HashMap<String, usize>) {
        let mut changed = false;
        for row in self.recents.iter_mut().chain(self.library.iter_mut()) {
            if let Some(&count) = updates.get(&row.path)
                && row.bookmark_count != count
            {
                row.bookmark_count = count;
                changed = true;
            }
        }
        changed |= self.library_projection.apply_bookmark_counts(updates);
        if changed {
            self.library_revision = self.library_revision.wrapping_add(1);
        }
    }

    /// The provider borrowed by the virtualised library view.
    pub fn library_rows(&self) -> &LibraryProjection {
        &self.library_projection
    }

    /// EPUB chapter count, or 0 when no EPUB is open.
    pub fn epub_chapter_count(&self) -> usize {
        self.epub
            .as_ref()
            .map(|e| e.load.chapters.len())
            .unwrap_or(0)
    }

    /// The reader-facing reading position as `(current, total)`, both 1-based and counting
    /// only content chapters, as the titlebar shows. `None` when no EPUB is open.
    pub fn epub_reading_position(&self) -> Option<(usize, usize)> {
        let epub = self.epub.as_ref()?;
        crate::plumbing::epub_support::reading_position(
            &epub.load.chapters,
            self.epub_current_chapter,
        )
    }

    /// The spine index of the previous content chapter, or `None` at the first one, so
    /// "previous" never lands on excluded front matter. Front matter stays reachable from
    /// the ToC via `GoToChapter`.
    pub fn epub_prev_content_chapter(&self) -> Option<usize> {
        let epub = self.epub.as_ref()?;
        crate::plumbing::epub_support::prev_content_chapter(
            &epub.load.chapters,
            self.epub_current_chapter,
        )
    }

    /// The spine index of the next CONTENT chapter, or `None` at the last one.
    /// Mirror of [`Model::epub_prev_content_chapter`].
    pub fn epub_next_content_chapter(&self) -> Option<usize> {
        let epub = self.epub.as_ref()?;
        crate::plumbing::epub_support::next_content_chapter(
            &epub.load.chapters,
            self.epub_current_chapter,
        )
    }

    /// Clamp font size to a sane reading range.
    pub fn clamp_font_size(&mut self) {
        self.epub_font_size = self.epub_font_size.clamp(10, 40);
    }

    /// Whether the page arranger is available for the open document.
    ///
    /// PDF only: an EPUB is a reflowable spine of chapters with no page tree
    /// to rearrange, and a CBZ is an image archive. The toolbar hides the
    /// control rather than showing one that reports "unsupported" on click.
    pub fn page_editing_supported(&self) -> bool {
        self.screen == Screen::Reading && self.format == Format::Pdf
    }

    /// Whether the metadata / properties dialogs are available. Same rule as
    /// [`Self::page_editing_supported`] — `/Info` is a PDF structure.
    pub fn metadata_supported(&self) -> bool {
        self.page_editing_supported()
    }

    /// Whether the metadata dialog has unsaved edits.
    pub fn metadata_is_dirty(&self) -> bool {
        self.metadata != self.metadata_baseline
    }

    /// Whether the draft is changed and every explicitly edited date is valid.
    pub fn metadata_can_save(&self) -> bool {
        self.metadata_is_dirty() && !self.metadata.has_invalid_date_edit()
    }

    /// Apply an edit to one metadata field. `value` is stored verbatim;
    /// date removal is authorized separately when an edited value is empty.
    pub fn edit_metadata(&mut self, field: MetadataField, value: String) {
        field.set(&mut self.metadata, value);
        if self.metadata_save_failure.take().is_some() {
            self.error = None;
        }
    }

    /// Forget everything about a password prompt: the pending path, the typed
    /// secret, the reveal flag and the error.
    ///
    /// One method, so no exit path can leave the typed password in memory or in the next
    /// document's prompt. It also invalidates any in-flight unlock attempt.
    pub fn clear_password(&mut self) {
        self.password_prompt = None;
        self.password_input.clear();
        self.password_revealed = false;
        self.password_error = None;
        self.password_unlocking = false;
        self.password_attempt = self.password_attempt.wrapping_add(1);
    }

    /// Whether zoom / rotation controls apply to the open document.
    ///
    /// PDF and CBZ pages can be scaled and turned; an EPUB zooms through font size and its
    /// flowing text does not rotate, so the toolbar hides these controls for it.
    pub fn page_view_controls_supported(&self) -> bool {
        self.screen == Screen::Reading && matches!(self.format, Format::Pdf | Format::Cbz)
    }
}
