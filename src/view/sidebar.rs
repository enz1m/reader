//! The reading sidebar.
//!
//! The sidebar exposes outline, thumbnails, annotations, full-text search,
//! bookmarks, and footnotes through one keyboard-navigable tab strip.

use enzui_iced_core::action::Action;
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::list_row_surface::enz_list_row_surface;
use enzui_iced_widgets::scroll_view::enz_scroll_view;
use enzui_iced_widgets::section_header::enz_section_header;
use enzui_iced_widgets::sidebar_row::enz_sidebar_row;
use enzui_iced_widgets::status_page::{StatusPageVariant, enz_status_page};
use enzui_iced_widgets::tab_pill::{TabPillGeometry, TabPillItem, enz_tab_list};
use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_iced_widgets::virtual_list::enz_virtual_list;
use iced::widget::{column, container, row};
use iced::{Alignment, Element, Length, Padding};

use crate::message::Msg;
use crate::model::{Model, SidebarTab};

pub fn view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let tabs = tab_switcher(model, theme);

    let list = match model.sidebar_tab {
        SidebarTab::Thumbnails => thumbnails_list(model, theme),
        SidebarTab::Annotations => super::annotations_panel::view(model, theme),
        SidebarTab::Search => super::search_panel::view(model, theme),
        SidebarTab::Footnotes => footnotes_list(model, theme),
        SidebarTab::Bookmarks => bookmarks_list(model, theme),
        // `states::outline_state` resolves loading, nothing open and empty so each
        // branch is reachable from a test.
        SidebarTab::Contents => match super::states::outline_state(model) {
            Some(state) => container({
                let mut page = enz_status_page(state.title)
                    .icon(state.icon)
                    .description(state.description)
                    .variant(StatusPageVariant::Inline)
                    .compact(true);
                // The "nothing open" description offers the open action in place.
                if super::states::family(state) == super::states::StateFamily::NothingOpen {
                    page = page.primary_action(
                        Action::new("reader.open", Msg::OpenDialog).with_label("Open File"),
                    );
                }
                page.into_element()
            })
            .padding(spacing.lg())
            .width(Length::Fill)
            .into(),
            None => toc_list(model, theme),
        },
    };

    let body: Element<'a, Msg> = match model.sidebar_tab {
        // These panels own their scroll or virtualized viewport.
        SidebarTab::Annotations | SidebarTab::Search | SidebarTab::Thumbnails => list,
        SidebarTab::Contents | SidebarTab::Bookmarks | SidebarTab::Footnotes => {
            enz_scroll_view(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    };

    enzui_iced_widgets::sidebar_content::enz_sidebar_content()
        .spacing_scale(*spacing)
        .padding_left_px(0.0)
        .padding_right_px(0.0)
        .padding_top_px(0.0)
        .padding_bottom_px(0.0)
        .spacing_px(0.0)
        .push(tabs)
        .push(body)
        .into_element()
}

/// One sidebar tab: the icon it shows and the name it announces.
///
/// Icon-only, with the full name on the accessible label. Six text pills do
/// not fit a 250px sidebar; the measured icon geometry does.
pub struct SidebarTabSpec {
    /// The Lucide icon drawn in the pill.
    pub icon: &'static str,
    /// The accessible name and tooltip: the full word, never the icon name.
    pub name: &'static str,
}

/// Sidebar tabs in display order.
pub const TABS: &[SidebarTabSpec] = &[
    SidebarTabSpec {
        icon: "grid-2x2",
        name: "Pages",
    },
    SidebarTabSpec {
        icon: "list",
        name: "Contents",
    },
    SidebarTabSpec {
        icon: "sticky-note",
        name: "Annotations",
    },
    SidebarTabSpec {
        icon: "search",
        name: "Find in document",
    },
    SidebarTabSpec {
        icon: "bookmark",
        name: "Bookmarks",
    },
    SidebarTabSpec {
        icon: "hash",
        name: "Footnotes",
    },
];

// The routing identity of each strip slot; the selected and `on_select` indices map
// through this one table.
const TAB_IDS: &[SidebarTab] = &[
    SidebarTab::Thumbnails,
    SidebarTab::Contents,
    SidebarTab::Annotations,
    SidebarTab::Search,
    SidebarTab::Bookmarks,
    SidebarTab::Footnotes,
];

