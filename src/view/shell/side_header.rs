//! Sidebar titlebar and content width.

use super::*;

pub(crate) fn sidebar_titlebar<'a>(model: &'a Model) -> Element<'a, Msg> {
    enz_title_bar()
        .title(sidebar::title(model.sidebar_tab))
        .id("reader-app-sidebar-header")
        .into_element()
}

// The content allocation used by both pagination and the shell.
pub(crate) fn content_width_for(model: &Model, window_width: f32) -> f32 {
    crate::view::shell_geometry(model, window_width)
        .content_width
        .max(1.0)
}
