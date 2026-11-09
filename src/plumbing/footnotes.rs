//! Extract footnote and endnote references from EPUB HTML.
//!
//! Recognises EPUB semantic attributes, ARIA roles, footnote classes and short
//! same-page anchors. Cross-chapter targets resolve against parsed spine resources.

/// A UI-ready row: either a chapter header (when `is_header == true`,
/// `label`/`text`/`id` unused, `group_label` filled with the chapter title)
/// or a footnote entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub id: String,
    pub chapter: usize,
    pub label: String,
    pub text: String,
    pub is_header: bool,
    pub group_label: String,
}

/// Group a resolved-footnotes list by chapter, emitting one header row per
/// chapter that has notes. `chapter_titles` maps `chapter_index -> title`
/// (any missing index is rendered as "Chapter {n+1}").
///
/// Preserves the reading order `resolve_all` produces.
pub fn group_for_sidebar(footnotes: &[Footnote], chapter_titles: &[String]) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    let mut current: Option<usize> = None;
    for fn_note in footnotes {
        if current != Some(fn_note.chapter) {
            let title = chapter_titles
                .get(fn_note.chapter)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Chapter {}", fn_note.chapter + 1));
            rows.push(SidebarRow {
                id: String::new(),
                chapter: fn_note.chapter,
                label: String::new(),
                text: String::new(),
                is_header: true,
                group_label: title,
            });
            current = Some(fn_note.chapter);
        }
        rows.push(SidebarRow {
            id: fn_note.id.clone(),
            chapter: fn_note.chapter,
            label: fn_note.label.clone(),
            text: fn_note.text.clone(),
            is_header: false,
            group_label: String::new(),
        });
    }
    rows
}

/// A parsed footnote reference + its target text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footnote {
    /// Target id (`fn1` in `#fn1`), unique within its source chapter.
    pub id: String,
    /// The short label shown at the reference site (`1`, `[1]`, `*`, …).
    pub label: String,
    /// The plain-text body of the note (entities decoded, tags stripped,
    /// whitespace normalised).
    pub text: String,
    /// The chapter index in which the note body lives.
    pub chapter: usize,
    /// Href to another chapter's document (`Some("notes.xhtml")`) when the reference
    /// points off-page, used for cross-chapter resolution.
    pub source_href: Option<String>,
}

/// Reference site: the anchor inside a chapter that points at a footnote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FootnoteRef {
    /// The chapter that contains the reference anchor.
    pub chapter: usize,
    /// Target id — matches `Footnote::id` when the note is same-chapter.
    pub target_id: String,
    /// Optional external file portion of the href (e.g. `notes.xhtml`).
    pub target_href: Option<String>,
    /// Short label shown at the reference site.
    pub label: String,
}

/// Extract same-chapter footnotes from one chapter's HTML as `(refs, notes)`.
/// Cross-chapter refs are included with `target_href = Some(...)`; the caller resolves
/// their bodies later.
pub fn extract_from_html(html: &str, chapter: usize) -> (Vec<FootnoteRef>, Vec<Footnote>) {
    let refs = find_note_refs(html, chapter);
    let notes = find_note_bodies(html, chapter);
    (refs, notes)
}

