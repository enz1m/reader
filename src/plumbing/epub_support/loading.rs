//! EPUB loading.

use super::toc::{
    build_toc_from_nav, choose_chapter_heading, extract_first_body_heading, extract_html_title,
};
use super::*;

/// Load and parse an EPUB file.
///
/// # Errors
/// Returns a bounded user-facing refusal for an unreadable, damaged or oversized book.
pub fn load_epub(path: &str) -> Result<EpubLoadResult, String> {
    load_epub_bounded(path, 8_192, 256 * 1024 * 1024)
}

fn load_epub_bounded(
    path: &str,
    max_chapters: usize,
    max_content_bytes: usize,
) -> Result<EpubLoadResult, String> {
    let mut doc = super::archive::open(path)?;
    if doc.spine.len() > max_chapters {
        return Err("The EPUB contains too many chapter references.".into());
    }
    let mut content_bytes = 0usize;

    let metadata = EpubMetadata {
        title: doc
            .mdata("title")
            .map(|m| m.value.clone())
            .unwrap_or_default(),
        author: doc
            .mdata("creator")
            .map(|m| m.value.clone())
            .unwrap_or_default(),
        language: doc
            .mdata("language")
            .map(|m| m.value.clone())
            .unwrap_or_default(),
        description: doc
            .mdata("description")
            .map(|m| m.value.clone())
            .unwrap_or_default(),
        publisher: doc
            .mdata("publisher")
            .map(|m| m.value.clone())
            .unwrap_or_default(),
    };

    let spine_items: Vec<epub::doc::SpineItem> = doc.spine.clone();

    // Nav labels are the reader-visible chapter names, unlike `<title>` metadata. Build
    // the nav TOC once; `or_insert` keeps the first, top-level label per chapter.
    let spine_paths = spine_items
        .iter()
        .map(|item| {
            doc.resources
                .get(&item.idref)
                .map(|resource| resource.path.clone())
                .ok_or_else(|| "The EPUB spine references a missing resource.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let nav_toc: Vec<EpubTocEntry> = if !doc.toc.is_empty() {
        build_toc_from_nav(&doc.toc, &spine_paths)
    } else {
        Vec::new()
    };
    let mut toc_labels: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    for entry in &nav_toc {
        toc_labels
            .entry(entry.chapter_index)
            .or_insert_with(|| entry.title.clone());
    }

    let mut chapters = Vec::new();
    let mut per_chapter_footnotes = Vec::new();
    let mut chapter_paths: Vec<String> = Vec::new();

    for (idx, spine_item) in spine_items.iter().enumerate() {
        // Footnotes resolve cross-chapter links against these archive paths.
        let path_str = doc
            .resources
            .get(&spine_item.idref)
            .map(|ri| ri.path.to_string_lossy().to_string())
            .unwrap_or_else(|| spine_item.idref.clone());
        chapter_paths.push(path_str);

        let (data, _mime) = doc
            .get_resource(&spine_item.idref)
            .ok_or_else(|| format!("Could not read EPUB chapter {}.", idx + 1))?;
        let content = String::from_utf8_lossy(&data);
        // A spine may reference the same archive resource repeatedly.
        content_bytes = content_bytes
            .checked_add(content.len())
            .filter(|total| *total <= max_content_bytes)
            .ok_or_else(|| "The EPUB exceeds the decoded chapter-content limit.".to_string())?;
        let content = content.into_owned();

        // `<title>` is kept for identity and diagnostics, never rendered as a heading.
        let meta_title = extract_html_title(&content);
        let title = meta_title
            .clone()
            .unwrap_or_else(|| format!("Chapter {}", idx + 1));
        // The reader-visible heading: nav label → real body heading → cleaned
        // metadata. This is what the reading pane draws.
        let body_heading = extract_first_body_heading(&content);
        let heading = choose_chapter_heading(
            toc_labels.get(&idx).map(String::as_str),
            body_heading.as_deref(),
            meta_title.as_deref(),
            idx,
        );
        // The body usually starts with the heading too, so the redundant leading line is
        // stripped below. Footnote bodies are removed from the reading flow.
        let body_html = strip_footnote_bodies(&content);
        let plain_text = strip_redundant_leading_heading(&html_to_plain_text(&body_html), &heading);
        per_chapter_footnotes.push(extract_from_html(&content, idx));

        chapters.push(EpubChapter {
            title,
            heading,
            content_html: content,
            plain_text,
            index: idx,
        });
    }

    let footnote_refs: Vec<FootnoteRef> = per_chapter_footnotes
        .iter()
        .flat_map(|(refs, _notes)| refs.clone())
        .collect();

    let footnotes = resolve_all(&per_chapter_footnotes, &chapter_paths);

    // Build TOC. Reuse the nav TOC computed above when the book ships one;
    // otherwise fall back to the reader-visible chapter headings (not the
    // `<title>` metadata) so the outline and the pane heading agree.
    let toc = if !nav_toc.is_empty() {
        nav_toc
    } else {
        chapters
            .iter()
            .map(|ch| EpubTocEntry {
                title: ch.display_heading().to_string(),
                chapter_index: ch.index,
                level: 0,
            })
            .collect()
    };

    Ok(EpubLoadResult {
        metadata,
        chapters,
        toc,
        footnotes,
        footnote_refs,
    })
}

#[cfg(test)]
#[path = "loading_tests.rs"]
mod tests;
