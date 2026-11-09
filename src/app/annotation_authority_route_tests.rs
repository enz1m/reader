use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const FOREIGN_NAME: &str = ".enzui-reader-probe";
const FOREIGN_BYTES: &[u8] = b"an unrelated sentinel nobody asked the reader to touch\n";
const SUBSTITUTE_BYTES: &[u8] = b"a foreign inode that took the staged name\n";

const DEADLINE: Duration = Duration::from_secs(5);

fn scratch(label: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-app-3019-route-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn copied_fixture(dir: &Path) -> PathBuf {
    let copy = dir.join("opened.pdf");
    std::fs::copy(
        crate::plumbing::persistence::fixtures_dir().join("test-multipage.pdf"),
        &copy,
    )
    .expect("the multipage PDF fixture must copy into the sandbox");
    copy
}

// Either override would short-circuit the resolver before the writability
// decision this route exercises.
fn booted(root: &Path) -> App {
    let (mut app, _task) = App::test_new();
    let _ = app.update(Msg::WindowResized(iced::Size::new(1440.0, 900.0)));
    app.enable_persist_with_data(
        crate::app::test_xdg_resolver(&root.join("state")),
        AppDataResolver::new(Some(root.join("data")), None::<PathBuf>),
    );
    assert!(app.annotations_root.is_none());
    assert!(
        crate::plumbing::persistence::thread_annotations_root().is_none(),
        "the capture seam must be inactive for a release-shaped route"
    );
    app
}

// The reducer only starts the load; `finish_pages`, which resolves the
// annotation authority, runs from the tasks fed back here.
fn open(app: &mut App, path: &Path) {
    let mut pending = vec![Msg::OpenPath(path.to_path_buf())];
    for _ in 0..8 {
        if pending.is_empty() {
            return;
        }
        let mut next = Vec::new();
        for message in pending.drain(..) {
            next.extend(super::drain_task(app.update(message)));
        }
        pending = next;
    }
    panic!("opening the document never settled");
}

fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn identity(path: &Path) -> (u64, u64) {
    let meta = std::fs::symlink_metadata(path).unwrap();
    (meta.dev(), meta.ino())
}

fn plant_foreign_file(dir: &Path) -> (PathBuf, (u64, u64)) {
    let foreign = dir.join(FOREIGN_NAME);
    std::fs::write(&foreign, FOREIGN_BYTES).unwrap();
    let id = identity(&foreign);
    (foreign, id)
}

fn flush(app: &mut App) {
    let _ = app.update(Msg::FlushPersistence);
    let _ = app.drain_persistence_debt_for_test();
}

fn data_fallback_sidecar(root: &Path) -> Option<PathBuf> {
    let found = data_fallback_sidecars(root);
    assert!(
        found.len() <= 1,
        "one document must not produce several fallback sidecars: {found:?}"
    );
    found.into_iter().next()
}

fn data_fallback_sidecars(root: &Path) -> Vec<PathBuf> {
    let base = root.join("data").join("reader-app");
    std::fs::read_dir(&base)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("annotations-"))
                })
                .collect()
        })
        .unwrap_or_default()
}

