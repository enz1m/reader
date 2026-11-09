//! Secure encrypted-PDF unlock form.
//!
//! Passwords are excluded from accessibility text and cleared when the prompt closes.

use enzui_iced_core::semantics::HeadingLevel;
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::modal_surface::enz_modal_surface;
use enzui_iced_widgets::secure_field::enz_secure_field;
use iced::widget::{Space, column, row};
use iced::{Alignment, Element, Length};

use enzui_iced_widgets::spinner::enz_spinner;

use super::states::{
    PASSWORD_LABEL, PASSWORD_MESSAGE, PASSWORD_PLACEHOLDER, PASSWORD_REQUIRED, PASSWORD_UNLOCKING,
    UNLOCK_LABEL,
};
use crate::actions::translated_label;
use crate::message::Msg;
use crate::model::Model;

pub const CANCEL_LABEL: &str = "Cancel";

// Keep the single-field unlock narrower than multi-field forms.
const DIALOG_WIDTH: f32 = 420.0;

/// Whether `Unlock` can be pressed: only with a non-empty password.
///
/// An empty submission would only repeat the wrong-password error, and a running unlock
/// would start a second rasterisation.
pub fn can_unlock(model: &Model) -> bool {
    !model.password_input.is_empty() && !model.password_unlocking
}

pub fn view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    // Page rung: `enz_modal_host` makes the presenting screen inert, so this is the
    // dialog frame's only heading.
    let heading = enz_text(translated_label(PASSWORD_REQUIRED))
        .size(TextSize::Title3)
        .heading(HeadingLevel::Page);
    // While the off-thread unlock runs the dialog shows a spinner.
    let message: Element<'a, Msg> = if model.password_unlocking {
        row![
            enz_spinner()
                .size(16.0)
                .label(translated_label(PASSWORD_UNLOCKING))
                .into_element(),
            enz_text(translated_label(PASSWORD_UNLOCKING))
                .size(TextSize::Body)
                .tone(TextTone::Muted)
                .into_element(),
        ]
        .spacing(spacing.sm())
        .align_y(Alignment::Center)
        .into()
    } else {
        enz_text(translated_label(PASSWORD_MESSAGE))
            .size(TextSize::Body)
            .tone(TextTone::Muted)
            .into_element()
    };

    // The framework secure field: masked, reveal-toggleable, and its value is
    // never exported to the AT tree. Enter submits, so a keyboard user need
    // not tab to the button.
    let field = enz_secure_field(
        &model.password_input,
        Msg::PasswordChanged,
        model.password_revealed,
        Msg::TogglePasswordReveal,
    )
    .label(translated_label(PASSWORD_LABEL))
    .placeholder(translated_label(PASSWORD_PLACEHOLDER))
    .error(match model.password_error.as_deref() {
        Some(super::states::PASSWORD_WRONG) => translated_label(super::states::PASSWORD_WRONG),
        Some(error) => error.to_string(),
        None => String::new(),
    })
    .on_submit(Msg::SubmitPassword)
    .id(enzui_iced_core::identity::derive_id(
        "reader-password",
        PASSWORD_LABEL,
    ))
    .into_element();

    let buttons = row![
        Space::new().width(Length::Fill),
        enz_button(translated_label(CANCEL_LABEL), Msg::CancelPassword)
            .id(iced::widget::Id::new("reader.password.cancel"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        enz_button(translated_label(UNLOCK_LABEL), Msg::SubmitPassword)
            .id(iced::widget::Id::new("reader.password.unlock"))
            .kind(ButtonKind::Accent)
            .size(ButtonSize::Sm)
            .enabled(can_unlock(model))
            .into_element(),
    ]
    .spacing(spacing.sm())
    .align_y(Alignment::Center);

    enz_modal_surface(
        column![heading, message, field, buttons]
            .spacing(spacing.md())
            .width(Length::Fill)
            .into(),
    )
    .width(Length::Fixed(DIALOG_WIDTH))
    .padding(spacing.lg())
    .into_element()
}
