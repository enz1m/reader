//! Library view and section headings.

use super::cards::count_label;
use super::list::book_list;
use super::metrics::{LIBRARY_MAX_WIDTH, library_track_count};
use super::*;
use iced::widget::{column, row};

pub fn view<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    content_width: f32,
    content_height: f32,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    if model.library_loading {
        return container(
            enz_status_page("Loading library…")
                .icon("loader")
                .description("Checking saved reading progress."),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    if model.library.is_empty() {
        return container(
            enz_status_page("Your library is empty")
                .icon("book-open")
                .description("Open a PDF, EPUB or CBZ from the titlebar to add it here."),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    // This is the persistent projection in Model, not a Vec rebuilt in
    // view. The virtual list borrows it and realises only the viewport window.
    let books = model.library_rows().rows();
    let filtering = !model.library_query.trim().is_empty();

    // Wide allocations become two bounded reading-order tracks and narrow ones one, so
    // each row keeps a readable measure.
    let bounded_width = LIBRARY_MAX_WIDTH.min(content_width).max(240.0);
    let tracks = library_track_count(bounded_width);

    let title = enz_text(LIBRARY_TITLE)
        .size(TextSize::Title3)
        .tone(TextTone::Default)
        .heading(HeadingLevel::Page);
    let subtitle = enz_text(count_label(books.len(), model.library.len()))
        .size(TextSize::Caption)
        .tone(TextTone::Muted);
    let sort_menu: Element<'a, Msg> = enz_menu_button(
        model.library_sort.label(),
        &model.library_sort_menu_model,
        &model.library_sort_menu,
        Msg::LibrarySortMenu,
    )
    .kind(MenuButtonKind::Quiet(ButtonSize::Sm))
    .alignment(enzui_iced_core::placement::Alignment::End)
    .aria_label("Sort by")
    .menu_id(iced::widget::Id::new("reader.library.sort"))
    .into();
    let header = row![
        column![title, subtitle].spacing(spacing.xs()),
        iced::widget::Space::new().width(Length::Fill),
        sort_menu,
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let filter = enz_search_field(&model.library_query, Msg::LibraryQueryChanged)
        .placeholder(LIBRARY_FILTER_LABEL)
        .focus_on_mount(true)
        .id("reader-library-filter");

    let mut content = column![header, filter]
        .spacing(spacing.md())
        .width(Length::Fill);

    // Recents are a section of the one library list: sorted by Recent the heading reads
    // "Continue reading", by Title/Author "All books", and a filter shows no heading.
    if books.is_empty() {
        content = content.push(
            container(
                enz_text(format!(
                    "No books match \u{201c}{}\u{201d}",
                    model.library_query.trim()
                ))
                .size(TextSize::Body)
                .tone(TextTone::Muted),
            )
            .padding(spacing.lg())
            .width(Length::Fill),
        );
    } else {
        if !filtering {
            let section = if model.library_sort == crate::model::LibrarySort::Recent {
                CONTINUE_READING_TITLE
            } else {
                ALL_BOOKS_TITLE
            };
            content = content.push(section_heading(section, theme));
        }
        content = content.push(book_list(
            model,
            theme,
            content_height,
            bounded_width,
            tracks,
        ));
    }

    // Only the shelf scrolls; the library header stays outside its viewport.
    let gutter = spacing.xl() + spacing.xs();
    let bounded = content
        .padding(iced::Padding {
            top: spacing.sm(),
            right: gutter,
            bottom: gutter,
            left: gutter,
        })
        .width(Length::Fixed(bounded_width))
        .height(Length::Fill);

    container(bounded)
        .center_x(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// A section heading within the library (`Continue reading`, `All books`).
// A `Section` rung under the screen's `Page` title, so the outline reads
// Page → Section and a screen-reader user can jump between the two groups.
pub(crate) fn section_heading<'a>(text: &'a str, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let _ = theme;
    enz_text(text)
        .size(TextSize::Title3)
        .tone(TextTone::Default)
        .heading(HeadingLevel::Section)
        .into()
}
