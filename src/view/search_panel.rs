//! Chapter and page-text search results.
//!
//! CBZ documents have no searchable text layer.

use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};

use enzui_iced_core::live_region::Announcement;
use enzui_iced_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey};
use enzui_iced_widgets::list_row_surface::enz_list_row_surface;
use enzui_iced_widgets::search_field::enz_search_field;
use enzui_iced_widgets::semantics_container::SemanticsContainer;
use enzui_iced_widgets::virtual_list::enz_virtual_list;
use iced::widget::{column, container};
use iced::{Element, Length, Padding};

use crate::message::Msg;
use crate::model::{Format, Model, Screen};

pub const FIND_LABEL: &str = "Find in document";

/// The query field's id: opening search focuses it.
pub fn find_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader.search.query")
}
/// Match navigation uses Enter, F3 and Shift+F3.
pub const NEXT_MATCH_LABEL: &str = "Next match";
pub const PREV_MATCH_LABEL: &str = "Previous match";
pub const NO_MATCHES: &str = "No matches";
pub const OPEN_PROMPT: &str = "Open a document to search.";
pub const CBZ_UNSUPPORTED: &str =
    "Find in document needs selectable text; this comic archive has none.";
pub const PDF_SEARCH_LOADING: &str = "Extracting searchable PDF text…";
pub const SEARCHING: &str = "Searching…";
pub const PDF_SEARCH_UNAVAILABLE: &str = "Searchable PDF text could not be loaded.";
pub const PDF_SEARCH_TRUNCATED: &str = "Search results are limited by the bounded PDF index.";
pub const EPUB_SEARCH_TRUNCATED: &str =
    "Showing the first matches; refine the query to narrow results.";

/// Search availability message; no-match queries keep their editable field and status rather
/// than replacing the panel.
pub fn body_state(model: &Model) -> Option<&'static str> {
    if model.screen != Screen::Reading {
        return Some(OPEN_PROMPT);
    }
    if model.format == Format::Cbz {
        return Some(CBZ_UNSUPPORTED);
    }
    if model.format == Format::Pdf && model.pdf_navigation_error.is_some() {
        return Some(PDF_SEARCH_UNAVAILABLE);
    }
    None
}

/// The `"3 of 12 matches"` counter for the current search, or an empty string when
/// there are no hits.
pub fn match_counter(model: &Model) -> String {
    let n = model.search_hit_count();
    if n == 0 {
        return String::new();
    }
    format!("{} of {n} matches", model.search_active + 1)
}

/// The one-line label for a single result row: `"CHAPTER I — …match preview…"`.
///
/// Names the reader-visible chapter heading, not `<title>` metadata.
pub fn result_label(model: &Model, hit: &crate::plumbing::epub_support::EpubHit) -> String {
    let chapter = model
        .epub
        .as_ref()
        .and_then(|e| e.load.chapters.iter().find(|c| c.index == hit.chapter))
        .map(|c| c.display_heading().to_string())
        .unwrap_or_else(|| format!("Chapter {}", hit.chapter + 1));
    format!("{chapter} \u{2014} {}", hit.preview.trim())
}

