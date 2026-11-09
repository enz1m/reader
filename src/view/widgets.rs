//! Small shared view helpers: icon buttons.
//!
//! These delegate to [`enz_toolbar_icon`], which composes `enz_fade_tooltip`, so each
//! label is both spoken (`.aria_label(..)`) and shown (`.tooltip(..)`).

use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
use iced::Element;

use crate::message::Msg;

/// A titlebar/toolbar icon button.
///
/// `toggle` separates a two-state control from an ordinary action:
///
/// * `None`: a push action, publishing `Role::Button` with no toggle state.
/// * `Some(state)`: a genuine toggle, publishing its state and painting the active
///   surface when `true`.
///
/// `label` is both spoken and shown as the tooltip.
pub fn icon_button<'a>(
    icon: &str,
    on_press: Msg,
    label: &str,
    toggle: Option<bool>,
) -> Element<'a, Msg> {
    icon_button_with_id(icon, on_press, label, toggle, None)
}

/// Override the label-derived toolbar identity for focus restoration.
pub fn icon_button_with_id<'a>(
    icon: &str,
    on_press: Msg,
    label: &str,
    toggle: Option<bool>,
    id: Option<iced::advanced::widget::Id>,
) -> Element<'a, Msg> {
    let mut b = enz_toolbar_icon(icon.to_string(), on_press)
        .aria_label(crate::actions::translated_label(label))
        .tooltip(crate::actions::translated_label(label));
    if let Some(state) = toggle {
        b = b.active(state);
    }
    b.id(id.unwrap_or_else(|| enzui_iced_core::identity::derive_id("reader.toolbar", label)))
        .into()
}

/// Stable id of the "Page arranger" toolbar button — the invoker and
/// focus-restore target of the PageArranger overlay.
pub fn page_arranger_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader.toolbar.page-arranger")
}

/// Stable id of the titlebar's overflow (⋮) trigger: the restore target of
/// the overlays whose actions live in the overflow menu.
pub fn toolbar_overflow_trigger_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader.toolbar.overflow")
}

/// Stable id of the "Edit metadata" toolbar button — invoker and restore
/// target of the Metadata overlay.
pub fn metadata_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader.toolbar.metadata")
}

/// Stable id of page `page`'s drag-to-select layer — the rectangle a
/// rubber-band selection is measured inside. Per page, because each page is a
/// separate selection surface with its own text.
pub fn page_selection_id(page: usize) -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from(format!("reader.page.selection.{page}"))
}

/// Stable id of the "Properties" toolbar button — invoker and restore target
/// of the Properties overlay.
pub fn properties_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader.toolbar.properties")
}

/// A disabled toolbar icon, announced as a disabled `Role::Button` named by its action
/// (e.g. "Zoom out"). Uses `enz_toolbar_icon` with `.enabled(false)` so it takes the
/// framework disabled treatment and the same 28px slot as its siblings.
pub fn icon_disabled<'a>(icon: &str, label: &str) -> Element<'a, Msg> {
    enz_toolbar_icon(icon.to_string(), Msg::Noop)
        .aria_label(crate::actions::translated_label(label))
        .tooltip(crate::actions::translated_label(label))
        .id(enzui_iced_core::identity::derive_id(
            "reader.toolbar",
            label,
        ))
        .enabled(false)
        .into()
}

pub const TITLEBAR_HEIGHT: f32 = enzui_iced_widgets::app_shell::DEFAULT_SHELL_TITLEBAR_HEIGHT;
/// Icon-button inner padding. Sits between the spacing scale's `xs` (4) and
/// `sm` (8); the scale has no 6px rung, so this stays a named constant rather
/// than shifting every titlebar icon a pixel to hit a token.
pub const ICON_PADDING: f32 = 6.0;
/// Sidebar width at desktop/laptop.
///
/// A literal inside the framework clamp (`sidebar_min_width` 200 .. `sidebar_max_width`
/// 340), not the 260 px `sidebar_width` default: the resting width is a per-shell choice.
pub const SIDEBAR_WIDTH: f32 = 250.0;
/// The welcome-card cover glyph. A deliberately large book glyph filling the
/// 56×72 cover tile; icon geometry sized to the tile, not to the type ladder.
pub const COVER_GLYPH_ICON: f32 = 28.0;

pub(crate) fn reading_theme_scope_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from("reader.reading-theme-popover")
}

pub(crate) fn reading_theme_trigger_id() -> iced::advanced::widget::Id {
    enzui_iced_core::identity::derive_id("reader.toolbar", "Reading theme")
}