// The substituting thread runs while the open holds the staged descriptor, so
// cleanup decides about a name it no longer owns.
#[test]
fn an_entry_substituted_during_the_open_probe_cleanup_survives_and_diverts_to_the_fallback() {
    let docs = scratch("substitute-docs");
    let state = scratch("substitute-state");
    let document = copied_fixture(&docs);

    let (staged_tx, staged_rx): (Sender<PathBuf>, Receiver<PathBuf>) = channel();
    let (replaced_tx, replaced_rx): (Sender<PathBuf>, Receiver<PathBuf>) = channel();
    let substituter = std::thread::spawn(move || {
        let mut replaced = Vec::new();
        while let Ok(staged) = staged_rx.recv_timeout(DEADLINE) {
            std::fs::remove_file(&staged).expect("the staged probe entry is removable");
            std::fs::write(&staged, SUBSTITUTE_BYTES)
                .expect("a foreign file takes the staged name");
            let substituted = identity(&staged);
            replaced.push((staged.clone(), substituted));
            if replaced_tx.send(staged).is_err() {
                break;
            }
        }
        replaced
    });

    let mut app = booted(&state);
    app.observe_annotation_probe(move |staged| {
        staged_tx
            .send(staged.to_path_buf())
            .expect("the substituting thread is listening");
        let echoed = replaced_rx
            .recv_timeout(DEADLINE)
            .expect("the substituting thread must replace the staged entry before the deadline");
        assert_eq!(echoed, staged);
    });
    open(&mut app, &document);
    app.clear_annotation_probe_observer();

    let replaced = substituter
        .join()
        .expect("the substituting thread panicked");
    assert_eq!(app.model.screen, Screen::Reading, "the fixture must open");
    assert_eq!(replaced.len(), 1, "one open, one probe, one substitution");
    let (substitute, substituted_identity) = &replaced[0];
    assert_eq!(
        std::fs::read(substitute).unwrap(),
        SUBSTITUTE_BYTES,
        "cleanup removed {substitute:?} after it stopped naming the staged inode"
    );
    assert_eq!(identity(substitute), *substituted_identity);

    let _ = app.seed_highlight_for_test(0, "substituted route", "yellow");
    flush(&mut app);
    let fallback = data_fallback_sidecar(&state)
        .expect("an uncertain probe must send the edit to the private data fallback");
    assert_eq!(
        app.annotation_authority_path(),
        Some(fallback.as_path()),
        "an uncertain probe must not claim sidecar authority"
    );
    assert!(
        std::fs::read_to_string(&fallback)
            .unwrap()
            .contains("substituted route")
    );
    assert!(
        !docs.join("opened.pdf.annotations.json").exists(),
        "no sidecar may appear beside the document"
    );

    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_dir_all(&state);
}

