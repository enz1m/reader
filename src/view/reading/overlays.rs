//! Selection overlays: actions card and footnote popover.

use super::{EnzTheme, Msg};
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::surface::{SurfaceTone, enz_surface};
use iced::Element;
use iced::Length;
use iced::widget::{column, container, row};

// One action on a selection: its icon, its name, and what it sends.
type SelectionAction = (&'static str, &'static str, Msg);

// The actions strip under a live selection: a small floating frosted pill of
// icon buttons, each named by its tooltip and its accessible label. Each is
// an ordinary framework toolbar icon, so Tab/Shift+Tab and assistive
// activation reach it without a pointer.
fn selection_strip<'a>(theme: &'a EnzTheme, actions: Vec<SelectionAction>) -> Element<'a, Msg> {
    use enzui_iced_widgets::glass::enz_glass;
    use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
    let spacing = &theme.spacing;
    let buttons = actions.into_iter().map(|(icon, label, message)| {
        enz_toolbar_icon(icon, message)
            .id(enzui_iced_core::identity::derive_id(
                "reader.selection",
                label,
            ))
            .size(32.0, 18.0)
            .aria_label(label)
            .tooltip(label)
            .into_element()
    });
    enz_glass(
        container(row(buttons).spacing(spacing.xs() / 2.0))
            .padding([spacing.xs() / 2.0, spacing.xs()]),
    )
    .style(enzui_iced_theme::MaterialStyle::FlatFrosted)
    .tint_opacity(0.18)
    .ink(enzui_iced_theme::GlassInk::Adaptive)
    .radius(crate::view::shape_radius(theme, theme.shape.popover))
    .elevation(3)
    .into()
}

pub(super) fn selection_actions<'a>(theme: &'a EnzTheme) -> Element<'a, Msg> {
    selection_strip(
        theme,
        vec![
            ("copy", "Copy", Msg::CopySelection),
            ("highlighter", "Highlight", Msg::HighlightSelection),
            ("message-square-plus", "Add note", Msg::AddNoteToSelection),
            ("search", "Find in book", Msg::FindSelection),
        ],
    )
}

pub(super) fn pdf_selection_actions<'a>(theme: &'a EnzTheme) -> Element<'a, Msg> {
    selection_strip(
        theme,
        vec![
            ("copy", "Copy", Msg::CopyPdfSelection),
            ("highlighter", "Highlight", Msg::AddPdfHighlight),
            ("message-square-plus", "Comment", Msg::AddPdfComment),
            ("search", "Find in document", Msg::FindSelection),
        ],
    )
}

pub(super) fn footnote_popover<'a>(
    note: &'a crate::plumbing::footnotes::Footnote,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let title = if note.label.trim().is_empty() {
        "Footnote".to_string()
    } else {
        format!("Footnote {}", note.label.trim())
    };
    let content = column![
        enz_text(title).size(TextSize::Title3),
        enz_text(note.text.clone())
            .size(TextSize::Body)
            .tone(TextTone::Default),
        enz_button("Close footnote", Msg::DismissFootnote)
            .kind(ButtonKind::Ghost)
            .size(ButtonSize::Sm)
            .into_element(),
    ]
    .spacing(theme.spacing.sm())
    .width(Length::Fixed(300.0));

    enz_surface(container(content).padding(theme.spacing.md()))
        .tone(SurfaceTone::Elevated)
        .elevation(3)
        .radius(crate::view::shape_radius(theme, theme.shape.popover))
        .into_element()
}
