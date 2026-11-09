//! `Protect with Password…` and `Remove Password…` through the dialog.

use super::*;
use crate::model::{Format, Overlay, PasswordPurpose, Screen};
use crate::view::pdf_password_dialog::{confirm_field, password_field};
use enzui_iced_core::form::{FieldId, FieldValue};
use enzui_iced_widgets::form_dialog::FormDialogEvent;

fn app_with_pdf(label: &str) -> (App, std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "reader-pdf-password-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("doc.pdf");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf"),
        &pdf,
    )
    .unwrap();
    let (mut app, _t) = App::test_new();
    app.model.file_path = Some(pdf.clone());
    app.model.file_name = "doc.pdf".to_string();
    app.model.format = Format::Pdf;
    app.model.screen = Screen::Reading;
    app.model.page_count = 5;
    app.model.total_page_count = 5;
    (app, pdf, dir)
}

fn submit(app: &mut App, fields: &[(FieldId, &str)]) {
    for (field, value) in fields {
        let _ = app.update(Msg::PdfPasswordForm(FormDialogEvent::Edited(
            field.clone(),
            FieldValue::secure((*value).to_string()),
        )));
    }
    let _ = app.update(Msg::PdfPasswordForm(FormDialogEvent::SubmitRequested));
}

fn encrypted(pdf: &std::path::Path) -> bool {
    lopdf::Document::load(pdf).unwrap().is_encrypted()
}

#[test]
fn pdf_password_changes_require_confirmation_and_current_authority() {
    {
        let (mut app, pdf, dir) = app_with_pdf("protect");
        let _ = app.update(Msg::OpenPdfPasswordForm);
        assert_eq!(app.model.overlay, Some(Overlay::PdfPassword));
        assert_eq!(
            app.model
                .pdf_password_form
                .as_ref()
                .map(|form| form.purpose),
            Some(PasswordPurpose::Protect)
        );

        submit(
            &mut app,
            &[(password_field(), "abcd1"), (confirm_field(), "abcd2")],
        );
        assert!(!encrypted(&pdf), "mismatched: nothing written");
        let form = app.model.pdf_password_form.as_ref().expect("still open");
        assert_eq!(form.error.as_deref(), Some("The passwords don't match."));

        submit(
            &mut app,
            &[(password_field(), "abcd1"), (confirm_field(), "abcd1")],
        );
        assert!(encrypted(&pdf));
        assert!(app.model.pdf_password_form.is_none(), "closed");
        assert_ne!(app.model.overlay, Some(Overlay::PdfPassword));
        assert_eq!(
            app.model.pdf_password.as_deref(),
            Some("abcd1"),
            "later pages render with it"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let (mut app, pdf, dir) = app_with_pdf("remove");
        crate::plumbing::pdf_ops::set_password(&pdf, "abcd1").unwrap();
        app.model.pdf_password = Some("abcd1".to_string());

        let _ = app.update(Msg::OpenPdfPasswordForm);
        assert_eq!(
            app.model
                .pdf_password_form
                .as_ref()
                .map(|form| form.purpose),
            Some(PasswordPurpose::Remove),
            "a PDF with a password offers to remove it"
        );
        submit(&mut app, &[(password_field(), "wrong")]);
        assert!(encrypted(&pdf), "a wrong password changes nothing");
        assert!(
            app.model
                .pdf_password_form
                .as_ref()
                .unwrap()
                .error
                .is_some()
        );

        submit(&mut app, &[(password_field(), "abcd1")]);
        assert!(!encrypted(&pdf));
        assert_eq!(app.model.pdf_password, None);
        assert!(app.model.pdf_password_form.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
