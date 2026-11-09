//! Citations and reading-time clocks.

use super::*;

/// The text representations used by annotation clipboard actions.
///
/// Citation and Markdown payloads share [`render_annotation`] so their
/// annotation wording cannot drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CitationFidelity {
    /// One-line plain-text citation for the clipboard (`Copy with citation`).
    /// e.g. `"the quoted snippet" — Book Title, p. 12`.
    Plain,
    /// Markdown blockquote used by `Copy Markdown`, e.g.
    /// `> the quoted snippet [squiggly] (yellow)`.
    Markdown,
}

/// Render one annotation for a plain citation or Markdown clipboard payload.
///
/// `file_name` is used by plain citations; the complete Markdown payload owns
/// its document heading.
pub fn render_annotation(ann: &Annotation, file_name: &str, fidelity: CitationFidelity) -> String {
    // Style/colour tags shared by both fidelities so the two never drift.
    let style_tag = if ann.kind == "highlight" && !ann.style.is_empty() && ann.style != "highlight"
    {
        format!(" [{}]", ann.style)
    } else {
        String::new()
    };
    let color_tag = if !ann.color.is_empty() {
        format!(" ({})", ann.color)
    } else {
        String::new()
    };

    match fidelity {
        CitationFidelity::Plain => {
            // `"snippet" — Title, p. N` for highlights; `Note (p. N): body`
            // for notes. `page` is 0-indexed internally; humans count from 1.
            let page_1 = ann.page + 1;
            if ann.kind == "highlight" {
                let mut s = format!("\u{201c}{}\u{201d}", ann.text.trim());
                if !file_name.is_empty() {
                    s.push_str(&format!(" \u{2014} {}", file_name));
                }
                s.push_str(&format!(", p. {page_1}"));
                if !ann.note.is_empty() {
                    s.push_str(&format!(" ({})", ann.note.trim()));
                }
                s
            } else {
                format!("Note (p. {page_1}): {}", ann.text.trim())
            }
        }
        CitationFidelity::Markdown => {
            if ann.kind == "highlight" {
                let mut s = format!("> {}{}{}", ann.text, style_tag, color_tag);
                if !ann.note.is_empty() {
                    s.push_str(&format!("\n\n{}", ann.note));
                }
                s
            } else {
                format!("**Note:** {}", ann.text)
            }
        }
    }
}

/// Render one annotation as a one-line clipboard citation (`Copy with
/// citation`). Thin wrapper over [`render_annotation`] at [`CitationFidelity::Plain`].
pub fn annotation_citation(ann: &Annotation, file_name: &str) -> String {
    render_annotation(ann, file_name, CitationFidelity::Plain)
}

/// Serialize annotations and bookmarks as pretty-printed JSON.
pub fn annotations_to_json(file: &AnnotationsFile) -> String {
    serde_json::to_string_pretty(file).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

/// Render annotations and bookmarks as Markdown.
pub fn annotations_to_markdown(file: &AnnotationsFile, file_name: &str) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Annotations — {}\n\n", file_name));

    if !file.bookmarks.is_empty() {
        md.push_str("## Bookmarks\n\n");
        for b in &file.bookmarks {
            md.push_str(&format!("- **{}** (page {})\n", b.label, b.page + 1));
        }
        md.push('\n');
    }

    if !file.annotations.is_empty() {
        md.push_str("## Highlights & Notes\n\n");

        // Group by page
        let mut by_page: std::collections::BTreeMap<usize, Vec<&Annotation>> =
            std::collections::BTreeMap::new();
        for a in &file.annotations {
            by_page.entry(a.page).or_default().push(a);
        }

        for (page, anns) in &by_page {
            md.push_str(&format!("### Page {}\n\n", page + 1));
            for a in anns {
                md.push_str(&render_annotation(a, file_name, CitationFidelity::Markdown));
                md.push('\n');
                md.push('\n');
            }
        }
    }

    if file.bookmarks.is_empty() && file.annotations.is_empty() {
        md.push_str("_No annotations or bookmarks._\n");
    }

    md
}

/// ISO-8601 UTC timestamp using chrono.
pub fn current_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Wall-clock seconds since the Unix epoch, for measuring reading-session
/// elapsed time. Only clamped non-negative deltas are used, so a backwards clock jump
/// under-counts one session at most.
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Accumulate a reading session's elapsed time onto the previously-stored
/// total. `session_start` and `now` are Unix-epoch seconds; `prev_total` is the
/// seconds already banked for this book. A non-positive or absurd delta (clock
/// jumped backwards, or the session start was never set) contributes zero, so
/// the total only ever moves forward by real elapsed time.
pub fn accumulate_reading_time(prev_total: u64, session_start: u64, now: u64) -> u64 {
    // Guard: unset session start (0) or a backwards clock → no contribution.
    if session_start == 0 || now <= session_start {
        return prev_total;
    }
    let delta = now - session_start;
    // Reject an implausibly large single-session delta (> 24h) — almost
    // certainly a clock change, not real reading.
    const MAX_SESSION_SECS: u64 = 24 * 60 * 60;
    if delta > MAX_SESSION_SECS {
        return prev_total;
    }
    prev_total.saturating_add(delta)
}