/// The message the strip emits when the tab at `index` is chosen.
///
/// The Find tab routes through `Msg::OpenSearch` so the panel is ready; every other tab
/// is a plain `SelectSidebarTab`. An out-of-range index falls back to Contents.
pub fn tab_message(index: usize) -> Msg {
    match TAB_IDS.get(index) {
        Some(SidebarTab::Search) => Msg::OpenSearch,
        Some(tab) => Msg::SelectSidebarTab(*tab),
        None => Msg::SelectSidebarTab(SidebarTab::Contents),
    }
}

/// The index of the currently-selected tab in `TAB_IDS`, for the strip's
/// `selected` cursor.
pub fn selected_tab_index(tab: SidebarTab) -> usize {
    TAB_IDS.iter().position(|t| *t == tab).unwrap_or(1)
}

// Icon-only presentation fits the sidebar while `aria_label` preserves full
// names and the tab list retains one roving keyboard stop.
fn tab_switcher<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let items: Vec<TabPillItem> = TABS
        .iter()
        .map(|spec| {
            // An empty visible label gives an icon-only pill; `aria_label` is both the
            // announced name and the routable identity key.
            TabPillItem::new("").icon(spec.icon).aria_label(spec.name)
        })
        .collect();

    // The compact inset keeps all six icon pills inside the fixed sidebar.
    let geometry = TabPillGeometry {
        pill_padding_x: spacing.xs(),
        pill_radius: crate::view::shape_radius(theme, theme.shape.button),
        label_size: enzui_iced_theme::text_scale::ui_px(13.0),
        strip_gap: spacing.xs(),
        ..TabPillGeometry::DEFAULT
    };

    let strip = enz_tab_list(items, selected_tab_index(model.sidebar_tab), tab_message)
        .geometry(geometry)
        // Names the TabList group for landmark navigation.
        .aria_label("Sidebar sections")
        // Stable id prefix so the derived tab ids never collide with another strip.
        .tab_id_prefix("reader-sidebar-tab")
        .into_element();

    container(strip)
        .padding(Padding {
            top: spacing.space0,
            right: spacing.lg(),
            bottom: spacing.xs(),
            left: spacing.lg(),
        })
        .into()
}

pub(crate) fn pdf_outline_message(entry: &crate::plumbing::pdf_navigation::PdfOutlineEntry) -> Msg {
    Msg::GoToPage(entry.page)
}

fn toc_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    if model.format == crate::model::Format::Pdf {
        let mut column = column![].spacing(1).width(Length::Fill);
        for (index, entry) in model.pdf_outline.iter().enumerate() {
            column = column.push(
                enz_sidebar_row(entry.title.clone(), pdf_outline_message(entry))
                    .selected(entry.page == model.current_page)
                    .leading_indent(spacing.md() + entry.level as f32 * 14.0)
                    .sizing(theme.sizing)
                    .id(iced::advanced::widget::Id::from(format!(
                        "reader-pdf-outline-{index}"
                    )))
                    .into_element(),
            );
        }
        if model.pdf_navigation_truncated {
            column = column.push(
                container(
                    enz_text("Outline truncated at the safe entry limit.")
                        .size(TextSize::Caption)
                        .tone(TextTone::Muted),
                )
                .padding(spacing.md()),
            );
        }
        return column.into();
    }
    let Some(epub) = model.epub.as_ref() else {
        return container(iced::widget::Space::new().width(Length::Fill)).into();
    };

    // A 1px hairline gap, below the spacing ladder's smallest rung (2px): it keeps
    // abutting TOC rows from merging without opening a readable gap.
    let mut col = column![].spacing(1).width(Length::Fill);
    for entry in &epub.load.toc {
        let selected = entry.chapter_index == model.epub_current_chapter;
        // Base gutter plus one step per TOC depth.
        let indent = spacing.md() + entry.level as f32 * 14.0;
        col = col.push(
            enz_sidebar_row(entry.title.clone(), Msg::GoToChapter(entry.chapter_index))
                .selected(selected)
                .leading_indent(indent)
                .sizing(theme.sizing)
                // Titles can repeat, so key each row on its unique chapter index.
                .id(iced::advanced::widget::Id::from(format!(
                    "reader-toc-{}",
                    entry.chapter_index
                )))
                .into_element(),
        );
    }
    col.into()
}

// The bookmarks panel.
//
// Its empty description is `states::bookmarks_hint()`, which reads the chord from the
// dispatcher's own table.
#[derive(Clone)]
struct ThumbnailPageProvider {
    count: usize,
}

impl enzui_iced_core::virtual_sequence::ItemProvider<usize> for ThumbnailPageProvider {
    fn len(&self) -> u64 {
        self.count as u64
    }

