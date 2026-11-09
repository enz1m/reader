//! Native document titlebar with measured actions and reading position.

use super::toolbar::toolbar_actions;
use super::*;

pub(crate) fn titlebar<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    spacing: &SpacingScale,
    window_width: f32,
) -> Element<'a, Msg> {
    let is_reading = model.screen == Screen::Reading;
    let mut bar = enz_title_bar()
        .id("reader-app-window")
        .actions(
            toolbar_actions(model, theme, spacing),
            Msg::ToggleToolbarOverflow,
        )
        .action_dismiss(Msg::CloseToolbarOverflow)
        .action_menu_id_prefix("reader.toolbar.menu")
        .action_overflow_trigger_id(super::super::widgets::toolbar_overflow_trigger_id())
        .overflow_open(model.toolbar_overflow_open);

    if model.screen != Screen::Library {
        bar = bar.leading(super::super::widgets::icon_button_with_id(
            "book-open",
            Msg::GoLibrary,
            "Browse library",
            None,
            Some(iced::widget::Id::new("reader.library")),
        ));
    } else if window_width >= 520.0 {
        bar = bar.leading(icon_disabled("book-open", "Browse library"));
    }

    if is_reading {
        bar = bar.title(super::super::states::document_name(model));
        if window_width >= 520.0 {
            bar = bar.center(
                enz_text(indicator_text(model))
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted),
            );
        }
    }

    bar.into_element()
}
