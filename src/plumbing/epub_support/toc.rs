//! Title extraction and table of contents.

use super::*;

pub(crate) fn extract_html_title(html: &str) -> Option<String> {
    // ASCII-only lowercasing preserves byte offsets into `html`; a
    // length-changing full lowercase would mis-slice the title text.
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")?;
    let content_start = start + 7;
    let end = lower[content_start..].find("</title>")?;
    let title = html[content_start..content_start + end].trim().to_string();
    if title.is_empty() { None } else { Some(title) }
}

// Strip HTML tags from an inline fragment, decode common entities and collapse
// whitespace, turning `CHAPTER <span>I</span>` into its plain text.
fn strip_tags_collapse(fragment: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The first **non-empty** `<h1>`…`<h6>` heading in the document body, as plain
// text, or `None` when the body has no heading with visible text.
//
// Some exporters lead with an empty `<h3 class="chapter-number"></h3>` before the real
// heading, so this returns the first heading in document order that carries text.
pub(crate) fn extract_first_body_heading(html: &str) -> Option<String> {
    // Match tags on an ASCII-lowercased copy and slice the original at the same offsets.
    let lower = html.to_ascii_lowercase();
    let body_start = lower.find("<body").unwrap_or(0);
    let mut cursor = body_start;
    loop {
        // The earliest heading open tag at or after `cursor`, across h1..h6.
        let mut next: Option<(usize, u8)> = None;
        for level in 1..=6u8 {
            let needle = format!("<h{level}");
            let Some(rel) = lower[cursor..].find(&needle) else {
                continue;
            };
            let pos = cursor + rel;
            // Require a tag boundary after the digit, so `<h1x` does not match.
            let boundary = lower[pos + needle.len()..].chars().next();
            if !matches!(boundary, Some(c) if c == '>' || c.is_whitespace() || c == '/') {
                continue;
            }
            if next.map(|(p, _)| pos < p).unwrap_or(true) {
                next = Some((pos, level));
            }
        }
        let (pos, level) = next?;
        let open_end = {
            let rel = lower[pos..].find('>')?;
            pos + rel + 1
        };
        let close = format!("</h{level}>");
        let close_pos = {
            let rel = lower[open_end..].find(&close)?;
            open_end + rel
        };
        let text = strip_tags_collapse(&html[open_end..close_pos]);
        if !text.is_empty() {
            return Some(text);
        }
        cursor = close_pos + close.len();
    }
}

// Clean a `<title>` metadata string down to just the chapter name by dropping
// a trailing `" -- book name"` / `" – book name"` / `" — book name"` suffix.
//
// Exporters often write `<title>CHAPTER I -- Book</title>`. Used only when there is no
// navigation label or body heading, so the book name never becomes a heading.
pub(crate) fn clean_metadata_title(title: &str) -> String {
    let t = title.trim();
    for sep in [" -- ", " \u{2013} ", " \u{2014} "] {
        if let Some(idx) = t.find(sep) {
            return t[..idx].trim().to_string();
        }
    }
    t.to_string()
}

// Choose the reader-visible chapter heading.
//
// Precedence, highest first:
// 1. the navigation/TOC label for this chapter;
// 2. a visible body heading (`<h1>`…`<h6>`);
// 3. the cleaned `<title>` metadata, since raw `<title>` is metadata, not prose;
// 4. `"Chapter N"`.
pub(crate) fn choose_chapter_heading(
    toc_label: Option<&str>,
    body_heading: Option<&str>,
    meta_title: Option<&str>,
    idx: usize,
) -> String {
    if let Some(label) = toc_label.map(str::trim).filter(|s| !s.is_empty()) {
        return label.to_string();
    }
    if let Some(body) = body_heading.map(str::trim).filter(|s| !s.is_empty()) {
        return body.to_string();
    }
    if let Some(meta) = meta_title {
        let cleaned = clean_metadata_title(meta);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    format!("Chapter {}", idx + 1)
}

pub(crate) fn build_toc_from_nav(
    toc: &[epub::doc::NavPoint],
    spine_paths: &[std::path::PathBuf],
) -> Vec<EpubTocEntry> {
    fn normalise(path: &std::path::Path) -> std::path::PathBuf {
        let mut result = std::path::PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    result.pop();
                }
                _ => result.push(component),
            }
        }
        result
    }
    fn walk(
        points: &[epub::doc::NavPoint],
        spine_paths: &[std::path::PathBuf],
        entries: &mut Vec<EpubTocEntry>,
        level: usize,
    ) {
        for point in points {
            let content = point.content.to_string_lossy();
            let target = normalise(std::path::Path::new(
                content.split('#').next().unwrap_or_default(),
            ));
            if let Some(chapter_index) = spine_paths.iter().position(|path| *path == target) {
                entries.push(EpubTocEntry {
                    title: point.label.clone(),
                    chapter_index,
                    level,
                });
            }
            walk(&point.children, spine_paths, entries, level + 1);
        }
    }
    let paths: Vec<_> = spine_paths.iter().map(|path| normalise(path)).collect();
    let mut entries = Vec::new();
    walk(toc, &paths, &mut entries, 0);
    entries
}
