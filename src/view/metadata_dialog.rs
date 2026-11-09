//! PDF metadata editing and read-only document properties.

use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::banner::{BannerTone, enz_banner};
use enzui_iced_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_iced_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_iced_widgets::modal_surface::enz_modal_surface;
use enzui_iced_widgets::preferences_group::enz_preferences_group;
use enzui_iced_widgets::preferences_row::enz_preferences_row;
use enzui_iced_widgets::scroll_view::enz_scroll_view;
use iced::widget::{Space, column, row};
use iced::{Alignment, Element, Length};

use crate::actions::translated_label;
use crate::message::Msg;
use crate::model::Model;
use crate::plumbing::pdf_ops::{MetadataField, PDF_DATE_REMOVAL_NOTICE, format_file_size};

/// The toolbar control's accessible name and tooltip, in sentence case.
pub const EDIT_METADATA_ACTION: &str = "Edit metadata";
/// The dialog heading, in title case like every dialog title.
pub const EDIT_METADATA_TITLE: &str = "Edit Metadata";
/// The dialog's supporting line.
pub const EDIT_METADATA_DESCRIPTION: &str = "Edit the PDF metadata fields.";
/// The button inside the metadata dialog.
pub const CLEAR_METADATA_BUTTON: &str = "Clear Metadata";
/// The confirm dialog's title and the menu wording, spelling out the scope.
pub const CLEAR_ALL_METADATA: &str = "Clear all metadata";
/// The `Document Properties` heading.
pub const PROPERTIES_TITLE: &str = "Document Properties";
pub const SAVE_LABEL: &str = "Save";
pub const CLOSE_LABEL: &str = "Close";

// Use the large dialog tier so date controls fit inline; the modal host clamps it at narrow
// widths.
fn dialog_width() -> f32 {
    enzui_iced_theme::SizingTokens::default().dialog_width_lg
}

// The width of a date row's controls: the day picker (about 120 px), an
// 86 px time field and a 28 px remove button, 6 px apart.
const DATE_CONTROL_WIDTH: f32 = 124.0 + 86.0 + 28.0 + 2.0 * 6.0;

fn date_control<'a>(model: &'a Model, field: MetadataField, slot: usize) -> Element<'a, Msg> {
    use enzui_iced_widgets::date_picker::enz_date_picker;
    use enzui_iced_widgets::time_field::enz_time_field;
    use enzui_iced_widgets::toolbar_icon::enz_toolbar_icon;
    let parts = crate::plumbing::pdf_ops::pdf_date_parts(field.get(&model.metadata));
    let ui = &model.metadata_dates[slot];
    let picker = enz_date_picker(
        parts.as_ref().map(|(date, _, _)| *date),
        &ui.picker,
        move |event| Msg::MetadataDatePicker(field, event),
    )
    .label(translated_label(field.label()))
    .label_visible(false)
    .placeholder(translated_label("No date"));
    let time = enz_time_field(
        &ui.time_draft,
        parts.as_ref().map_or(
            enzui_iced_core::time_of_day::TimeOfDay::MIDNIGHT,
            |(_, time, _)| *time,
        ),
        move |event| Msg::MetadataTime(field, event),
    )
    .format(crate::model::metadata_time_format())
    .id(enzui_iced_core::identity::derive_id(
        "enz-text-field",
        &format!("{} time", field.label()),
    ))
    .aria_label(translated_label(&format!("{} time", field.label())));
    let mut controls = row![
        picker,
        iced::widget::container(time).width(Length::Fixed(86.0)),
    ]
    .spacing(6.0)
    .align_y(Alignment::Center);
    if parts.is_some() {
        controls = controls.push(
            enz_toolbar_icon("x", Msg::ClearMetadataDate(field))
                .id(enzui_iced_core::identity::derive_id(
                    "reader.metadata.remove-date",
                    field.label(),
                ))
                .aria_label(translated_label(&format!(
                    "Remove {}",
                    field.label().to_lowercase()
                )))
                .tooltip(translated_label("Remove date"))
                .into_element(),
        );
    }
    controls.into()
}

