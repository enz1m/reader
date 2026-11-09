//! Zoom, fit, rotation, and view modes.

use super::*;

#[test]
fn epub_view_mode_survives_restart_and_legacy_sidecars_default_to_scrolled() {
    let dir = tmpdir("view-mode-roundtrip");
    let resolver = crate::app::test_xdg_resolver(&dir);
    let path = dir.join("book.epub");
    std::fs::write(&path, b"test fixture path").unwrap();
    let key = persistence::canonical_key(&path.to_string_lossy());

    {
        let (mut app, _task) = App::test_new();
        app.enable_persist(resolver.clone());
        finish_test_epub(&mut app, &path);
        assert_eq!(app.model.epub_view_mode, EpubViewMode::Scrolled);
        let _ = app.update(Msg::ToggleEpubViewMode);
        assert_eq!(app.model.epub_view_mode, EpubViewMode::Paginated);
        let _ = app.update(Msg::CloseWindow(None));
        let _ = app.drain_persistence_debt_for_test();
    }

    {
        let (mut reloaded, _task) = App::test_new();
        reloaded.enable_persist(resolver.clone());
        reloaded.model.app_state =
            std::sync::Arc::new(persistence::load_state_with_resolver(&resolver));
        finish_test_epub(&mut reloaded, &path);
        assert_eq!(
            reloaded.model.epub_view_mode,
            EpubViewMode::Paginated,
            "a fresh app must restore the choice written by the toggle"
        );
    }

    // Removes the field to make an older sidecar.
    let state_path = persistence::state_path_with_resolver(&resolver);
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    json["files"][&key]
        .as_object_mut()
        .unwrap()
        .remove("epub_view_mode");
    std::fs::write(&state_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    assert!(
        json["files"][&key].get("epub_view_mode").is_none(),
        "the legacy-sidecar setup must actually omit the field"
    );

    {
        let (mut legacy, _task) = App::test_new();
        legacy.enable_persist(resolver.clone());
        legacy.model.app_state =
            std::sync::Arc::new(persistence::load_state_with_resolver(&resolver));
        // Positive control: relying on Model::new's default would be self-fulfilling.
        legacy.model.epub_view_mode = EpubViewMode::Paginated;
        finish_test_epub(&mut legacy, &path);
        assert_eq!(
            legacy.model.epub_view_mode,
            EpubViewMode::Scrolled,
            "a sidecar without epub_view_mode uses the documented default"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
