//! Split, encryption, and password prompt.

use super::*;

#[test]
fn password_prompt_handles_success_failure_and_cancellation_without_retaining_secrets() {
    {
        let dir = tmpdir("unlock");
        let locked = dir.join("secret.pdf");
        std::fs::copy(encrypted_fixture(), &locked).unwrap();

        let (mut app, _t) = App::test_new();
        let _ = app.update(Msg::PasswordRequired(locked.clone()));

        let _ = app.update(Msg::PasswordChanged("wrong".to_string()));
        let task = app.update(Msg::SubmitPassword);
        assert!(
            app.model.password_unlocking,
            "the dialog must show it is working while the unlock runs"
        );
        assert!(
            app.model.password_error.is_none(),
            "the previous error clears when a new attempt starts"
        );
        for msg in super::drain_task(task) {
            let _ = app.update(msg);
        }
        assert!(
            !app.model.password_unlocking,
            "the pending flag clears when the attempt finishes"
        );
        assert_eq!(
            app.model.password_error.as_deref(),
            Some(crate::view::states::PASSWORD_WRONG),
            "the error rides on the field, not in a banner"
        );
        assert!(
            app.model.password_input.is_empty(),
            "a rejected password must not be left in the field"
        );
        assert_eq!(
            app.model.overlay,
            Some(Overlay::Password),
            "the prompt stays up so the user can retry"
        );
        assert!(app.model.error.is_none());

        let _ = app.update(Msg::PasswordChanged("testpass123".to_string()));
        let task = app.update(Msg::SubmitPassword);
        assert_eq!(
            app.model.overlay,
            Some(Overlay::Password),
            "the prompt stays up until the off-thread unlock returns"
        );
        assert_eq!(
            app.model.page_count, 0,
            "no page may be rendered by the reducer itself"
        );
        for msg in super::drain_task(task) {
            let _ = app.update(msg);
        }
        assert_eq!(app.model.overlay, None, "the prompt closes on success");
        assert_eq!(app.model.screen, Screen::Reading);
        assert_eq!(app.model.page_count, 1, "the one-page fixture rendered");
        assert_eq!(app.model.file_name, "secret.pdf");

        assert!(app.model.password_input.is_empty());
        assert!(app.model.password_prompt.is_none());
        assert!(app.model.password_error.is_none());
        assert!(!app.model.password_revealed);
        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        let (mut app, _t) = App::test_new();
        let _ = app.update(Msg::PasswordRequired(std::path::PathBuf::from("/x.pdf")));
        let _ = app.update(Msg::PasswordChanged("s3cret".to_string()));
        let _ = app.update(Msg::TogglePasswordReveal);
        assert!(app.model.password_revealed);

        let _ = app.update(Msg::CancelPassword);
        assert!(app.model.password_input.is_empty());
        assert!(app.model.password_prompt.is_none());
        assert!(!app.model.password_revealed, "the reveal flag resets too");
        assert_eq!(app.model.overlay, None);
    }
}
