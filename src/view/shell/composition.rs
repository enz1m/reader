//! Shell composition.

use super::banners::recovery_panel;
use super::content::content_area;
use super::side_header::sidebar_titlebar;
use super::*;

// Keys of the content column's recovery panels; the page row uses 5.
const RECOVERY_KEY: usize = 100;

pub(crate) fn shell_composition<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    spacing: &'a SpacingScale,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Msg> {
    let gap0 = spacing.space0;

    // Error, persistence and publication notices float bottom-right with the toasts
    // (`banners::notices`); only the state-recovery panels stay above the page.
    let content = move |geometry: &enzui_iced_widgets::app_shell::ShellGeometry| {
        let content_width = geometry.content_width;
        let inner = content_area(model, theme, content_width, window_height);
        // Keyed, so a recovery panel arriving above the page does not shift
        // the page's place in the column and reset its scroll.
        let mut rows: Vec<(usize, Element<Msg>)> = Vec::new();
        for (index, authority) in model.recovery_authorities.iter().enumerate() {
            let revealed = model
                .revealed_recovery_location
                .as_ref()
                .filter(|(origin, _)| origin == &authority.origin)
                .map(|(_, path)| path.as_path());
            rows.push((
                RECOVERY_KEY + index,
                recovery_panel(authority, revealed, theme),
            ));
        }
        rows.push((5, inner));
        iced::widget::keyed_column(rows)
            .spacing(gap0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    // Sidebar slots are supplied only while reading: a supplied slot is what makes the
    // shell's toggle exist, and off the reading screen it would reveal an empty pane.
    let mut builder = enz_app_shell_with_geometry(content)
        .layout(ShellLayout::TitlebarSidebarContent)
        .on_window_gesture(Msg::Window)
        .tokens(shell_tokens())
        // Without the app's `NavSplitState` the overlay drawer cannot open at the compact
        // rung.
        .state(model.nav)
        .sidebar_label("Document navigation")
        .titlebar(move || super::titlebar::titlebar(model, theme, spacing, window_width));

    if model.screen == Screen::Reading {
        builder = builder
            // The sidebar's own header, drawn atop the full-height pane under `Inside`.
            .sidebar_titlebar(move || sidebar_titlebar(model))
            .sidebar(move || sidebar::view(model, theme));
    }

    builder
        .on_toggle_sidebar(Msg::ToggleSidebar)
        // The overlay drawer's scrim dismissal still routes to the app.
        .on_dismiss_overlay(Msg::DismissSidebar)
        .into_element()
}
