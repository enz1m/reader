//! Reading-theme chooser.

use super::*;
use iced::widget::column;

pub(crate) fn reading_theme_control<'a>(
    open: bool,
    current: ReadingTheme,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let trigger = super::super::widgets::icon_button_with_id(
        "palette",
        Msg::ToggleReadingThemeMenu,
        "Reading theme",
        Some(open),
        Some(super::super::widgets::reading_theme_trigger_id()),
    );
    let popup = open.then(|| reading_theme_menu(current, theme));
    enzui_iced_widgets::popover::enz_popover(trigger, popup, || Msg::CloseReadingThemeMenu)
        .alignment(PlacementAlignment::End)
        .padding(theme.spacing.xs())
        .scope(super::super::widgets::reading_theme_scope_id())
        .label(crate::actions::translated_label("Reading theme"))
        .into_element()
}

// Keyboard-accessible palette rows carry swatches and current selection; choosing one closes
// the popup.
pub(crate) fn reading_theme_menu<'a>(
    current: ReadingTheme,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let heading = container(
        enz_text(crate::actions::translated_label("Reading theme"))
            .size(TextSize::Caption)
            .tone(TextTone::Muted),
    )
    .padding(iced::Padding {
        top: spacing.xs(),
        right: spacing.sm(),
        bottom: spacing.xs(),
        left: spacing.sm(),
    });

    let mut col = column![heading]
        .spacing(spacing.space0)
        .width(Length::Fixed(228.0));
    for &t in ReadingTheme::ALL {
        col = col.push(reading_theme_row(t, t == current, theme));
    }

    col.into()
}

// One row of [`reading_theme_menu`]: `[swatch] Name .... [check]`, the whole
// row an `enz_list_row_surface` whose activation selects the theme.
pub(crate) fn reading_theme_row<'a>(
    t: ReadingTheme,
    selected: bool,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    let colors = crate::view::reading_colors(t, theme);

    // The theme's ink "Aa" over its page colour, as a framework tonal surface.
    let swatch = enz_surface(
        container(enz_text("Aa").size(TextSize::Caption).color(rgb(colors.fg)))
            .center_x(Length::Fixed(30.0))
            .center_y(Length::Fixed(22.0)),
    )
    .fill(rgb(colors.bg))
    .radius(4.0)
    .width(Length::Fixed(30.0))
    .height(Length::Fixed(22.0))
    .clip(true)
    .into_element();

    let name = enz_text(t.label())
        .size(TextSize::Body)
        .tone(TextTone::Default);

    // The current theme is checked in `IconTone::Accent`; other rows reserve the width so
    // names stay aligned.
    let check: Element<'a, Msg> = if selected {
        enz_icon("check")
            .size(16.0)
            .tint_theme_tone(IconTone::Accent)
            .into_element()
    } else {
        Space::new().width(Length::Fixed(16.0)).into()
    };

    let body = container(
        row![swatch, name, Space::new().width(Length::Fill), check]
            .spacing(spacing.sm())
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .padding(spacing.xs())
    .width(Length::Fill);

    enz_list_row_surface(body, Msg::SetReadingTheme(t))
        .content_padding(0.0)
        .selectable(true)
        .selected(selected)
        .content_height(true)
        .width(Length::Fill)
        .label(t.label())
        .into_element()
}
