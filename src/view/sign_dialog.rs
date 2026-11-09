//! Typed-handwriting and optional certificate-signature controls.
//!
//! Placement chooses the page position before requesting the signed-copy destination.

use enzui_iced_core::semantics::HeadingLevel;
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::combo_box::enz_combo_box;
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::modal_surface::enz_modal_surface;
use enzui_iced_widgets::secure_field::enz_secure_field;
use enzui_iced_widgets::surface::{BorderStyle, RadiusSize, enz_surface};
use enzui_iced_widgets::text_field::enz_text_field;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length};

use crate::message::Msg;
use crate::signing::SignDraft;

pub const TITLE: &str = "Sign document";
pub const NAME_LABEL: &str = "Your name";
pub const FONT_LABEL: &str = "Handwriting";
pub const CERTIFICATE_LABEL: &str = "Choose certificate\u{2026}";
pub const PLACE_LABEL: &str = "Place on page";
pub const CANCEL_LABEL: &str = "Cancel";
pub const PLACING_HINT: &str = "Click where your signature goes.";

/// The name field's id: opening the dialog focuses it.
pub fn name_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("reader-sign-name")
}

// The dialog's width: room for a long name in a wide script face.
const DIALOG_WIDTH: f32 = 480.0;
// The preview strip's height.
const PREVIEW_HEIGHT: f32 = 88.0;

pub fn view<'a>(draft: &'a SignDraft, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;
    let heading = enz_text(TITLE)
        .size(TextSize::Title3)
        .heading(HeadingLevel::Page);

    let name = enz_text_field(NAME_LABEL, "Your name", &draft.name, Msg::SignNameChanged)
        .on_submit(Msg::PlaceSignature)
        .id(name_field_id())
        .into_element();
    let fonts: Vec<String> = draft.fonts.iter().map(|f| f.family.clone()).collect();
    let font = enz_combo_box(
        FONT_LABEL,
        fonts,
        Some(draft.family.clone()),
        Msg::SignFontSelected,
    )
    .width(Length::Fill)
    .error(if draft.certificate.is_none() {
        draft.error.as_deref().unwrap_or("")
    } else {
        ""
    })
    .into_element();

    // The signature as it will look: dark ink on paper.
    let preview: Element<'a, Msg> = match draft.preview.as_ref() {
        Some(handle) => iced::widget::image(handle.clone())
            .height(Length::Fixed(PREVIEW_HEIGHT - 16.0))
            .into(),
        None => enz_text("Type your name to see it written.")
            .size(TextSize::Body)
            .tone(TextTone::Muted)
            .into_element(),
    };
    // Paper behind the ink, whatever the theme: the signature is written on
    // a white page.
    let preview = enz_surface(
        container(preview)
            .center_x(Length::Fill)
            .center_y(Length::Fixed(PREVIEW_HEIGHT)),
    )
    .fill(iced::Color::WHITE)
    .border_style(BorderStyle::None)
    .radius_size(RadiusSize::Md)
    .width(Length::Fill)
    .into_element();

    let certificate: Element<'a, Msg> = match draft.certificate.as_ref() {
        None => column![
            enz_text("Add a certificate to make a digital signature that viewers can verify.")
                .size(TextSize::Footnote)
                .tone(TextTone::Muted),
            enz_button(CERTIFICATE_LABEL, Msg::ChooseSigningCertificate)
                .id(iced::widget::Id::new("reader.sign.certificate"))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .icon("file-key")
                .into_element(),
        ]
        .spacing(spacing.xs())
        .into(),
        Some(path) => column![
            row![
                enz_text(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                )
                .size(TextSize::Body)
                .single_line_elided(),
                Space::new().width(Length::Fill),
                enz_button("Remove", Msg::ClearSigningCertificate)
                    .id(iced::widget::Id::new("reader.sign.remove-certificate"))
                    .kind(ButtonKind::Quiet)
                    .size(ButtonSize::Sm)
                    .into_element(),
            ]
            .align_y(Alignment::Center),
            enz_secure_field(
                &draft.password,
                Msg::SignPasswordChanged,
                draft.password_revealed,
                Msg::ToggleSignPasswordReveal,
            )
            .label("Certificate password")
            .error(draft.error.as_deref().unwrap_or(""))
            .on_submit(Msg::PlaceSignature)
            .id_prefix("reader-sign-password")
            .into_element(),
        ]
        .spacing(spacing.xs())
        .into(),
    };

    let buttons = row![
        Space::new().width(Length::Fill),
        enz_button(CANCEL_LABEL, Msg::CancelSigning)
            .id(iced::widget::Id::new("reader.sign.cancel"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        enz_button(PLACE_LABEL, Msg::PlaceSignature)
            .id(iced::widget::Id::new("reader.sign.place"))
            .kind(ButtonKind::Accent)
            .size(ButtonSize::Sm)
            .enabled(draft.ready())
            .into_element(),
    ]
    .spacing(spacing.sm())
    .align_y(Alignment::Center);

    enz_modal_surface(
        column![heading, name, font, preview, certificate, buttons]
            .spacing(spacing.md())
            .width(Length::Fill)
            .into(),
    )
    .width(Length::Fixed(DIALOG_WIDTH))
    .padding(spacing.lg())
    .into_element()
}

/// The bar over the pages while placing: what to do, and a way out.
pub fn placing_bar<'a>(theme: &'a EnzTheme) -> Element<'a, Msg> {
    use enzui_iced_widgets::glass::enz_glass;
    let spacing = &theme.spacing;
    enz_glass(
        container(
            row![
                enz_text(PLACING_HINT).size(TextSize::Body),
                enz_button(CANCEL_LABEL, Msg::CancelSigning)
                    .id(iced::widget::Id::new("reader.sign.cancel"))
                    .kind(ButtonKind::Quiet)
                    .size(ButtonSize::Sm)
                    .into_element(),
            ]
            .spacing(spacing.md())
            .align_y(Alignment::Center),
        )
        .padding([spacing.xs(), spacing.md()]),
    )
    .style(enzui_iced_theme::MaterialStyle::FlatFrosted)
    .tint_opacity(0.18)
    .ink(enzui_iced_theme::GlassInk::Adaptive)
    .radius(crate::view::shape_radius(theme, theme.shape.popover))
    .elevation(3)
    .into()
}
