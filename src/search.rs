//! Immutable EPUB/PDF search snapshots with generation-checked results.
//!
//! Queries run on blocking workers; older results cannot replace a newer query.

use std::sync::Arc;

use crate::plumbing::epub_support::{EpubHit, find_hits_in};

pub const MAX_PDF_SEARCH_HITS: usize = 2_048;

#[derive(Clone, Debug, PartialEq)]
pub struct PdfSearchHit {
    pub page: usize,
    pub line: usize,
    pub preview: String,
    pub rect: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct PdfSearchIndex {
    pages: Arc<Vec<crate::pdf::PdfPageText>>,
}

impl PdfSearchIndex {
    pub fn build(pages: Vec<crate::pdf::PdfPageText>) -> Self {
        Self {
            pages: Arc::new(pages),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn run(&self, query: &str) -> Vec<PdfSearchHit> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (page, text) in self.pages.iter().enumerate() {
            for (line, item) in text.lines.iter().enumerate() {
                let haystack = item.text.to_lowercase();
                for _ in haystack.match_indices(&needle) {
                    hits.push(PdfSearchHit {
                        page,
                        line,
                        preview: item.text.clone(),
                        rect: item.rect,
                    });
                    if hits.len() == MAX_PDF_SEARCH_HITS {
                        return hits;
                    }
                }
            }
        }
        hits
    }
}

// One chapter's searchable text, owned so the index does not borrow the model.
#[derive(Clone, Debug)]
struct IndexedChapter {
    /// The chapter's spine index (what an [`EpubHit::chapter`] reports).
    index: usize,
    /// The chapter's plain reading text, exactly as `find_hits` scans it. Char
    /// offsets in a hit index into this string.
    plain_text: String,
}

/// Immutable, cheaply cloned chapter text for worker searches; built once per document load.
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    chapters: Arc<Vec<IndexedChapter>>,
    /// A test-only barrier the scan waits on, proving the reducer returned first. `None` in
    /// production. A field rather than a `#[cfg(test)]` global, so parallel tests cannot block
    /// each other.
    barrier: Option<Arc<std::sync::Barrier>>,
}

impl SearchIndex {
    /// Build the index from the freshly-loaded chapters. Called once per open.
    pub fn build(chapters: &[crate::plumbing::epub_support::EpubChapter]) -> Self {
        Self {
            chapters: Arc::new(
                chapters
                    .iter()
                    .map(|c| IndexedChapter {
                        index: c.index,
                        plain_text: c.plain_text.clone(),
                    })
                    .collect(),
            ),
            barrier: None,
        }
    }

    #[cfg(test)]
    pub fn with_barrier(
        chapters: &[crate::plumbing::epub_support::EpubChapter],
        barrier: Arc<std::sync::Barrier>,
    ) -> Self {
        let mut idx = Self::build(chapters);
        idx.barrier = Some(barrier);
        idx
    }

    /// Whether the index holds any chapters (an empty index is the non-EPUB /
    /// no-document state).
    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    /// Run the full-text scan for `query`, returning every hit in reading
    /// order. Blank query → no hits. This is what the search `Task` awaits; it
    /// never touches the model.
    pub fn run(&self, query: &str) -> Vec<EpubHit> {
        // Test-only: block until the test releases the barrier. In production
        // `barrier` is always `None`, so this is a no-op.
        if let Some(barrier) = &self.barrier {
            barrier.wait();
        }
        let needle = query.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        find_hits_in(
            self.chapters
                .iter()
                .map(|c| (c.index, c.plain_text.as_str())),
            needle,
        )
    }
}
