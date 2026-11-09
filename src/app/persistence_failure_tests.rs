//! Persistence failure retention, banner route, retry, and close veto.
//!
//! Tests run the real task body and post its `Msg::PersistenceFinished` back through
//! `App::update`. Failures come from a read-only state leaf, so no timing is involved.

use super::*;
use crate::plumbing::persistence::{self};

fn scratch(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-2569-{}-{}-{:?}",
        label,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn broken_state_resolver(dir: &Path) -> enzui_iced_core::persist::XdgResolver {
    let state_root = dir.join("state-root");
    let app_dir = state_root.join("reader-app");
    std::fs::create_dir_all(&app_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    enzui_iced_core::persist::XdgResolver::new(
        None::<PathBuf>,
        Some(state_root),
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

fn restore_broken_state_writes(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let app_dir = dir.join("state-root").join("reader-app");
        std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

fn healthy_state_resolver(dir: &Path) -> enzui_iced_core::persist::XdgResolver {
    enzui_iced_core::persist::XdgResolver::new(
        None::<PathBuf>,
        Some(dir.to_path_buf()),
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

#[test]
fn failed_persistence_retains_debt_without_stalling_other_stores() {
    {
        let dir = scratch("font-up-fail");

        let (mut app, _t) = App::test_new();
        app.enable_persist(broken_state_resolver(&dir));
        app.model_mut().screen = crate::model::Screen::Reading;
        app.model_mut().format = Format::Epub;
        app.model_mut().file_path = Some(dir.join("some-book.epub"));
        app.model_mut().page_count = 10;
        persistence::set_progress(
            std::sync::Arc::make_mut(&mut app.model_mut().app_state),
            &dir.join("some-book.epub").to_string_lossy(),
            persistence::ProgressRecord {
                page: 0,
                total: 10,
                title: "Fixture".to_string(),
                author: "Fixture".to_string(),
                format: "epub".to_string(),
                ..Default::default()
            },
        );

        assert!(!app.persistence_pending());
        let _ = app.update(Msg::FontSizeUp);
        assert!(
            app.persistence_pending(),
            "FontSizeUp must mark the library dirty (progress_dirty)"
        );

        let _ = app.update(Msg::FlushPersistence);
        assert!(
            !app.persistence_pending(),
            "flush clears the dirty flags; the tick must not free-run on debt"
        );
        let _ = app.drain_persistence_debt_for_test();

        assert!(
            app.has_persistence_failure(),
            "a failed library completion must retain the debt entry, not clear the map"
        );
        let alert = app
            .model()
            .persistence_alert
            .clone()
            .expect("model.persistence_alert must carry the bounded safe summary");
        assert!(
            !alert.contains(dir.to_string_lossy().as_ref()),
            "banner summary must not contain a destination path: {alert:?}"
        );
        assert!(
            !alert.contains('\n') && !alert.contains('\r'),
            "banner summary is bounded single-line: {alert:?}"
        );
        let diagnostics = app.persistence_failure_diagnostics();
        assert_eq!(diagnostics.len(), 1, "one failed destination is retained");
        assert_ne!(
            diagnostics[0], alert,
            "raw diagnostic must be distinct from the bounded summary"
        );

        let shown = |app: &App| {
            crate::view::shell::notices(app.model())
                .iter()
                .any(|notice| format!("{notice:?}").contains("reader-persistence"))
        };
        assert!(shown(&app), "the failure is a bottom-right notice");
        let _ = app.update(Msg::HidePersistenceAlert);
        assert!(!shown(&app), "closing the notice hides it");
        assert!(
            app.has_persistence_failure(),
            "closing the notice must not settle the retained debt"
        );

        restore_broken_state_writes(&dir);
        app.enable_persist(healthy_state_resolver(&dir));
        let _ = app.update(Msg::RetryPersistence);
        let _ = app.drain_persistence_debt_for_test();
        assert!(
            !app.has_persistence_failure(),
            "a successful retry must retire the failed entry"
        );
        assert!(
            app.model().persistence_alert.is_none(),
            "clearing the last failure must clear the alert"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        let dir = scratch("indep");
        let broken_ann_root = dir.join("ann-root-read-only");

        let (mut app, _t) = App::test_new();
        app.enable_persist(healthy_state_resolver(&dir));
        app.annotations_root = Some(broken_ann_root.clone());
        app.model_mut().screen = crate::model::Screen::Reading;
        app.model_mut().format = Format::Pdf;
        let doc = dir.join("doc.pdf");
        std::fs::write(&doc, b"%PDF-1.4\n").unwrap();
        app.model_mut().file_path = Some(doc.clone());
        app.model_mut().file_name = "doc.pdf".to_string();
        app.model_mut().page_count = 5;
        app.load_annotations_for_document(&doc);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&broken_ann_root, std::fs::Permissions::from_mode(0o500))
                .unwrap();
        }

        let _ = app.update(Msg::ToggleBookmark(1));
        persistence::set_progress(
            std::sync::Arc::make_mut(&mut app.model_mut().app_state),
            "/nonexistent/other-book.pdf",
            persistence::ProgressRecord::default(),
        );
        let _ = app.update(Msg::RemoveFromRecents(
            "/nonexistent/other-book.pdf".to_string(),
        ));

        let _ = app.update(Msg::FlushPersistence);
        let _ = app.drain_persistence_debt_for_test();

        assert!(
            app.has_persistence_failure(),
            "the annotation sidecar failure must be retained"
        );
        let diagnostics = app.persistence_failure_diagnostics();
        assert_eq!(
            diagnostics.len(),
            1,
            "exactly ONE destination is retained (the sidecar); the library was\n         retired independently. got={diagnostics:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn close_window_vetoes_on_persistence_failure() {
    let dir = scratch("close-veto");
    let (mut app, _t) = App::test_new();
    app.enable_persist(broken_state_resolver(&dir));
    app.model_mut().screen = crate::model::Screen::Reading;
    app.model_mut().format = Format::Epub;
    app.model_mut().file_path = Some(dir.join("book.epub"));
    persistence::set_progress(
        std::sync::Arc::make_mut(&mut app.model_mut().app_state),
        &dir.join("book.epub").to_string_lossy(),
        persistence::ProgressRecord::default(),
    );
    let _ = app.update(Msg::FontSizeUp);
    let _ = app.update(Msg::CloseWindow(None));
    let _ = app.drain_persistence_debt_for_test();
    assert!(
        app.has_persistence_failure(),
        "close must not clear a failed destination"
    );
    assert!(
        app.model().persistence_alert.is_some(),
        "the persistence alert must remain reachable after a vetoed close"
    );

    restore_broken_state_writes(&dir);
    app.enable_persist(healthy_state_resolver(&dir));
    let _ = app.update(Msg::RetryPersistence);
    let _ = app.drain_persistence_debt_for_test();
    assert!(!app.has_persistence_failure());
    let _ = app.update(Msg::CloseWindow(None));
    let _ = app.drain_persistence_debt_for_test();
    assert!(!app.has_persistence_failure());

    let _ = std::fs::remove_dir_all(&dir);
}