    fn item(&self, index: u64) -> Option<usize> {
        usize::try_from(index)
            .ok()
            .filter(|index| *index < self.count)
    }
}

impl enzui_iced_core::virtual_sequence::KeyedProvider<usize> for ThumbnailPageProvider {
    fn key(&self, index: u64) -> Option<enzui_iced_core::virtual_sequence::SequenceKey> {
        usize::try_from(index)
            .ok()
            .filter(|index| *index < self.count)
            .map(|page| {
                enzui_iced_core::virtual_sequence::SequenceKey::new(format!("pdf-thumbnail-{page}"))
            })
    }
}

fn thumbnails_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    if model.format != crate::model::Format::Pdf {
        return container(
            enz_status_page("No PDF thumbnails")
                .icon("grid-2x2")
                .description("Page thumbnails are available for PDF documents.")
                .variant(StatusPageVariant::Inline)
                .compact(true)
                .into_element(),
        )
        .padding(theme.spacing.lg())
        .width(Length::Fill)
        .into();
    }
    if let Some(state) = super::states::thumbnails_state(model) {
        return container(
            enz_status_page(state.title)
                .icon(state.icon)
                .description(state.description)
                .variant(StatusPageVariant::Inline)
                .compact(true)
                .into_element(),
        )
        .padding(theme.spacing.lg())
        .width(Length::Fill)
        .into();
    }

    let provider = std::rc::Rc::new(ThumbnailPageProvider {
        count: model.page_count,
    });
    let spacing = &theme.spacing;
    enz_virtual_list(
        provider,
        crate::raster::THUMBNAIL_ROW_HEIGHT,
        move |_index, page| {
            let page = *page;
            let preview: Element<'a, Msg> = if let Some(thumbnail) =
                model.thumbnail_rasters.get(page)
            {
                // The page as it will be saved and as it reads: its own
                // unsaved rotation composed with the view's, as in the
                // page arranger.
                let rotation = super::reading::effective_page_rotation(model, page);
                let quarter = !rotation.degrees().is_multiple_of(180);
                let (w, h) = (thumbnail.width as f32, thumbnail.height as f32);
                let (shown_w, shown_h) = if quarter { (h, w) } else { (w, h) };
                let scale = (88.0 / shown_w).min(108.0 / shown_h).min(1.0);
                iced::widget::image(thumbnail.handle.clone())
                    .width(Length::Fixed(shown_w * scale))
                    .height(Length::Fixed(shown_h * scale))
                    .rotation(iced::Rotation::Solid(iced::Radians(rotation.radians())))
                    .into()
            } else {
                use enzui_iced_core::async_image::{FailureReason, Generation, ImageState};
                use enzui_iced_widgets::async_image::{EnzAsyncImageFit, enz_async_image_view};
                let state: ImageState<iced::widget::image::Handle> =
                    if model.thumbnail_failed.contains(&page) {
                        ImageState::Failed(FailureReason::Service("thumbnail unavailable".into()))
                    } else {
                        ImageState::Loading(Generation::FIRST)
                    };
                enz_async_image_view(&state)
                    .fit(EnzAsyncImageFit::Contain)
                    .alt_text(format!("Page {} thumbnail", page + 1))
                    .width(Length::Fixed(88.0))
                    .height(Length::Fixed(108.0))
                    .into_element()
            };
            let body = row![
                preview,
                enz_text(format!("Page {}", page + 1)).size(TextSize::Body)
            ]
            .spacing(spacing.md())
            .align_y(Alignment::Center);
            // The page sits flush with the row's leading edge, so the
            // selection fill starts where the page does and shows only to
            // its right. Right-click offers the page's own menu.
            enz_list_row_surface(body, Msg::GoToPage(page))
                .row_height(crate::raster::THUMBNAIL_ROW_HEIGHT - spacing.sm())
                .content_padding(0.0)
                .outer_gutter(spacing.lg())
                .on_context(move |at| {
                    Msg::OpenContextMenu(crate::context_menu::ContextTarget::Page(page), at)
                })
                .context_label(format!("Page {} actions", page + 1))
                .selected(page == model.current_page)
                .label(format!("Page {} thumbnail", page + 1))
                .id(iced::advanced::widget::Id::from(format!(
                    "reader-pdf-thumbnail-{page}"
                )))
                .width(Length::Fill)
                .into_element()
        },
    )
    .overscan(2)
    .viewport_height(model.thumbnail_viewport_height.max(240.0))
    .scroll_offset(model.thumbnail_scroll_offset)
    .on_scroll(move |viewport| Msg::PdfThumbnailsScrolled {
        generation: model.thumbnail_generation,
        offset: viewport.absolute_offset().y,
        viewport_height: viewport.bounds().height,
    })
    .id(iced::advanced::widget::Id::new("reader-pdf-thumbnails"))
    .width(Length::Fill)
    .height(Length::Fill)
    .into_element()
}

