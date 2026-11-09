//! Shell tokens and sidebar title.

use super::*;

// Window layout with an independent sidebar header and flush reading content.
pub(crate) fn shell_tokens() -> ShellTokens {
    ShellTokens {
        sidebar_width: super::super::widgets::SIDEBAR_WIDTH,
        sidebar_width_adaptive: false,
        content_minimum: crate::view::SIDEBAR_CONTENT_MINIMUM,
        sidebar_header: SidebarHeader::Inside,
        ..ShellTokens::flush_page()
    }
}
