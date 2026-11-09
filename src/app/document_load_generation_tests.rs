//! Superseded whole-document loads must not land.
//!
//! `super::drain_task` runs the real load task and returns its `Msg::DocumentLoaded`
//! unapplied, so holding that message keeps a load in flight without threads or sleeps.

use super::*;
use crate::model::Overlay;

fn sandboxed_app(label: &str) -> (App, tempfile::TempDir) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("reader-document-load-{label}-"))
        .tempdir()
        .expect("a private sandbox directory");
    let (mut app, _t) = App::test_new();
    app.enable_persist(crate::app::test_xdg_resolver(dir.path()));
    app.annotations_root = Some(dir.path().join("annotations"));
    (app, dir)
}

fn epub_a() -> std::path::PathBuf {
    std::path::PathBuf::from("tests/fixtures/quiet-coast.epub")
}

fn pdf_b() -> std::path::PathBuf {
    std::path::PathBuf::from("tests/fixtures/test-multipage.pdf")
}

fn start_open_holding_completion(app: &mut App, path: &std::path::Path) -> Msg {
    let mut held = None;
    for msg in super::drain_task(app.update(Msg::OpenPath(path.to_path_buf()))) {
        if matches!(msg, Msg::DocumentLoaded { .. }) {
            assert!(held.is_none(), "one open must produce one completion");
            held = Some(msg);
        } else {
            let _ = app.update(msg);
        }
    }
    held.expect("opening a document must dispatch a DocumentLoaded completion")
}

fn open_and_apply(app: &mut App, path: &std::path::Path) {
    let completion = start_open_holding_completion(app, path);
    let _ = app.update(completion);
}

fn state_has(app: &App, path: &std::path::Path) -> bool {
    let key = persistence::canonical_key(&path.to_string_lossy());
    app.model.app_state.files.contains_key(&key)
}

#[test]
fn document_navigation_rejects_stale_loads_and_preserves_last_good_progress() {
    {
        let (mut app, _sandbox) = sandboxed_app("stale-after-newer");
        let previous = _sandbox.path().join("previous.epub");
        std::fs::copy(epub_a(), &previous).unwrap();
        open_and_apply(&mut app, &previous);
        let _ = app.seed_note_for_test(0, "history from the previous document");
        assert!(!app.model.undo_stack.undo_empty());

        let stale = start_open_holding_completion(&mut app, &epub_a());
        assert!(app.model.undo_stack.undo_empty());

        open_and_apply(&mut app, &pdf_b());
        let _ = app.seed_note_for_test(0, "a note on B");

        let b_title = app.model.display_title.clone();
        let b_pages = app.model.page_count;
        let b_notes = app.model.annotations.annotations.len();
        let b_format = app.model.format;
        assert!(b_pages > 0, "B must really have opened");
        assert_eq!(b_notes, 1, "the note on B must have landed");
        assert!(!app.model.loading, "B finished loading");

        let _ = app.update(stale);

        assert_eq!(
            app.model.file_path.as_deref(),
            Some(pdf_b().as_path()),
            "the open document is still B"
        );
        assert_eq!(app.model.format, b_format, "B's format must survive");
        assert_eq!(app.model.page_count, b_pages, "B's page count must survive");
        assert_eq!(app.model.display_title, b_title, "B's title must survive");
        assert!(
            app.model.epub.is_none(),
            "a superseded EPUB must not be installed over the open PDF"
        );
        assert_eq!(
            app.model.annotations.annotations.len(),
            b_notes,
            "B's annotations must not be replaced by the superseded document's sidecar"
        );
        assert!(
            app.model.password_prompt.is_none() && app.model.overlay != Some(Overlay::Password),
            "no unlock prompt may be raised by a superseded load"
        );
        assert!(
            !state_has(&app, &epub_a()),
            "a superseded load must not write a progress/recents record"
        );
        assert!(
            state_has(&app, &pdf_b()),
            "positive control: the applied open DOES record its progress"
        );
    }
    {
        let (mut app, sandbox) = sandboxed_app("malformed-close");
        let resolver = crate::app::test_xdg_resolver(sandbox.path());
        let path = sandbox.path().join("malformed.epub");
        std::fs::write(&path, b"not a zip archive").unwrap();

        let expected = persistence::ProgressRecord {
            page: 1,
            total: 2,
            title: "Last good title".to_string(),
            format: "epub".to_string(),
            epub_view_mode: Some("scrolled".to_string()),
            epub_anchor: Some(persistence::EpubProgressAnchor {
                paragraph: 3,
                char_offset: 12,
            }),
            ..persistence::ProgressRecord::default()
        };
        let mut state = persistence::AppState::default();
        persistence::set_progress(&mut state, &path.to_string_lossy(), expected.clone());
        persistence::save_state_with_resolver(&state, &resolver).unwrap();
        app.seed_app_state_for_test(state);

        let completion = start_open_holding_completion(&mut app, &path);
        let _ = app.update(completion);
        assert_eq!(app.model.screen, Screen::Library);
        assert_eq!(
            app.model.error.as_deref(),
            Some("The EPUB archive is corrupt.")
        );
        assert!(
            app.model.file_path.is_none(),
            "a failed load must not remain the current document"
        );

        let _ = app.update(Msg::CloseWindow(None));
        let _ = app.drain_persistence_debt_for_test();
        let reloaded = persistence::load_state_with_resolver(&resolver);
        let key = persistence::canonical_key(&path.to_string_lossy());
        assert_eq!(reloaded.files.get(&key), Some(&expected));
    }
}