pub fn view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    if let Some(msg) = body_state(model) {
        return container(enz_text(msg).size(TextSize::Body).tone(TextTone::Muted))
            .padding(spacing.lg())
            .width(Length::Fill)
            .into();
    }

    let mut body = column![]
        .spacing(spacing.sm())
        .width(Length::Fill)
        .padding(Padding {
            top: spacing.sm(),
            right: spacing.lg(),
            bottom: spacing.sm(),
            left: spacing.lg(),
        });

    // Enter steps to the next match, as in every find bar; `NextMatch` wraps.
    body = body.push(
        enz_search_field(&model.search_query, Msg::SearchQueryChanged)
            .placeholder(FIND_LABEL)
            .shortcut("Ctrl+F")
            .on_clear(Msg::SearchQueryChanged(String::new()))
            .responsive(false)
            .focus_on_mount(true)
            .on_submit(Msg::NextMatch)
            .id(find_field_id())
            .into_element(),
    );

    // One status line under the field, always there while there is a query,
    // so nothing below it moves as the search runs.
    let status = if model.format == Format::Pdf && model.pdf_navigation_loading {
        PDF_SEARCH_LOADING.to_owned()
    } else if model.search_pending
        || model.search_in_flight_generation == Some(model.search_generation)
    {
        SEARCHING.to_owned()
    } else if model.search_hit_count() == 0 {
        NO_MATCHES.to_owned()
    } else {
        match_counter(model)
    };
    if !model.search_query.trim().is_empty() {
        // A polite live region, so a screen reader hears the match count
        // the sighted user reads as they type.
        let line = enz_text(status.clone())
            .size(TextSize::Caption)
            .tone(TextTone::Muted)
            .into_element();
        body = body.push(
            SemanticsContainer::leaf(line, Announcement::Status.region(status))
                .id(iced::advanced::widget::Id::new("reader-search-status")),
        );
    }
    let truncated = (model.format == Format::Pdf && model.pdf_search_truncated)
        .then_some(PDF_SEARCH_TRUNCATED)
        .or(
            (model.format == Format::Epub && model.epub_search_truncated)
                .then_some(EPUB_SEARCH_TRUNCATED),
        );
    if let Some(note) = truncated {
        body = body.push(enz_text(note).size(TextSize::Caption).tone(TextTone::Muted));
    }

    // The results: one row per match, its page or chapter over the passage,
    // realised only where the list is scrolled to. The previous results
    // stay until a new search lands, so typing does not blank the list.
    let count = model.search_hit_count();
    let list: Element<'a, Msg> = if count == 0 {
        iced::widget::Space::new().height(Length::Fill).into()
    } else {
        enz_virtual_list(
            std::rc::Rc::new(HitRows(count)),
            RESULT_ROW_HEIGHT,
            move |_, &index| result_row(model, theme, index),
        )
        .self_scrolling(model.search_scroll.clone())
        .id(iced::widget::Id::new("reader.search.results"))
        .width(Length::Fill)
        .height(Length::Fill)
        .into_element()
    };

    column![
        body,
        container(list)
            .padding(Padding {
                top: 0.0,
                right: spacing.sm(),
                bottom: spacing.sm(),
                left: spacing.sm(),
            })
            .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// A result row's height: the place line, two lines of passage, and room
// around them.
const RESULT_ROW_HEIGHT: f32 = 68.0;

// The search results, by position: the list realises only the rows in view.
struct HitRows(usize);

impl ItemProvider<usize> for HitRows {
    fn len(&self) -> u64 {
        self.0 as u64
    }

    fn item(&self, index: u64) -> Option<usize> {
        (index < self.0 as u64).then_some(index as usize)
    }
}

impl KeyedProvider<usize> for HitRows {
    fn key(&self, index: u64) -> Option<SequenceKey> {
        (index < self.0 as u64).then(|| SequenceKey::new(format!("hit-{index}")))
    }
}

// Where match `index` is and the passage around it.
fn result_parts(model: &Model, index: usize) -> Option<(String, String)> {
    match model.format {
        Format::Epub => model.search_hits.get(index).map(|hit| {
            let chapter = model
                .epub
                .as_ref()
                .and_then(|e| e.load.chapters.iter().find(|c| c.index == hit.chapter))
                .map(|c| c.display_heading().to_string())
                .unwrap_or_else(|| format!("Chapter {}", hit.chapter + 1));
            (chapter, hit.preview.trim().to_owned())
        }),
        Format::Pdf => model.pdf_search_hits.get(index).map(|hit| {
            (
                format!("Page {}", hit.page + 1),
                hit.preview.trim().to_owned(),
            )
        }),
        Format::Cbz => None,
    }
}

// Where `query` first appears in `passage`, ignoring case: the bytes a
// result row draws heavier, as Okular and Papers mark the match.
fn match_range(passage: &str, query: &str) -> Option<std::ops::Range<usize>> {
    let query: Vec<char> = query.trim().chars().collect();
    if query.is_empty() {
        return None;
    }
    passage.char_indices().find_map(|(start, _)| {
        let mut rest = passage[start..].char_indices();
        let mut end = start;
        for wanted in &query {
            let (offset, found) = rest.next()?;
            if !found.to_lowercase().eq(wanted.to_lowercase()) {
                return None;
            }
            end = start + offset + found.len_utf8();
        }
        Some(start..end)
    })
}

fn result_row<'a>(model: &'a Model, theme: &'a EnzTheme, index: usize) -> Element<'a, Msg> {
    let Some((place, passage)) = result_parts(model, index) else {
        return iced::widget::Space::new().into();
    };
    let label = format!("{place} \u{2014} {passage}");
    let semantics = enzui_iced_widgets::listbox_shape::listbox_option(
        label.clone(),
        index == model.search_active,
        index + 1,
        model.search_hit_count(),
    );
    let content = column![
        enz_text(place)
            .size(TextSize::Caption)
            .tone(TextTone::Muted)
            .single_line_elided(),
        {
            let matched = match_range(&passage, &model.search_query);
            let text = enz_text(passage).size(TextSize::Footnote).lines(2);
            match matched {
                Some(range) => text.emphasize(range),
                None => text,
            }
        },
    ]
    .spacing(2)
    .width(Length::Fill);
    container(
        enz_list_row_surface(content, Msg::SearchResultActivated(index))
            .selected(index == model.search_active)
            .row_height(RESULT_ROW_HEIGHT - theme.spacing.xs())
            .content_align_y(iced::Alignment::Center)
            .outer_gutter(0.0)
            .label(label)
            .semantics(semantics)
            .id_prefix(format!("reader-search-hit-{index}"))
            .into_element(),
    )
    .padding(Padding::new(0.0).bottom(theme.spacing.xs()))
    .into()
}
