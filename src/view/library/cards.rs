//! Book cards, covers, and counts.

use super::metrics::{LIBRARY_COVER_HEIGHT, LIBRARY_COVER_WIDTH};
use super::text::{LIBRARY_MISSING_FILE_TEXT, book_author_text, book_stats_text, book_title_text};
use super::*;
use iced::widget::column;

pub(crate) fn cover_placeholder<'a>() -> Element<'a, Msg> {
    enz_surface(
        container(
            enz_icon("book")
                .size(crate::view::widgets::COVER_GLYPH_ICON)
                .tint_theme_tone(IconTone::Subtle)
                .into_element(),
        )
        .center_x(Length::Fixed(LIBRARY_COVER_WIDTH))
        .center_y(Length::Fixed(LIBRARY_COVER_HEIGHT)),
    )
    .tone(SurfaceTone::ImageCanvas)
    .radius(6.0)
    .width(Length::Fixed(LIBRARY_COVER_WIDTH))
    .height(Length::Fixed(LIBRARY_COVER_HEIGHT))
    .clip(true)
    .into_element()
}

// The row surface yields presses to its trailing remove button, so removal cannot also open
// the book.
pub(crate) fn book_item<'a>(
    book: &RecentBook,
    thumbnail: Option<&'a iced::widget::image::Handle>,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let cover: Element<'a, Msg> = match thumbnail {
        Some(handle) => enz_surface(
            iced::widget::image(handle.clone())
                .content_fit(iced::ContentFit::Cover)
                .width(Length::Fixed(LIBRARY_COVER_WIDTH))
                .height(Length::Fixed(LIBRARY_COVER_HEIGHT)),
        )
        .tone(SurfaceTone::Transparent)
        .radius(6.0)
        .width(Length::Fixed(LIBRARY_COVER_WIDTH))
        .height(Length::Fixed(LIBRARY_COVER_HEIGHT))
        .clip(true)
        .into_element(),
        None => cover_placeholder(),
    };

    let title = enz_text(book_title_text(book))
        .size(TextSize::Title3)
        .tone(TextTone::Default);
    let author = enz_text(book_author_text(book))
        .size(TextSize::Body)
        .tone(TextTone::Muted);

    // The same helper the row-height derivation shapes owns the progress caption.
    let progress = enz_text(book_stats_text(book))
        .size(TextSize::Caption)
        .tone(if book.file_missing {
            TextTone::Disabled
        } else {
            TextTone::Subtle
        });

    let bar = enz_progress_bar(book.progress_percentage.clamp(0.0, 1.0))
        .bar_height(6.0)
        .animated(false)
        .into_element();

    let mut info = column![title, author, progress, bar]
        .spacing((spacing.xs() + spacing.sm()) / 2.0)
        .width(Length::Fill);
    if book.file_missing {
        info = info.push(
            enz_text(LIBRARY_MISSING_FILE_TEXT.to_string())
                .size(TextSize::Caption)
                .tone(TextTone::Danger),
        );
    }

    // The row's own trailing control, shown on hover or focus like every list's remove
    // button, with a path-keyed id so a removal can hand focus to the successor's remove
    // button, and a full accessible name.
    let remove = enz_toolbar_icon("x", Msg::RemoveFromRecents(book.path.clone()))
        .aria_label(REMOVE_FROM_RECENTS)
        .id(remove_from_recents_id(&book.path))
        .into_element();

    let body = container(
        row![cover, info]
            .spacing(spacing.lg())
            .align_y(Alignment::Start)
            .width(Length::Fill),
    )
    .padding([spacing.md(), 0.0])
    .width(Length::Fill);

    // A missing file is not openable, so a click is inert rather than an error banner.
    let on_activate = if book.file_missing {
        Msg::Noop
    } else {
        Msg::OpenPath(book.path.clone().into())
    };
    let aria = book_row_label(&book.title, book.file_missing);

    // `on_context` publishes a routable `ShowContextMenu` node hugging the row and
    // reports the cursor for the point-anchored menu host.
    let path_for_ctx = book.path.clone();
    let ctx_title = book.title.clone();
    let content = enz_list_row_surface(body, on_activate)
        .trailing(remove)
        .trailing_visibility(TrailingVisibility::OnHoverOrFocus)
        .content_padding(spacing.md())
        .content_height(true)
        .width(Length::Fill)
        .label(aria)
        .id(crate::context_menu::library_card_id(&book.path))
        .on_context(move |at| {
            Msg::OpenContextMenu(ContextTarget::LibraryBook(path_for_ctx.clone()), at)
        })
        .context_id(crate::context_menu::context_target_id(
            &crate::context_menu::library_card_id(&book.path),
        ))
        .context_label(format!("{ctx_title} actions"))
        .into_element();
    if book.file_missing {
        content
    } else {
        enzui_iced_widgets::file_drag::enz_file_drag_source(
            content,
            [std::path::PathBuf::from(&book.path)],
        )
        .key(&book.path)
        .allows_captured_press(true)
        .into()
    }
}

pub(crate) fn count_label(shown: usize, total: usize) -> String {
    if shown == total {
        if total == 1 {
            "1 book".to_string()
        } else {
            format!("{total} books")
        }
    } else {
        format!("{shown} of {total} books")
    }
}
