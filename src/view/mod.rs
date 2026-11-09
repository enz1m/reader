//! View layer: reads only the `Model`, never touches the filesystem.

pub mod annotations_panel;
pub mod library;
pub mod metadata_dialog;
pub mod note_editor;
pub mod page_arranger;
pub mod password_dialog;
pub mod pdf_password_dialog;
pub mod reading;
pub mod search_panel;
pub mod shell;
pub mod sidebar;
pub mod sign_dialog;
pub mod states;
pub mod widgets;

pub use shell::view;

use crate::model::{Model, Screen};
use enzui_iced_widgets::app_shell::{ShellGeometry, ShellLayout, resolve_shell_geometry};
use enzui_iced_widgets::nav_split::SidebarPresentation;

pub(crate) fn shell_geometry(model: &Model, window_width: f32) -> ShellGeometry {
    let layout = if model.screen == Screen::Reading {
        ShellLayout::TitlebarSidebarContent
    } else {
        ShellLayout::TitlebarContent
    };
    resolve_shell_geometry(
        layout,
        model.nav,
        shell::shell_tokens().with_flush_defaults(layout),
        Default::default(),
        window_width,
    )
}

/// Resolves navigation presentation with the shell's width policy.
pub fn sidebar_presentation(model: &Model, window_width: f32) -> SidebarPresentation {
    if window_width <= 0.0 {
        return SidebarPresentation::Hidden;
    }
    shell_geometry(model, window_width).sidebar
}

/// The narrowest content pane that justifies an inline sidebar.
///
/// With a 250 px sidebar, 768 px leaves 518 (overlay) and 1024 px leaves 774 (inline).
/// Reflowable reading text needs more room than the framework default of 320.
pub const SIDEBAR_CONTENT_MINIMUM: f32 = 600.0;

pub(crate) fn shape_radius(
    theme: &enzui_iced_theme::EnzTheme,
    step: enzui_iced_theme::RadiusStep,
) -> f32 {
    match step {
        enzui_iced_theme::RadiusStep::Window => theme.window_radius,
        step => theme.radius.step(step),
    }
}

pub(crate) fn reading_colors(
    reading: crate::model::ReadingTheme,
    theme: &enzui_iced_theme::EnzTheme,
) -> crate::model::ReadingThemeColors {
    use crate::model::ReadingTheme;
    let mut colors = reading.colors();
    if matches!(reading, ReadingTheme::Light | ReadingTheme::Dark) {
        let hex = |color: iced::Color| {
            let [r, g, b, _] = color.into_rgba8();
            u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        };
        let palette = match (reading, theme.mode()) {
            (ReadingTheme::Light, enzui_iced_theme::EnzThemeMode::Dark) => {
                enzui_iced_theme::Palette::light()
            }
            (ReadingTheme::Dark, enzui_iced_theme::EnzThemeMode::Light) => {
                enzui_iced_theme::Palette::dark()
            }
            _ => theme.palette.clone(),
        };
        colors.bg = hex(palette.background);
        colors.card_bg = colors.bg;
        colors.surface_bg = colors.bg;
        colors.fg = hex(palette.text);
        colors.link = hex(palette.accent);
        colors.separator = hex(palette.border);
    }
    colors
}
