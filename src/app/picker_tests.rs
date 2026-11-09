//! Reducer-level tests for the Open File portal chooser, driven through
//! `picker::Picker::with_backend` so no D-Bus or dialog is touched.

use super::*;
use crate::picker::tests::ScriptedBackend;
use enzui_iced_widgets::file_chooser::PortalResponse;

fn app_with(backend: ScriptedBackend) -> App {
    let (mut app, _t) = App::test_new();
    app.picker = crate::picker::Picker::with_backend(Box::new(backend));
    app
}

fn fixture_uri() -> (std::path::PathBuf, String) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("quiet-coast.epub");
    let uri = format!("file://{}", path.to_string_lossy());
    (path, uri)
}

#[test]
fn chooser_success_opens_the_document_and_cancellation_is_silent() {
    {
        let (path, uri) = fixture_uri();
        let mut app = app_with(ScriptedBackend::replying(
            PortalResponse::Success,
            vec![uri],
        ));

        let _ = app.update(Msg::OpenDialog);
        let _ = app.update(Msg::PortalTick);

        assert_eq!(
            app.model.file_path.as_deref(),
            Some(path.as_path()),
            "a completed pick must open the chosen file through the open path"
        );
        assert_eq!(
            app.model.screen,
            Screen::Reading,
            "opening the file must route to the reading screen"
        );
        assert!(
            app.model.error.is_none() && app.model.picker_failure.is_none(),
            "a successful open must raise no failure banner, got document={:?}, picker={:?}",
            app.model.error,
            app.model.picker_failure,
        );
        assert!(
            !app.picker.is_pending(),
            "a completed pick must clear the request slot"
        );
        assert!(
            app.model.toasts.is_empty(),
            "opening the chosen document is the visible success; it must not add a redundant toast"
        );
    }
    {
        let mut app = app_with(ScriptedBackend::replying(PortalResponse::Cancelled, vec![]));
        let screen_before = app.model.screen;

        let _ = app.update(Msg::OpenDialog);
        let _ = app.update(Msg::PortalTick);

        assert!(
            app.model.file_path.is_none(),
            "cancelling must not open any file"
        );
        assert_eq!(
            app.model.screen, screen_before,
            "cancelling must not change the screen"
        );
        assert!(
            app.model.error.is_none() && app.model.picker_failure.is_none(),
            "cancelling is a decision, not an error, got document={:?}, picker={:?}",
            app.model.error,
            app.model.picker_failure,
        );
        assert!(!app.picker.is_pending(), "a cancel must clear the slot");
    }
}
