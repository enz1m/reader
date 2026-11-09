use super::*;
use enzui_iced_core::activation::ApplicationIntent;

#[test]
fn desktop_launch_waits_for_restore_and_preserves_live_editing() {
    {
        let root = tempfile::tempdir().unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(root.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let (mut app, _) = App::test_new();
        app.model.library_loading = true;
        let _ = app.receive_launch(ApplicationIntent::Open(vec![
            "file:///tmp/newer.epub".into(),
        ]));
        assert_eq!(
            app.pending_launch_document.as_deref(),
            Some(Path::new("/tmp/newer.epub"))
        );
        assert!(app.model.file_path.is_none());
        let reload =
            persistence::reload_recovery(persistence::StateOrigin::Library, &resolver, None);
        let _ = app.apply_startup_library_restore(
            app.library_recovery_generation,
            0,
            Some("/tmp/initial.epub".into()),
            reload,
        );
        assert_eq!(
            app.model.file_path.as_deref(),
            Some(Path::new("/tmp/newer.epub"))
        );
        assert!(app.pending_launch_document.is_none());
    }
    {
        let (mut app, _) = App::test_new();
        app.model.overlay = Some(model::Overlay::Metadata);
        let _ = app.receive_launch(ApplicationIntent::Open(vec![
            "file:///tmp/another.pdf".into(),
        ]));
        assert_eq!(app.model.overlay, Some(model::Overlay::Metadata));
        assert!(app.model.file_path.is_none());
        assert!(app.model.picker_failure.is_some());
    }
}
