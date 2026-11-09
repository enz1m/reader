//! Read and annotate PDF, EPUB and CBZ documents.
//!
//! [`run`] starts the application. [`plumbing`] exposes document parsing,
//! persistence and editing; [`pdf`] provides Poppler page rendering.

#![deny(unconditional_recursion)]

pub mod actions;
pub mod app;
#[cfg(feature = "center")]
pub mod center;
pub mod context_menu;
mod localization;
pub mod message;
pub mod model;
pub(crate) mod overlay_focus;
pub mod pages;
/// Viewport-sized pages cut out of a chapter's prose, anchored to a character offset so
/// a reflow keeps the reader's place.
pub mod pagination;
pub mod pdf;
pub mod picker;
pub mod plumbing;
pub mod raster;
pub mod reading_pane;
pub(crate) mod recovery_location;
pub mod search;
pub mod services;
pub mod shortcuts;
pub mod signing;
pub mod theme;
pub mod undo;
pub(crate) mod user_copy;
pub mod view;
pub mod viewport;

use enzui_iced_widgets::app_shell::{ShellLayout, enz_window_settings};

#[cfg(test)]
mod live_feedback;

#[cfg(test)]
mod render_tests;

#[cfg(test)]
mod context_menu_tests;

#[cfg(test)]
mod perf_tests;

#[cfg(test)]
mod availability;

pub use plumbing::{
    cbz_support, epub_support, footnotes, pdf_ops, persistence, reading_stats, spread,
};

/// Run the reader application.
pub fn run() -> iced::Result {
    use enzui_iced_widgets::app_setup::{self, AppServices, TraversalBinding};
    use enzui_iced_widgets::single_instance::{InstanceStart, SingleInstance};

    let instance = match SingleInstance::start(DESKTOP_APP_ID) {
        InstanceStart::Forwarded => return Ok(()),
        InstanceStart::Owner(instance, launch) => std::cell::RefCell::new(Some((instance, launch))),
    };
    let program = iced::application(
        move || {
            let (instance, launch) = instance
                .borrow_mut()
                .take()
                .expect("application boots once");
            let (mut app, task) = app::App::new_with_launch(launch);
            app.instance = Some(instance);
            (app, task)
        },
        app::App::update,
        app::App::view,
    )
    .title("Reader")
    .subscription(app::App::subscription)
    .theme(app::App::theme)
    .window(with_desktop_identity(enz_window_settings(
        ShellLayout::TitlebarSidebarContent,
    )))
    .exit_on_close_request(enzui_iced_core::close_guard::EXIT_ON_CLOSE_REQUEST);
    app_setup::run_with_services(
        program,
        AppServices::default()
            .localizer(localization::localizer())
            .accessibility_actions(false)
            .traversal(TraversalBinding::Disabled),
    )
}

const DESKTOP_APP_ID: &str = "io.github.enzui.Reader";

fn with_desktop_identity(settings: iced::window::Settings) -> iced::window::Settings {
    enzui_iced_widgets::app_shell::DesktopApplicationIdentity::new(DESKTOP_APP_ID)
        .expect("BUG: DESKTOP_APP_ID must be a portable desktop-entry basename")
        .apply_to_window_settings(settings)
}