fn bookmarks_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    if let Some((title, description)) = super::states::bookmarks_state(model) {
        let nothing_open = description.as_str() == super::states::OPEN_FOR_BOOKMARKS;
        let mut page = enz_status_page(title)
            .icon("bookmark")
            .description(description)
            .variant(StatusPageVariant::Inline)
            .compact(true);
        // The "nothing open" description offers the open action in place.
        if nothing_open {
            page = page.primary_action(
                Action::new("reader.open", Msg::OpenDialog).with_label("Open File"),
            );
        }
        return container(page.into_element())
            .padding(spacing.lg())
            .width(Length::Fill)
            .into();
    }

    let document_key = model
        .file_path
        .as_ref()
        .map(|path| crate::plumbing::persistence::canonical_key(&path.to_string_lossy()))
        .unwrap_or_default();
    let mut col = column![].spacing(1).width(Length::Fill);
    for mark in &model.annotations.bookmarks {
        // Each row carries its own `Remove bookmark` control in the `trailing` slot, so
        // the button sits inside the row's hover pill while keeping its own
        // `Role::Button` node beside the ListItem.
        let remove = enz_toolbar_icon("x", Msg::RemoveBookmark(mark.page))
            .aria_label(super::states::REMOVE_BOOKMARK)
            .tooltip(super::states::REMOVE_BOOKMARK)
            .into_element();
        col = col.push(
            enz_sidebar_row(
                mark.label.clone(),
                Msg::GoToBookmark {
                    format: model.format,
                    document_key: document_key.clone(),
                    position: mark.page,
                },
            )
            .selected(mark.page == model.current_position())
            .leading_indent(spacing.md())
            .sizing(theme.sizing)
            .id(iced::advanced::widget::Id::from(format!(
                "reader-bookmark-{}",
                mark.page
            )))
            .trailing(remove)
            .trailing_visibility(
                enzui_iced_widgets::list_row_surface::TrailingVisibility::OnHoverOrFocus,
            ),
        );
    }
    col.into()
}

fn footnotes_list<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    if let Some(state) = super::states::footnotes_state(model) {
        let mut page = enz_status_page(state.title)
            .icon(state.icon)
            .description(state.description)
            .variant(StatusPageVariant::Inline)
            .compact(true);
        if super::states::family(state) == super::states::StateFamily::NothingOpen {
            page = page.primary_action(
                Action::new("reader.open", Msg::OpenDialog).with_label("Open File"),
            );
        }
        return container(page.into_element())
            .padding(spacing.lg())
            .width(Length::Fill)
            .into();
    }

    let Some(epub) = model.epub.as_ref() else {
        return container(iced::widget::Space::new().width(Length::Fill)).into();
    };
    // Missing or blank titles fall back to `states::chapter_label`, the composer's
    // formatter.
    let titles: Vec<String> = epub
        .load
        .chapters
        .iter()
        .map(|c| c.display_heading().to_string())
        .collect();
    let rows = crate::plumbing::footnotes::group_for_sidebar(&epub.load.footnotes, &titles);

    let mut col = column![].spacing(1).width(Length::Fill);
    for (i, entry) in rows.iter().enumerate() {
        if entry.is_header {
            col = col.push(
                container(enz_section_header(entry.group_label.clone()).into_element()).padding(
                    Padding {
                        top: spacing.sm(),
                        right: spacing.lg(),
                        bottom: spacing.xs(),
                        left: spacing.lg(),
                    },
                ),
            );
            continue;
        }
        col = col.push(
            enz_sidebar_row(
                format!("{}  {}", entry.label, entry.text),
                Msg::GoToChapter(entry.chapter),
            )
            .leading_indent(spacing.md() + 14.0)
            .sizing(theme.sizing)
            // Labels repeat across chapters, so the id is keyed on the row index.
            .id(iced::advanced::widget::Id::from(format!(
                "reader-footnote-{i}"
            )))
            .into_element(),
        );
    }
    col.into()
}

pub(crate) fn title(tab: SidebarTab) -> &'static str {
    TABS[selected_tab_index(tab)].name
}