// The directory turns writable between the two Apps, so a reopen that re-probes
// would choose a different destination than the first open did.
#[test]
fn a_document_annotated_in_a_read_only_directory_recovers_it_after_a_restart() {
    let docs = scratch("readonly-docs");
    let state = scratch("readonly-state");
    let document = copied_fixture(&docs);
    let (foreign, foreign_id) = plant_foreign_file(&docs);

    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o500)).unwrap();
    let control = std::fs::write(docs.join("control"), b"x");
    assert!(
        control.is_err(),
        "the fixture cannot distinguish anything: {docs:?} still accepts a write"
    );

    let fallback = {
        let mut app = booted(&state);
        open(&mut app, &document);
        assert_eq!(app.model.screen, Screen::Reading, "the fixture must open");
        let chosen = app
            .annotation_authority_path()
            .expect("the open document must hold an annotation authority")
            .to_path_buf();
        let _ = app.seed_highlight_for_test(0, "read-only route", "yellow");
        flush(&mut app);
        let fallback =
            data_fallback_sidecar(&state).expect("the private data fallback must hold the edit");
        assert_eq!(
            chosen, fallback,
            "the edit must land on the authority the open reported"
        );
        assert!(
            std::fs::read_to_string(&fallback)
                .unwrap()
                .contains("read-only route")
        );
        fallback
    };

    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut restarted = booted(&state);
    open(&mut restarted, &document);
    assert_eq!(
        restarted.annotation_authority_path(),
        Some(fallback.as_path()),
        "a restarted Reader must adopt the durable record, not an empty sidecar"
    );
    let recovered: Vec<String> = restarted
        .model
        .annotations
        .annotations
        .iter()
        .map(|annotation| annotation.text.clone())
        .collect();
    assert_eq!(
        recovered,
        vec!["read-only route".to_string()],
        "the restarted Reader must recover the annotation from the durable record"
    );

    let _ = restarted.seed_highlight_for_test(1, "second read-only route", "green");
    flush(&mut restarted);
    let body = std::fs::read_to_string(&fallback).unwrap();
    assert!(
        body.contains("read-only route") && body.contains("second read-only route"),
        "the restarted edit must join the same durable record at {fallback:?}"
    );
    assert_eq!(
        entry_names(&docs),
        vec![FOREIGN_NAME.to_string(), "opened.pdf".to_string()],
        "a refused directory must gain neither a sidecar nor probe residue"
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), FOREIGN_BYTES);
    assert_eq!(identity(&foreign), foreign_id);

    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn opening_a_document_with_two_records_retains_custody_and_writes_neither() {
    let docs = scratch("split-docs");
    let state = scratch("split-state");
    let document = copied_fixture(&docs);

    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o500)).unwrap();
    let fallback = {
        let mut app = booted(&state);
        open(&mut app, &document);
        let _ = app.seed_highlight_for_test(0, "in the state directory", "yellow");
        flush(&mut app);
        data_fallback_sidecar(&state)
            .expect("the refused parent must use the private data fallback")
    };
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o700)).unwrap();

    let sidecar = docs.join("opened.pdf.annotations.json");
    let mut beside = crate::plumbing::persistence::empty_annotations(&document.to_string_lossy());
    crate::plumbing::persistence::add_annotation(
        &mut beside,
        1,
        "note",
        "beside the document",
        "aqua",
    );
    std::fs::write(
        &sidecar,
        crate::plumbing::persistence::serialize_annotations_pretty(&beside).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o600)).unwrap();
    let sidecar_before = std::fs::read(&sidecar).unwrap();
    let fallback_before = std::fs::read(&fallback).unwrap();

    let mut app = booted(&state);
    open(&mut app, &document);
    assert_eq!(
        app.annotation_authority_path(),
        Some(fallback.as_path()),
        "the state record is read while the other is retained"
    );
    let custody: Vec<crate::plumbing::persistence::StateFailureKind> = app
        .model
        .recovery_authorities
        .iter()
        .filter(|authority| {
            matches!(
                &authority.origin,
                crate::plumbing::persistence::StateOrigin::Annotations { source_path }
                    if crate::plumbing::persistence::canonical_key(source_path)
                        == crate::plumbing::persistence::canonical_key(&document.to_string_lossy())
            )
        })
        .map(|authority| authority.failure)
        .collect();
    assert_eq!(
        custody,
        vec![crate::plumbing::persistence::StateFailureKind::SplitAuthority],
        "two records must retain typed write custody for this document"
    );

    let _ = app.seed_highlight_for_test(2, "must not be written", "green");
    flush(&mut app);
    assert_eq!(
        std::fs::read(&sidecar).unwrap(),
        sidecar_before,
        "custody must leave the record beside the document untouched"
    );
    assert_eq!(
        std::fs::read(&fallback).unwrap(),
        fallback_before,
        "custody must leave the state record untouched"
    );
    assert_eq!(
        entry_names(&docs),
        vec![
            "opened.pdf".to_string(),
            "opened.pdf.annotations.json".to_string()
        ],
        "retaining custody must leave no probe residue in {docs:?}"
    );

    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn opening_a_document_inside_a_checkout_writes_nothing_beside_it() {
    let docs = scratch("checkout-docs");
    let state = scratch("checkout-state");
    let document = copied_fixture(&docs);
    std::fs::create_dir(docs.join(".git")).unwrap();
    let (foreign, foreign_id) = plant_foreign_file(&docs);

    let mut app = booted(&state);
    open(&mut app, &document);
    assert_eq!(app.model.screen, Screen::Reading, "the fixture must open");
    assert!(app.model.page_count > 0, "the real PDF must report pages");

    let _ = app.seed_highlight_for_test(0, "checkout highlight", "yellow");
    flush(&mut app);

    assert_eq!(
        entry_names(&docs),
        vec![
            FOREIGN_NAME.to_string(),
            ".git".to_string(),
            "opened.pdf".to_string()
        ],
        "a document inside a checkout must gain no sidecar and no probe residue"
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), FOREIGN_BYTES);
    assert_eq!(identity(&foreign), foreign_id);

    let fallback = data_fallback_sidecars(&state);
    assert_eq!(
        fallback.len(),
        1,
        "the edit must persist to the private data fallback, found {fallback:?}"
    );
    assert_eq!(
        app.annotation_authority_path(),
        Some(fallback[0].as_path()),
        "a checked-out document must hold the private-data-fallback authority"
    );
    assert!(
        std::fs::read_to_string(&fallback[0])
            .unwrap()
            .contains("checkout highlight"),
        "the private data fallback must hold the first edit"
    );

    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_dir_all(&state);
}
