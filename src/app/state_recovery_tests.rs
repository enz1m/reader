use super::*;
use crate::plumbing::pdf_ops::{PdfAnnotationSnapshot, StoredAnnotation};
use crate::plumbing::persistence::{
    self, AuthorityLoad, QuarantineStatus, StateFailureKind, StateOrigin,
};
use enzui_iced_core::collect::{CollectedNode, SemanticsCollector};
use enzui_iced_core::portal::backend::{BackendError, RequestId};
use enzui_iced_core::portal::open_uri::{OpenUri, OpenUriBackend, OpenUriReply};
use enzui_iced_core::portal::parent::ParentHandle;
use iced::advanced::renderer::Headless;
use iced::advanced::widget::Operation;

struct RecordingOpenUriBackend {
    opened: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl OpenUriBackend for RecordingOpenUriBackend {
    fn start_open_uri(
        &mut self,
        uri: &OpenUri,
        _parent: &ParentHandle,
    ) -> Result<RequestId, BackendError> {
        self.opened.lock().unwrap().push(uri.as_str().to_string());
        Ok(RequestId::new(1))
    }

    fn poll_open_uri(&mut self, _id: RequestId) -> Option<OpenUriReply> {
        Some(OpenUriReply::Success)
    }

    fn close_open_uri(&mut self, _id: RequestId, _reason: enzui_iced_core::portal::CancelReason) {}

    fn describe_open_uri(&self) -> String {
        "recording recovery OpenURI backend".into()
    }
}

fn resolver(root: &Path) -> XdgResolver {
    XdgResolver::new(
        None::<PathBuf>,
        Some(root.to_path_buf()),
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

fn collect_nodes(app: &App) -> Vec<CollectedNode> {
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        App::view(app),
        iced::Size::new(1200.0, 800.0),
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let operation: &mut dyn Operation = &mut collector;
        operation
    });
    collector.into_collected()
}

fn has_label(nodes: &[CollectedNode], label: &str) -> bool {
    nodes
        .iter()
        .any(|node| node.semantics.label.as_deref() == Some(label))
}

fn has_label_containing(nodes: &[CollectedNode], text: &str) -> bool {
    nodes.iter().any(|node| {
        node.semantics
            .label
            .as_deref()
            .is_some_and(|label| label.contains(text))
    })
}

fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn install_library_load(app: &mut App, resolver: &XdgResolver) {
    let (store, outcome) = persistence::load_state_authority(resolver);
    let (state, recovery) = outcome.into_parts();
    app.library_store = store;
    app.model.app_state = std::sync::Arc::new(state);
    app.model.recovery_authorities = recovery.into_iter().collect();
}

#[test]
fn state_recovery_requires_explicit_action_and_respects_newer_versions() {
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = resolver(fixture.path());
        let state_path = persistence::state_path_with_resolver(&resolver);
        let corrupt = b"{truncated library";
        drop(persistence::app_state_store_with_resolver(&resolver));
        std::fs::write(&state_path, corrupt).unwrap();
        make_private(&state_path);

        let (mut app, _) = App::test_new();
        app.enable_persist(resolver.clone());
        install_library_load(&mut app, &resolver);
        let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        app.recovery_location_opener =
            crate::recovery_location::RecoveryLocationOpener::with_backend(Box::new(
                RecordingOpenUriBackend {
                    opened: opened.clone(),
                },
            ));
        app.progress_dirty = true;
        drop(app.update(Msg::FlushPersistence));
        let _ = app.drain_persistence_debt_for_test();
        assert_eq!(std::fs::read(&state_path).unwrap(), corrupt);
        assert!(app.persistence_debt.is_empty());

        let nodes = collect_nodes(&app);
        assert!(has_label_containing(&nodes, "Library state needs recovery"));
        assert!(has_label(&nodes, "Retry"));
        assert!(has_label(&nodes, "Open recovery location"));
        assert!(has_label(&nodes, "Start empty"));

        drop(app.update(Msg::RevealRecoveryLocation(StateOrigin::Library)));
        let (_, recovery_path) = app
            .model
            .revealed_recovery_location
            .as_ref()
            .expect("the action exposes a recovery location");
        assert!(recovery_path.exists());
        let recovery_label = recovery_path.display().to_string();
        let opened_uri = OpenUri::parse(opened.lock().unwrap()[0].clone()).unwrap();
        assert_eq!(
            opened_uri.local_file_path(),
            recovery_path.parent().map(Path::to_path_buf)
        );
        assert!(
            collect_nodes(&app)
                .iter()
                .any(|node| node.semantics.label.as_deref() == Some(recovery_label.as_str()))
        );
        for message in drain_task(app.update(Msg::RecoveryLocationTick)) {
            drop(app.update(message));
        }
        assert_eq!(
            crate::live_feedback::announced(&app),
            Some("Opened recovery location.".to_string())
        );

        drop(app.update(Msg::StartEmptyState(StateOrigin::Library)));
        let _ = app.drain_persistence_debt_for_test();
        assert!(app.model.recovery_authorities.is_empty());
        let AuthorityLoad::Loaded(state) = persistence::load_state_authority(&resolver).1 else {
            panic!("explicit retirement must publish valid empty state");
        };
        assert!(state.files.is_empty());
        assert_ne!(std::fs::read(state_path).unwrap(), corrupt);
    }
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = resolver(fixture.path());
        let state_path = persistence::state_path_with_resolver(&resolver);
        drop(persistence::app_state_store_with_resolver(&resolver));
        let future = br#"{"version":2,"files":{}}"#;
        std::fs::write(&state_path, future).unwrap();
        make_private(&state_path);

