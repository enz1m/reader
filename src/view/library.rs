//! Filterable persisted library and recent reading progress.

use enzui_iced_core::semantics::HeadingLevel;
use enzui_iced_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey};
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::ButtonSize;
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::icon::{IconTone, enz_icon};
use enzui_iced_widgets::list_row_surface::{
    ListRowSurfaceTokens, TrailingVisibility, enz_list_row_surface,
};
use enzui_iced_widgets::menu_button::{MenuButtonKind, enz_menu_button};
use enzui_iced_widgets::progress_bar::enz_progress_bar;
use enzui_iced_widgets::search_field::enz_search_field;
use enzui_iced_widgets::status_page::enz_status_page;
use enzui_iced_widgets::surface::{SurfaceTone, enz_surface};
use enzui_iced_widgets::text_measure::enz_text_measure;
use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_iced_widgets::virtual_list::enz_virtual_list_variable;
use iced::widget::{container, row};
use iced::{Alignment, Element, Length, Padding};

use crate::context_menu::ContextTarget;
use crate::message::Msg;
use crate::model::Model;
use crate::plumbing::persistence::RecentBook;

mod cards;
mod chrome;
mod list;
mod metrics;
mod rows;
mod text;
mod tracks;

pub use chrome::view;

pub use rows::{
    ALL_BOOKS_TITLE, CONTINUE_READING_TITLE, LIBRARY_FILTER_LABEL, LIBRARY_LIST_ID, LIBRARY_TITLE,
    REMOVE_FROM_RECENTS, remove_from_recents_id,
};
pub(crate) use text::book_row_label;
