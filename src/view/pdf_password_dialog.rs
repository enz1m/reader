//! Secure password changes for the current PDF.

use enzui_iced_core::form::{FieldId, FormField, FormModel};
use enzui_iced_theme::EnzTheme;
use enzui_iced_widgets::form_dialog::{FormPresenter, enz_form_dialog};
use iced::Element;

use crate::message::Msg;
use crate::model::{Model, PasswordPurpose};

pub const PROTECT_TITLE: &str = "Protect with Password";
pub const REMOVE_TITLE: &str = "Remove Password";
pub const PROTECT_ACTION: &str = "Protect with Password\u{2026}";
pub const REMOVE_ACTION: &str = "Remove Password\u{2026}";
/// The shortest password the dialog accepts.
pub const MIN_PASSWORD_CHARS: usize = 4;

const PASSWORD: &str = "password";
const CONFIRM: &str = "confirm";

pub fn password_field() -> FieldId {
    FieldId::new(PASSWORD)
}

pub fn confirm_field() -> FieldId {
    FieldId::new(CONFIRM)
}

/// The fields for `purpose`: a new password twice, or the current one.
pub fn form_model(purpose: PasswordPurpose) -> FormModel {
    let fields = match purpose {
        PasswordPurpose::Protect => vec![
            FormField::secure(PASSWORD, "Password")
                .helper(format!("At least {MIN_PASSWORD_CHARS} characters"))
                .required(true),
            FormField::secure(CONFIRM, "Repeat password").required(true),
        ],
        PasswordPurpose::Remove => {
            vec![FormField::secure(PASSWORD, "Current password").required(true)]
        }
    };
    FormModel::new(fields).expect("distinct field ids")
}

/// A fresh presentation of the form for `purpose`.
pub fn present(purpose: PasswordPurpose) -> FormPresenter {
    FormPresenter::new(form_model(purpose)).with_field_id_scope("reader-pdf-password")
}

/// What the dialog says, with the reason the last attempt was refused.
pub fn description(purpose: PasswordPurpose, error: Option<&str>) -> String {
    let about = match purpose {
        PasswordPurpose::Protect => {
            "The PDF will be encrypted (AES-128) and will ask for this password whenever it is opened. \
             There is no way to recover a forgotten password."
        }
        PasswordPurpose::Remove => {
            "The PDF will be saved without encryption and will open without a password."
        }
    };
    match error {
        Some(error) => format!("{error}\n\n{about}"),
        None => about.to_owned(),
    }
}

pub fn view<'a>(model: &'a Model, _theme: &'a EnzTheme) -> Option<Element<'a, Msg>> {
    let form = model.pdf_password_form.as_ref()?;
    let (title, submit) = match form.purpose {
        PasswordPurpose::Protect => (PROTECT_TITLE, "Protect"),
        PasswordPurpose::Remove => (REMOVE_TITLE, "Remove Password"),
    };
    Some(
        enz_form_dialog(title, form.presenter.session(), Msg::PdfPasswordForm)
            .presenter(&form.presenter)
            .description(description(form.purpose, form.error.as_deref()))
            .submit_label(submit)
            .cancel_label("Cancel")
            .into_surface(),
    )
}