/// Resolve all chapters' `(refs, notes)` pairs into one list of `Footnote`s.
///
/// The file portion of a ref's `target_href` is matched against `chapter_paths`
/// (case-insensitive, path components ignored). Notes are unique per `(chapter, id)`, and
/// a note without a matching ref is still included.
pub fn resolve_all(
    per_chapter: &[(Vec<FootnoteRef>, Vec<Footnote>)],
    chapter_paths: &[String],
) -> Vec<Footnote> {
    let mut out: Vec<Footnote> = Vec::new();
    // Collect all note bodies keyed by (chapter, id); `out` starts from them.
    for (_refs, notes) in per_chapter.iter() {
        for note in notes {
            out.push(note.clone());
        }
    }
    // Thread ref-site labels onto notes, since some EPUBs number notes only at the ref.
    for (refs, _notes) in per_chapter.iter() {
        for r in refs {
            // Same-chapter ref → find note with matching id in that chapter.
            if r.target_href.is_none() {
                if let Some(n) = out
                    .iter_mut()
                    .find(|n| n.chapter == r.chapter && n.id == r.target_id)
                    && n.label.is_empty()
                {
                    n.label = r.label.clone();
                }
                continue;
            }
            // Cross-chapter ref: find the target chapter by filename.
            let href = r.target_href.as_ref().unwrap();
            let target_file = basename(href).to_lowercase();
            let dest_chapter = chapter_paths
                .iter()
                .enumerate()
                .find(|(_, p)| basename(p).to_lowercase() == target_file)
                .map(|(i, _)| i);
            if let Some(dc) = dest_chapter
                && let Some(n) = out
                    .iter_mut()
                    .find(|n| n.chapter == dc && n.id == r.target_id)
            {
                if n.label.is_empty() {
                    n.label = r.label.clone();
                }
                // Marks the note as reached through a cross-chapter link.
                n.source_href = Some(href.clone());
            }
        }
    }
    // Deduplicate: keep the first occurrence of each (chapter, id).
    let mut seen = std::collections::HashSet::new();
    out.retain(|n| seen.insert((n.chapter, n.id.clone())));
    out
}

fn basename(href: &str) -> String {
    // Split at '#' to drop the fragment; then keep the last path component.
    let no_frag = href.split('#').next().unwrap_or("");
    let last = no_frag.rsplit('/').next().unwrap_or(no_frag);
    last.to_string()
}

// Find `<a href="#..">` anchors that look like footnote references.
fn find_note_refs(html: &str, chapter: usize) -> Vec<FootnoteRef> {
    // ASCII-only lowercasing preserves byte offsets used to slice `html`; a full
    // `to_lowercase()` can expand a char such as `İ`.
    let lower = html.to_ascii_lowercase();
    let mut refs = Vec::new();
    // Scan each "<a " opening to its ">" and "</a>", inspecting attributes.
    let mut i = 0;
    while let Some(start) = lower[i..].find("<a ") {
        let a_open = i + start;
        // Find end of opening tag.
        let tag_end = match lower[a_open..].find('>') {
            Some(o) => a_open + o,
            None => break,
        };
        // Attribute region: between "<a " and ">"
        let attrs = &html[a_open + 3..tag_end];
        // Anchor inner text: between ">" and "</a>"
        let inner_start = tag_end + 1;
        let inner_end = match lower[inner_start..].find("</a>") {
            Some(o) => inner_start + o,
            None => {
                i = tag_end + 1;
                continue;
            }
        };
        let inner = &html[inner_start..inner_end];

        let href = extract_attr(attrs, "href");
        let role = extract_attr(attrs, "role");
        let epub_type = extract_attr(attrs, "epub:type");
        let class = extract_attr(attrs, "class");

        // Classify: is this a footnote/endnote reference?
        let is_note_ref = matches_semantic(&epub_type, &["noteref"])
            || matches_semantic(&role, &["doc-noteref", "doc-biblioref"])
            || matches_class(&class, &["footnote", "endnote", "noteref", "footnote-ref"])
            || is_bare_short_label_ref(&href, inner);

        if is_note_ref && href.starts_with('#') {
            // Same-page ref.
            let target_id = href.trim_start_matches('#').to_string();
            if !target_id.is_empty() {
                refs.push(FootnoteRef {
                    chapter,
                    target_id,
                    target_href: None,
                    label: normalise_label(inner),
                });
            }
        } else if is_note_ref && href.contains('#') {
            // Cross-file ref.
            let (file, frag) = href.split_once('#').unwrap();
            if !frag.is_empty() && !file.is_empty() {
                refs.push(FootnoteRef {
                    chapter,
                    target_id: frag.to_string(),
                    target_href: Some(file.to_string()),
                    label: normalise_label(inner),
                });
            }
        }

        i = inner_end + 4; // past </a>
    }
    refs
}

