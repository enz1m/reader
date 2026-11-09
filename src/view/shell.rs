//! Flush reading panes with shell-owned headers, controls and drawer geometry.

use enzui_iced_core::action_overflow::{Descriptor, VisibilityPriority};
use enzui_iced_core::placement::Alignment as PlacementAlignment;
use enzui_iced_core::{Action, Icon};
use enzui_iced_theme::{EnzTheme, SpacingScale};
use enzui_iced_widgets::action_bar::ActionItem;
use enzui_iced_widgets::app_shell::{
    ShellLayout, ShellTokens, SidebarHeader, enz_app_shell_with_geometry,
};
use enzui_iced_widgets::banner::{BannerTone, enz_banner};
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::confirm_dialog::enz_confirm_dialog;
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::icon::{IconTone, enz_icon};
use enzui_iced_widgets::list_row_surface::enz_list_row_surface;
use enzui_iced_widgets::shortcuts_dialog::enz_shortcuts_dialog;
use enzui_iced_widgets::surface::enz_surface;
use enzui_iced_widgets::title_bar::enz_title_bar;
use iced::widget::{Space, container, row};
use iced::{Alignment, Element, Length};

use super::widgets::{TITLEBAR_HEIGHT, icon_button, icon_disabled};
use super::{reading, sidebar};
use crate::message::Msg;
use crate::model::{Format, Model, ReadingTheme, Screen, rgb};

mod banners;
mod composition;
mod content;
mod decorations;
mod indicator;
mod shell_view;
mod side_header;
mod theme;
mod titlebar;
mod tokens;
mod toolbar;

pub(crate) use banners::notices;
pub use content::confirm_message;
pub(crate) use indicator::indicator_text;
pub use shell_view::view;
pub(crate) use side_header::content_width_for;
pub(crate) use tokens::shell_tokens;

pub(crate) use theme::reading_theme_control;
