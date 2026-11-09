//! Model search and pagination.

use super::*;

impl Model {
    /// Record a new query and open the debounce window.
    ///
    /// Does not scan: it updates the query, resets the selected result and bumps the
    /// generation so in-flight scans go stale, then marks a scan pending for a searchable
    /// query. A blank query, or a document without an index, clears the hits.
    #[must_use]
    pub fn begin_search(&mut self, query: String) -> bool {
        self.search_query = query;
        self.search_active = 0;
        // The previous results stay listed until the new ones arrive, so the
        // list does not blank and refill on every keystroke.
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_in_flight_generation = None;
        let searchable = match self.format {
            Format::Epub => self.epub.is_some(),
            Format::Pdf => !self.pdf_search_index.is_empty(),
            Format::Cbz => false,
        } && !self.search_query.trim().is_empty();
        if searchable {
            self.search_pending = true;
            true
        } else {
            self.search_hits.clear();
            self.pdf_search_hits.clear();
            self.epub_search_truncated = false;
            self.search_pending = false;
            false
        }
    }

    /// Apply the hits produced by an off-thread scan, keeping the selection on
    /// the first result. Called from the reducer only when the scan's
    /// generation is still current.
    pub fn apply_search_hits(&mut self, hits: Vec<crate::plumbing::epub_support::EpubHit>) {
        self.search_hits = hits;
        self.pdf_search_hits.clear();
        self.search_active = 0;
    }

    pub fn apply_pdf_search_hits(&mut self, hits: Vec<crate::search::PdfSearchHit>) {
        self.pdf_search_hits = hits;
        self.search_hits.clear();
        self.search_active = 0;
    }

    pub fn search_hit_count(&self) -> usize {
        match self.format {
            Format::Epub => self.search_hits.len(),
            Format::Pdf => self.pdf_search_hits.len(),
            Format::Cbz => 0,
        }
    }

    pub fn active_pdf_search_hit(&self) -> Option<&crate::search::PdfSearchHit> {
        (self.format == Format::Pdf)
            .then(|| self.pdf_search_hits.get(self.search_active))
            .flatten()
    }

    /// The currently-selected search hit, if any. This is both the result the
    /// list highlights and the passage the transient in-content highlight is
    /// drawn over.
    pub fn active_hit(&self) -> Option<&crate::plumbing::epub_support::EpubHit> {
        self.search_hits.get(self.search_active)
    }

    /// The transient search highlight for `chapter`, if the selected hit falls in it.
    ///
    /// A view-time projection that vanishes when the search clears; it never touches
    /// [`Self::annotations`]. Persisting a match is the separate `Highlight this match`
    /// action.
    pub fn transient_search_highlight(
        &self,
        chapter: usize,
    ) -> Option<crate::plumbing::epub_support::EpubHighlightRange> {
        let hit = self.active_hit()?;
        if hit.chapter != chapter {
            return None;
        }
        Some(crate::plumbing::epub_support::EpubHighlightRange {
            chapter: hit.chapter,
            char_start: hit.char_start,
            char_end: hit.char_end,
            color: "yellow".to_string(),
            style: "highlight".to_string(),
        })
    }

    /// Project persisted EPUB highlights for `chapter` into the renderer's char-offset
    /// range type; search matches stay in [`Self::transient_search_highlight`].
    pub fn persisted_epub_highlights(
        &self,
        chapter: usize,
    ) -> Vec<crate::plumbing::epub_support::EpubHighlightRange> {
        self.annotations
            .annotations
            .iter()
            .filter(|ann| ann.kind == "highlight")
            .filter_map(|ann| {
                let crate::plumbing::persistence::AnnotationPosition::EpubCfi {
                    chapter: anchored_chapter,
                    char_start,
                    char_end,
                } = crate::plumbing::persistence::AnnotationPosition::parse(
                    &ann.position,
                    ann.page,
                )
                else {
                    return None;
                };
                (anchored_chapter == chapter && char_start < char_end).then(|| {
                    crate::plumbing::epub_support::EpubHighlightRange {
                        chapter: anchored_chapter,
                        char_start,
                        char_end,
                        color: if ann.color.is_empty() {
                            "yellow".to_string()
                        } else {
                            ann.color.clone()
                        },
                        style: if ann.style.is_empty() {
                            "highlight".to_string()
                        } else {
                            ann.style.clone()
                        },
                    }
                })
            })
            .collect()
    }

    /// Convert the reading scope's paragraph-local selection into the open
    /// EPUB chapter's stable plain-text character space.
    ///
    /// Paginated mode passes fragment slices to the scope, so the paragraph index resolves
    /// through the slice the view paints and `ReadingParagraph::char_start` lifts it to
    /// chapter coordinates. No screen pixels reach persistence.
    pub fn active_epub_selection_anchor(&self) -> Option<(usize, usize, usize, String)> {
        if self.format != Format::Epub {
            return None;
        }
        let selection = self.active_selection.as_ref()?;
        if selection.text.trim().is_empty() {
            return None;
        }
        let paragraphs = if self.epub_is_paginated() {
            self.epub_page_content.as_slice()
        } else {
            self.epub
                .as_ref()?
                .chapter_paragraphs
                .get(&self.epub_current_chapter)?
                .as_slice()
        };
        let start_para = paragraphs.get(selection.start.paragraph)?;
        let end_para = paragraphs.get(selection.end.paragraph)?;
        if start_para.chapter != self.epub_current_chapter
            || end_para.chapter != self.epub_current_chapter
        {
            return None;
        }
        let start =
            start_para.char_start + selection.start.offset.min(start_para.text.chars().count());
        let end = end_para.char_start + selection.end.offset.min(end_para.text.chars().count());
        (start < end).then(|| {
            (
                self.epub_current_chapter,
                start,
                end,
                selection.text.clone(),
            )
        })
    }

    /// Advance the active-match cursor by `delta` (±1), wrapping at both ends.
    /// No-op when there are no hits.
    pub fn step_search(&mut self, forward: bool) {
        let n = self.search_hit_count();
        if n == 0 {
            return;
        }
        self.search_active = if forward {
            (self.search_active + 1) % n
        } else {
            (self.search_active + n - 1) % n
        };
    }

    /// The current page (PDF/CBZ) or chapter (EPUB) index, 0-based. Used as the
    /// default page for a note added from the annotations panel.
    pub fn current_position(&self) -> usize {
        match self.format {
            Format::Epub => self.epub_current_chapter,
            Format::Pdf | Format::Cbz => self.current_page,
        }
    }

    /// Whether the reading surface is currently laid out as pages.
    ///
    /// Requires both the paginated mode and a chapter that produced pages, so an empty
    /// chapter never shows "page 1 of 0".
    pub fn epub_is_paginated(&self) -> bool {
        self.format == Format::Epub
            && self.epub_view_mode == EpubViewMode::Paginated
            && !self.epub_pages.is_empty()
    }

    /// The 0-based index of the page containing [`Self::epub_page_offset`].
    ///
    /// Derived, never stored (see [`Self::epub_page_offset`]).
    pub fn epub_page_index(&self) -> usize {
        crate::pagination::page_for_offset(&self.epub_pages, self.epub_page_offset)
    }

    /// How many pages the current chapter occupies at the current metrics.
    pub fn epub_page_count(&self) -> usize {
        self.epub_pages.len()
    }

    /// The span of the page on screen, if the surface is paginated.
    pub fn epub_current_page_span(&self) -> Option<crate::pagination::PageSpan> {
        if !self.epub_is_paginated() {
            return None;
        }
        self.epub_pages.get(self.epub_page_index()).copied()
    }
}
