//! Footnote stripping and HTML-to-text.

/// Strip footnote/endnote body elements from HTML so they don't appear in
/// the reading flow's plain text.  Removes `<aside>`, `<div>`, `<section>`,
/// `<p>`, and `<li>` elements whose `epub:type`, `role`, or `class` marks
/// them as footnote/endnote bodies.  Ref *anchors* (`<a>`) are kept so the
/// label text ("1", "†") remains in the prose for `locate_footnote_refs`.
pub fn strip_footnote_bodies(html: &str) -> String {
    // ASCII-only lowercasing so `lchars` (below) stays index-aligned with
    // `chars`. A full `to_lowercase()` can expand a char (`İ` → two chars),
    // desynchronising the two parallel arrays and stripping the wrong region.
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let tags: &[&str] = &["aside", "div", "section", "p", "li"];
    let note_markers: &[&str] = &[
        "footnote",
        "endnote",
        "note",
        "rearnote",
        "doc-footnote",
        "doc-endnote",
        "doc-note",
    ];
    // Work on char indices to stay UTF-8-safe.
    let chars: Vec<char> = html.chars().collect();
    let lchars: Vec<char> = lower.chars().collect();
    let len = chars.len();
    let mut ci = 0; // char index
    while ci < len {
        let mut matched = false;
        if lchars[ci] == '<' {
            for tag in tags {
                let open: Vec<char> = format!("<{}", tag).chars().collect();
                let olen = open.len();
                if ci + olen > len {
                    continue;
                }
                let slice: Vec<char> = lchars[ci..ci + olen].to_vec();
                if slice != open {
                    continue;
                }
                let after = ci + olen;
                if after >= len {
                    continue;
                }
                let ch = lchars[after];
                if ch != ' ' && ch != '>' && ch != '\n' && ch != '\r' && ch != '\t' {
                    continue;
                }
                // Find end of opening tag.
                let tag_end = match lchars[ci..].iter().position(|&c| c == '>') {
                    Some(o) => ci + o,
                    None => continue,
                };
                let attrs: String = lchars[after..tag_end].iter().collect();
                let is_note = note_markers.iter().any(|m| attrs.contains(m));
                if is_note {
                    let close: Vec<char> = format!("</{}>", tag).chars().collect();
                    let clen = close.len();
                    // Search for close tag from tag_end onward.
                    let mut si = tag_end;
                    let mut found_close = false;
                    while si + clen <= len {
                        if lchars[si..si + clen] == close[..] {
                            ci = si + clen;
                            matched = true;
                            found_close = true;
                            break;
                        }
                        si += 1;
                    }
                    if found_close {
                        break;
                    }
                }
            }
        }
        if !matched {
            out.push(chars[ci]);
            ci += 1;
        }
    }
    out
}

/// Strip HTML tags and return plain text content.
pub fn html_to_plain_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_style = false;
    let mut in_script = false;
    let mut last_was_space = true;

    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if !in_tag && chars[i] == '<' {
            in_tag = true;
            // Check for block elements that need newlines
            let rest_end = std::cmp::min(i + 20, len);
            let rest: String = chars[i..rest_end].iter().collect();
            let rest_lower = rest.to_lowercase();
            if (rest_lower.starts_with("<br")
                || rest_lower.starts_with("<p")
                || rest_lower.starts_with("<div")
                || rest_lower.starts_with("<h1")
                || rest_lower.starts_with("<h2")
                || rest_lower.starts_with("<h3")
                || rest_lower.starts_with("<h4")
                || rest_lower.starts_with("<h5")
                || rest_lower.starts_with("<h6")
                || rest_lower.starts_with("<li")
                || rest_lower.starts_with("<tr"))
                && !result.is_empty()
                && !result.ends_with('\n')
            {
                result.push('\n');
                last_was_space = true;
            }
            if rest_lower.starts_with("<style") {
                in_style = true;
            }
            if rest_lower.starts_with("<script") {
                in_script = true;
            }
            i += 1;
            continue;
        }

        if in_tag && chars[i] == '>' {
            in_tag = false;
            // Check for closing style/script
            let back_start = i.saturating_sub(8);
            let back: String = chars[back_start..=i].iter().collect();
            let back_lower = back.to_lowercase();
            if back_lower.contains("/style>") {
                in_style = false;
            }
            if back_lower.contains("/script>") {
                in_script = false;
            }
            i += 1;
            continue;
        }

        if !in_tag && !in_style && !in_script {
            let ch = chars[i];
            if ch == '&' {
                let rest_end = std::cmp::min(i + 10, len);
                let rest: String = chars[i..rest_end].iter().collect();
                if rest.starts_with("&amp;") {
                    result.push('&');
                    i += 5;
                } else if rest.starts_with("&lt;") {
                    result.push('<');
                    i += 4;
                } else if rest.starts_with("&gt;") {
                    result.push('>');
                    i += 4;
                } else if rest.starts_with("&nbsp;") {
                    result.push(' ');
                    i += 6;
                } else if rest.starts_with("&quot;") {
                    result.push('"');
                    i += 6;
                } else if rest.starts_with("&#") {
                    if let Some(semi) = rest.find(';') {
                        let num_str = &rest[2..semi];
                        if let Ok(code) = if num_str.starts_with('x') || num_str.starts_with('X') {
                            u32::from_str_radix(&num_str[1..], 16)
                        } else {
                            num_str.parse::<u32>()
                        } && let Some(c) = char::from_u32(code)
                        {
                            result.push(c);
                        }
                        i += semi + 1;
                    } else {
                        result.push('&');
                        i += 1;
                    }
                } else {
                    result.push('&');
                    i += 1;
                }
                last_was_space = false;
                continue;
            } else if ch.is_whitespace() {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            } else {
                result.push(ch);
                last_was_space = false;
            }
        }
        i += 1;
    }

    result.trim().to_string()
}