/// The metadata editor, as a modal surface for the shell's `enz_modal_host`.
pub fn metadata_view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let mut rows = column![].spacing(spacing.space0).width(Length::Fill);
    // Row order is `MetadataField::ALL` order.
    for &field in MetadataField::ALL {
        let date = match field {
            MetadataField::CreationDate => Some(&model.metadata.creation_date),
            MetadataField::ModificationDate => Some(&model.metadata.modification_date),
            _ => None,
        };
        let mut row = match crate::model::metadata_date_slot(field) {
            // A date is picked from a calendar and a time field rather than
            // typed in the PDF's `D:YYYYMMDDHHmmSS` syntax.
            Some(slot) => enz_preferences_row(translated_label(field.label()))
                .control(date_control(model, field, slot))
                // Room for the day, the time and the remove button.
                .tokens(enzui_iced_widgets::preferences_row::PreferencesRowTokens {
                    inline_field_width: DATE_CONTROL_WIDTH,
                    inline_field_max_width: DATE_CONTROL_WIDTH,
                    ..enzui_iced_widgets::preferences_row::PreferencesRowTokens::default()
                }),
            // Text fields share the date controls' width, so every control
            // lines up in one column wide enough to read a title in.
            None => enz_preferences_row(translated_label(field.label()))
                .entry(
                    field.get(&model.metadata),
                    translated_label(field.placeholder()),
                    move |text| Msg::MetadataFieldChanged(field, text),
                )
                .tokens(enzui_iced_widgets::preferences_row::PreferencesRowTokens {
                    inline_field_width: DATE_CONTROL_WIDTH,
                    inline_field_max_width: DATE_CONTROL_WIDTH,
                    ..enzui_iced_widgets::preferences_row::PreferencesRowTokens::default()
                }),
        };
        row = row.id(enzui_iced_core::identity::derive_id(
            enzui_iced_widgets::preferences_row::DEFAULT_FIELD_ID_PREFIX,
            field.label(),
        ));
        if let Some(date) = date {
            if date.requests_removal() {
                row = row.subtitle(PDF_DATE_REMOVAL_NOTICE).subtitle_wrap(true);
            }
            if let Some(warning) = date.warning() {
                row = row.error(warning);
            }
        }
        rows = rows.push(row.into_element());
    }

    let group = enz_preferences_group(rows)
        .title(&translated_label(EDIT_METADATA_TITLE))
        .description(&translated_label(EDIT_METADATA_DESCRIPTION))
        .into_element();

    let actions = row![
        enz_button(
            translated_label(CLEAR_METADATA_BUTTON),
            Msg::RequestClearMetadata
        )
        .id(iced::widget::Id::new("reader.metadata.clear"))
        .kind(ButtonKind::Destructive)
        .size(ButtonSize::Sm)
        // Nothing to clear is a disabled state, not a no-op button.
        .enabled(!model.metadata.is_empty())
        .into_element(),
        Space::new().width(Length::Fill),
        enz_button(translated_label(CLOSE_LABEL), Msg::CloseOverlay)
            .id(iced::widget::Id::new("reader.metadata.close"))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        enz_button(translated_label(SAVE_LABEL), Msg::SaveMetadata)
            .id(iced::widget::Id::new("reader.metadata.save"))
            .kind(ButtonKind::Accent)
            .size(ButtonSize::Sm)
            .enabled(model.metadata_can_save())
            .into_element(),
    ]
    .spacing(spacing.xs())
    .align_y(Alignment::Center);

    let mut content = column![].spacing(spacing.md()).width(Length::Fill);
    if let Some(failure) = &model.metadata_save_failure {
        content = content.push(
            enz_banner(translated_label("Couldn’t save metadata"))
                .body(failure.message.clone())
                .tone(BannerTone::Danger)
                .action_text(translated_label("Retry"))
                .on_action(Msg::RetryMetadataSave)
                .dismissible(false)
                .id_prefix("reader-metadata-save")
                .into_element(),
        );
    }
    content = content
        .push(enz_scroll_view(group).width(Length::Fill))
        .push(actions);
    enz_modal_surface(content.into())
        .width(Length::Fixed(dialog_width()))
        .padding(spacing.lg())
        .into_element()
}

/// Property labels and formatted values in display order.
pub fn property_rows(model: &Model) -> Vec<(String, String)> {
    let props = &model.properties;
    vec![
        (translated_label("File name"), model.file_name.clone()),
        (translated_label("Pages"), props.page_count.to_string()),
        (
            translated_label("File size"),
            format_file_size(props.file_size_bytes),
        ),
        (translated_label("PDF version"), props.pdf_version.clone()),
        (
            translated_label("Encrypted"),
            translated_label(if props.is_encrypted { "Yes" } else { "No" }),
        ),
    ]
}

/// The read-only `Document Properties` surface.
pub fn properties_view<'a>(model: &'a Model, theme: &'a EnzTheme) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let mut rows = column![].spacing(spacing.space0).width(Length::Fill);
    for (title, value) in property_rows(model) {
        // `enz_preferences_row` borrows for `'a`, so owned values go into a per-row
        // `column` of `enz_text` instead of `.subtitle(&value)`.
        rows = rows.push(
            row![
                enz_text(title).size(TextSize::Body).tone(TextTone::Muted),
                Space::new().width(Length::Fill),
                enz_text(value).size(TextSize::Body),
            ]
            .spacing(spacing.sm())
            .align_y(Alignment::Center),
        );
    }

    let group = enz_preferences_group(rows)
        .title(&translated_label(PROPERTIES_TITLE))
        .into_element();

    enz_modal_surface(
        column![
            group,
            row![
                Space::new().width(Length::Fill),
                enz_button(translated_label(CLOSE_LABEL), Msg::CloseOverlay)
                    .id(iced::widget::Id::new("reader.properties.close"))
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .into_element(),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(spacing.md())
        .width(Length::Fill)
        .into(),
    )
    .width(Length::Fixed(dialog_width()))
    .padding(spacing.lg())
    .into_element()
}