// Find note body elements (`<aside>`, `<div>`, `<p>`, `<section>` etc.
// with the right epub:type / role / class).  Returns their plain text
// keyed by their id attribute.
fn find_note_bodies(html: &str, chapter: usize) -> Vec<Footnote> {
    // Candidate elements marked as a footnote or endnote by epub:type, role or class,
    // with an id attribute.
    let candidates = ["aside", "div", "section", "p", "li"];
    // ASCII-only lowercasing preserves byte offsets (see `find_note_refs`).
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    for tag in candidates {
        let open_marker = format!("<{tag}");
        let close_marker = format!("</{tag}>");
        let mut i = 0;
        while let Some(start) = lower[i..].find(&open_marker) {
            let el_open = i + start;
            // Ensure the character after tag name is space or '>' (not a
            // longer tag like <asides>).
            let after = el_open + open_marker.len();
            if after >= html.len() {
                break;
            }
            let ch_after = html.as_bytes()[after] as char;
            if ch_after != ' '
                && ch_after != '>'
                && ch_after != '\n'
                && ch_after != '\r'
                && ch_after != '\t'
            {
                i = after;
                continue;
            }
            // Find end of opening tag.
            let tag_end = match lower[el_open..].find('>') {
                Some(o) => el_open + o,
                None => break,
            };
            let attrs = &html[el_open + open_marker.len()..tag_end];

            let id = extract_attr(attrs, "id");
            let epub_type = extract_attr(attrs, "epub:type");
            let role = extract_attr(attrs, "role");
            let class = extract_attr(attrs, "class");

            let is_note =
                matches_semantic(&epub_type, &["footnote", "endnote", "note", "rearnote"])
                    || matches_semantic(&role, &["doc-footnote", "doc-endnote", "doc-note"])
                    || matches_class(&class, &["footnote", "endnote", "note"]);

            if !is_note || id.is_empty() {
                i = tag_end + 1;
                continue;
            }

            // Nested same-tag elements are not supported; note bodies are leaf elements.
            let close_at = match lower[tag_end..].find(&close_marker) {
                Some(o) => tag_end + o,
                None => {
                    i = tag_end + 1;
                    continue;
                }
            };
            let inner_html = &html[tag_end + 1..close_at];
            let text = strip_tags_and_normalise(inner_html);
            if !text.is_empty() {
                out.push(Footnote {
                    id: id.clone(),
                    label: String::new(), // filled in later from the ref
                    text,
                    chapter,
                    source_href: None,
                });
            }
            i = close_at + close_marker.len();
        }
    }
    out
}

// Return the attribute value or "" if the attribute is missing.
// Case-insensitive attribute name match.  Attribute values may be
// quoted with `"` or `'`.
fn extract_attr(attrs: &str, name: &str) -> String {
    // ASCII-only lowercasing: offsets below index the original `attrs`, so the lowered
    // copy must preserve byte positions.
    let attrs_lower = attrs.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    // A match must follow whitespace, '<' or the start, so `epub` skips `xmlns:epub`.
    let mut search_from = 0usize;
    while let Some(o) = attrs_lower[search_from..].find(&format!("{name_lower}=")) {
        let idx = search_from + o;
        let prev = if idx == 0 {
            ' '
        } else {
            attrs.as_bytes()[idx - 1] as char
        };
        if !prev.is_whitespace() && prev != '\n' && prev != '\r' && idx != 0 {
            search_from = idx + name_lower.len();
            continue;
        }
        let val_start = idx + name_lower.len() + 1; // skip "="
        if val_start >= attrs.len() {
            return String::new();
        }
        let quote = attrs.as_bytes()[val_start] as char;
        let (start, end_char) = if quote == '"' || quote == '\'' {
            (val_start + 1, quote)
        } else {
            (val_start, ' ')
        };
        // Find the closing quote or whitespace.
        let mut end = start;
        while end < attrs.len() {
            let c = attrs.as_bytes()[end] as char;
            if c == end_char || (end_char == ' ' && c.is_whitespace()) {
                break;
            }
            end += 1;
        }
        return attrs[start..end].to_string();
    }
    String::new()
}

