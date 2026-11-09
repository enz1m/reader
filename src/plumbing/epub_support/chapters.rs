//! EPUB chapters and content navigation.

/// Represents a chapter/section from the EPUB spine.
#[derive(Clone, Debug)]
pub struct EpubChapter {
    /// The XHTML document `<title>` metadata, verbatim (often the chapter name with the
    /// book name appended). Never drawn as a heading; kept for identity, the TOC fallback
    /// and diagnostics.
    pub title: String,
    /// The reader-visible chapter heading: the TOC label, a visible body heading, or the
    /// cleaned `<title>` metadata. Empty means "fall back to [`Self::title`]", resolved by
    /// [`Self::display_heading`]. Set by `load_epub`.
    pub heading: String,
    pub content_html: String,
    pub plain_text: String,
    pub index: usize,
}

impl EpubChapter {
    /// The string the reading pane draws as the chapter heading.
    ///
    /// [`Self::heading`] when set, else [`Self::title`], which synthetic test chapters
    /// rely on; `load_epub` always fills `heading`.
    pub fn display_heading(&self) -> &str {
        if self.heading.is_empty() {
            &self.title
        } else {
            &self.heading
        }
    }
}

/// Readable chapter indices excluding short front matter: at least 25% of the longest chapter
/// and 400 characters, with nonempty/spine fallback.
pub fn content_chapter_indices(chapters: &[EpubChapter]) -> Vec<usize> {
    if chapters.is_empty() {
        return Vec::new();
    }
    let lengths: Vec<usize> = chapters
        .iter()
        .map(|c| c.plain_text.chars().count())
        .collect();
    let max_len = lengths.iter().copied().max().unwrap_or(0);
    if max_len == 0 {
        return Vec::new();
    }
    let threshold = (max_len / 4).max(400);
    let by_threshold: Vec<usize> = chapters
        .iter()
        .filter(|c| c.plain_text.chars().count() >= threshold)
        .map(|c| c.index)
        .collect();
    if !by_threshold.is_empty() {
        return by_threshold;
    }
    // Nothing cleared the threshold: fall back to every non-empty chapter so a
    // book of uniformly short chapters still yields a sensible "N of M".
    let non_empty: Vec<usize> = chapters
        .iter()
        .filter(|c| !c.plain_text.is_empty())
        .map(|c| c.index)
        .collect();
    if !non_empty.is_empty() {
        non_empty
    } else {
        chapters.iter().map(|c| c.index).collect()
    }
}