        let (mut app, _) = App::test_new();
        app.enable_persist(resolver.clone());
        install_library_load(&mut app, &resolver);
        assert_eq!(
            app.model.recovery_authorities[0].failure,
            StateFailureKind::FutureVersion
        );
        assert_eq!(
            app.model.recovery_authorities[0].quarantine,
            QuarantineStatus::OriginalRetained
        );
        let nodes = collect_nodes(&app);
        assert!(has_label_containing(&nodes, "newer Reader version"));
        assert!(has_label(&nodes, "Retry"));
        assert!(has_label(&nodes, "Open recovery location"));
        assert!(!has_label(&nodes, "Start empty"));

        drop(app.update(Msg::StartEmptyState(StateOrigin::Library)));
        assert_eq!(std::fs::read(&state_path).unwrap(), future);
        assert_eq!(app.model.recovery_authorities.len(), 1);

        let superseded_generation = app.library_recovery_generation;
        drop(app.update(Msg::RetryStateRecovery(StateOrigin::Library)));
        let repaired = br#"{"version":1,"files":{"/fixed.pdf":{"page":4,"total":8,"updated":1,"zoom":1.0,"layout":"single","direction":"ltr","font_size":18.0,"font_family":"Sans","line_height":1.5,"margin":32.0,"theme":"system","title":"Fixed","author":"","format":"pdf","epub_view_mode":null,"epub_anchor":null,"reading_time_secs":null}}}"#;
        std::fs::write(&state_path, repaired).unwrap();
        let stale_reload = persistence::reload_recovery(StateOrigin::Library, &resolver, None);
        let base_revision = app.library_state_revision;
        drop(app.update(Msg::StateRecoveryReloaded {
            generation: superseded_generation,
            base_revision,
            origin: StateOrigin::Library,
            settlement: persistence::BlockingLoadSettlement::Applied(stale_reload),
        }));
        assert_eq!(app.model.recovery_authorities.len(), 1);

        let reload = persistence::reload_recovery(StateOrigin::Library, &resolver, None);
        let generation = app.library_recovery_generation;
        app.annotation_recovery_generation = app.annotation_recovery_generation.wrapping_add(1);
        let base_revision = app.library_state_revision;
        drop(app.update(Msg::StateRecoveryReloaded {
            generation,
            base_revision,
            origin: StateOrigin::Library,
            settlement: persistence::BlockingLoadSettlement::Applied(reload),
        }));
        assert!(app.model.recovery_authorities.is_empty());
        assert_eq!(app.model.app_state.files["/fixed.pdf"].title, "Fixed");
        assert_eq!(std::fs::read(state_path).unwrap(), repaired);
    }
}

#[test]
fn corrupt_sidecar_recovery_keeps_native_pdf_annotation_identity_independent() {
    let fixture = tempfile::tempdir().unwrap();
    let resolver = resolver(fixture.path());
    let annotations_root = fixture.path().join("annotations");
    let document = fixture.path().join("annotated.pdf");
    std::fs::write(&document, b"%PDF-1.4\n").unwrap();
    let source_path = document.to_string_lossy().into_owned();
    let sidecar =
        persistence::annotations_path_with_roots(&source_path, &resolver, Some(&annotations_root));
    let corrupt = b"{truncated sidecar";
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    std::fs::write(&sidecar, corrupt).unwrap();
    make_private(&sidecar);

    let (mut app, _) = App::test_new();
    app.enable_persist(resolver);
    app.annotations_root = Some(annotations_root);
    app.model.file_path = Some(document);
    app.model.format = Format::Pdf;
    app.load_annotations_for_document(Path::new(&source_path));
    assert!(matches!(
        &app.model.recovery_authorities[0].origin,
        StateOrigin::Annotations { .. }
    ));
    let recovery_copy = app.model.recovery_authorities[0]
        .recovery_path()
        .expect("corrupt sidecar bytes are copied")
        .to_path_buf();

    app.reconcile_native_pdf_annotations(PdfAnnotationSnapshot {
        document_fingerprint: "document-fingerprint".into(),
        pages: Vec::new(),
        annotations: vec![StoredAnnotation {
            page: 0,
            native_id: "native-note-7".into(),
            subtype: "Text".into(),
            contents: "Native note survives".into(),
            color: String::new(),
            rect: None,
            has_quad_points: false,
            fingerprint: "native-fingerprint".into(),
        }],
    });
    assert_eq!(app.model.annotations.annotations.len(), 1);
    assert_eq!(
        app.model.annotations.annotations[0]
            .pdf_binding
            .as_ref()
            .unwrap()
            .native_id,
        "native-note-7"
    );
    drop(app.update(Msg::FlushPersistence));
    let _ = app.drain_persistence_debt_for_test();
    assert_eq!(std::fs::read(&sidecar).unwrap(), corrupt);

    drop(app.update(Msg::StartEmptyState(StateOrigin::Annotations {
        source_path: source_path.clone(),
    })));
    let _ = app.drain_persistence_debt_for_test();
    let reloaded = persistence::load_annotations_with_roots(
        &source_path,
        &app.xdg_resolver,
        app.annotations_root.as_deref(),
    );
    assert_eq!(reloaded.annotations.len(), 1);
    assert_eq!(
        reloaded.annotations[0]
            .pdf_binding
            .as_ref()
            .unwrap()
            .native_id,
        "native-note-7"
    );
    assert_eq!(std::fs::read(recovery_copy).unwrap(), corrupt);
    assert_eq!(std::fs::read(&source_path).unwrap(), b"%PDF-1.4\n");
}
