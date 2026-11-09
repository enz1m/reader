//! Page indicator text.

use super::*;

pub(crate) fn indicator_text(model: &Model) -> String {
    match model.format {
        Format::Epub => {
            // Count content chapters, not spine items, so front matter is excluded;
            // `epub_reading_position` is shared with the first-open heuristic.
            match model.epub_reading_position() {
                // Paginated mode reports the page derived from the anchored char offset,
                // so it is the page actually painted.
                Some((current, total)) if model.epub_is_paginated() => format!(
                    "Chapter {current} of {total} · Page {} of {}",
                    model.epub_page_index() + 1,
                    model.epub_page_count()
                ),
                Some((current, total)) => format!("Chapter {current} of {total}"),
                None => String::new(),
            }
        }
        Format::Pdf | Format::Cbz => {
            // "Page N of M" tracks the page under the viewport centre, which
            // `Msg::PagesScrolled` keeps in `current_page`.
            let total = model.total_page_count.max(model.page_count);
            match total {
                0 => String::new(),
                1 => "1 page".to_string(),
                n => format!(
                    "Page {} of {n}",
                    model.current_page.saturating_add(1).min(n)
                ),
            }
        }
    }
}