// Case-insensitive check whether any whitespace-separated token in `value`
// matches any target.
fn matches_semantic(value: &str, targets: &[&str]) -> bool {
    if value.is_empty() {
        return false;
    }
    let v = value.to_lowercase();
    v.split_whitespace().any(|t| targets.contains(&t))
}

fn matches_class(value: &str, targets: &[&str]) -> bool {
    matches_semantic(value, targets)
}

// Bare fallback: an anchor whose visible text is a short numeric or
// symbol label (e.g. `1`, `[1]`, `¹`, `*`, `†`, `‡`, `a`).
fn is_bare_short_label_ref(href: &str, inner_html: &str) -> bool {
    if !href.contains('#') {
        return false;
    }
    let inner = strip_tags_and_normalise(inner_html);
    let inner = inner.trim();
    if inner.is_empty() || inner.chars().count() > 4 {
        return false;
    }
    // Accept purely numeric labels, bracketed numbers, or single symbols
    // typical of footnote markers.
    let numeric_or_bracketed = inner
        .trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')')
        .chars()
        .all(|c| c.is_ascii_digit() || c == ',');
    let is_symbol = matches!(inner, "*" | "†" | "‡" | "§" | "¶" | "#");
    let is_superscript = inner
        .chars()
        .all(|c| matches!(c, '¹' | '²' | '³' | '⁰' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹'));
    let is_short_alpha =
        inner.chars().count() == 1 && inner.chars().next().unwrap().is_alphabetic();
    numeric_or_bracketed || is_symbol || is_superscript || is_short_alpha
}

// Trim, decode entities, and drop tags from a small HTML snippet — used
// on inner-anchor text and on footnote body HTML.  Keeps whitespace
// normalised to single spaces.
fn strip_tags_and_normalise(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_ws = true;
    let bytes: Vec<char> = html.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_tag {
            if c == '>' {
                in_tag = false;
            }
            i += 1;
            continue;
        }
        if c == '<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if c == '&' {
            // Entity decode: same set as epub_support.
            let rest_end = std::cmp::min(i + 10, bytes.len());
            let rest: String = bytes[i..rest_end].iter().collect();
            let (ch, consumed) = if rest.starts_with("&amp;") {
                ('&', 5)
            } else if rest.starts_with("&lt;") {
                ('<', 4)
            } else if rest.starts_with("&gt;") {
                ('>', 4)
            } else if rest.starts_with("&nbsp;") {
                (' ', 6)
            } else if rest.starts_with("&quot;") {
                ('"', 6)
            } else if rest.starts_with("&#") {
                // Numeric entity.
                if let Some(semi) = rest.find(';') {
                    let num_str = &rest[2..semi];
                    let code = if num_str.starts_with('x') || num_str.starts_with('X') {
                        u32::from_str_radix(&num_str[1..], 16).ok()
                    } else {
                        num_str.parse::<u32>().ok()
                    };
                    if let Some(code) = code.and_then(char::from_u32) {
                        (code, semi + 1)
                    } else {
                        ('&', 1)
                    }
                } else {
                    ('&', 1)
                }
            } else {
                ('&', 1)
            };
            if ch.is_whitespace() {
                if !last_ws {
                    out.push(' ');
                    last_ws = true;
                }
            } else {
                out.push(ch);
                last_ws = false;
            }
            i += consumed;
            continue;
        }
        if c.is_whitespace() {
            if !last_ws {
                out.push(' ');
                last_ws = true;
            }
        } else {
            out.push(c);
            last_ws = false;
        }
        i += 1;
    }
    out.trim().to_string()
}

// Turn an anchor's inner HTML into a short label suitable for display.
// Falls back to trimming to 4 characters.
fn normalise_label(inner_html: &str) -> String {
    let s = strip_tags_and_normalise(inner_html);
    // Drop enclosing brackets so "[1]" becomes "1".
    let trimmed = s.trim_matches(|c: char| c == '[' || c == ']');
    if trimmed.chars().count() > 4 {
        trimmed.chars().take(4).collect()
    } else {
        trimmed.to_string()
    }
}
