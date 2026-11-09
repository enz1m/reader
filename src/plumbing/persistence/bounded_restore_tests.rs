use super::annotations_io::load_annotations_authority_with_data;
use super::backend::RawJsonBackend;
use super::reading::{
    RawJsonRead, arm_after_json_read_hook, arm_after_read_metadata_hook,
    arm_after_retained_read_hook, read_raw_json,
};
use super::state::authority_outcome;
use super::*;
use enzui_iced_core::persist::Persistable;

fn path(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn state_admission_bounds_bytes_and_refuses_special_files() {
    {
        let fixture = tempfile::tempdir().unwrap();
        let exact = path(fixture.path(), "exact.json");
        fs::write(
            &exact,
            vec![b'x'; usize::try_from(bounded_json::MAX_LIBRARY_JSON_BYTES).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            read_raw_json(&exact, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Body(bytes, _)
                if u64::try_from(bytes.len()).unwrap() == bounded_json::MAX_LIBRARY_JSON_BYTES
        ));

        let plus_one = path(fixture.path(), "plus-one.json");
        fs::write(
            &plus_one,
            vec![b'x'; usize::try_from(bounded_json::MAX_LIBRARY_JSON_BYTES + 1).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            read_raw_json(&plus_one, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::TooLarge, _, _)
        ));
        let store: AppStateStore = Arc::new(JsonStore::new(
            RawJsonBackend::new(plus_one.clone()),
            "reader-library-json",
        ));
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(fixture.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        assert!(matches!(
            authority_outcome(
                store.load(),
                StateOrigin::Library,
                plus_one.clone(),
                &resolver,
                None,
            ),
            AuthorityLoad::Recovery { .. }
        ));
        assert!(store.is_write_blocked());
        assert_eq!(
            fs::metadata(&plus_one).unwrap().len(),
            bounded_json::MAX_LIBRARY_JSON_BYTES + 1
        );

        let growing = path(fixture.path(), "growing.json");
        fs::write(&growing, b"x").unwrap();
        let replacement = growing.clone();
        arm_after_read_metadata_hook(move || {
            fs::write(
                replacement,
                vec![b'x'; usize::try_from(bounded_json::MAX_LIBRARY_JSON_BYTES + 1).unwrap()],
            )
            .unwrap();
        });
        assert!(matches!(
            read_raw_json(&growing, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::TooLarge, _, _)
        ));

        let under_cap_growth = path(fixture.path(), "under-cap-growth.json");
        fs::write(&under_cap_growth, b"x").unwrap();
        let append_path = under_cap_growth.clone();
        arm_after_read_metadata_hook(move || {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(append_path)
                .unwrap();
            file.write_all(b"y").unwrap();
        });
        assert!(matches!(
            read_raw_json(&under_cap_growth, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::ReadFailed, _, _)
        ));
        assert_eq!(fs::read(&under_cap_growth).unwrap(), b"xy");

        let shrinking = path(fixture.path(), "shrinking.json");
        fs::write(&shrinking, b"xy").unwrap();
        let shrink_path = shrinking.clone();
        arm_after_read_metadata_hook(move || {
            fs::OpenOptions::new()
                .write(true)
                .open(shrink_path)
                .unwrap()
                .set_len(1)
                .unwrap();
        });
        assert!(matches!(
            read_raw_json(&shrinking, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::ReadFailed, _, _)
        ));
        assert_eq!(fs::read(&shrinking).unwrap(), b"x");

        let after_read_growth = path(fixture.path(), "after-read-growth.json");
        fs::write(&after_read_growth, b"x").unwrap();
        let append_path = after_read_growth.clone();
        arm_after_json_read_hook(move || {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(append_path)
                .unwrap();
            file.write_all(b"y").unwrap();
        });
        assert!(matches!(
            read_raw_json(&after_read_growth, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::ReadFailed, _, _)
        ));

        let replaced = path(fixture.path(), "replaced.json");
        fs::write(&replaced, b"original").unwrap();
        let replacement_path = replaced.clone();
        arm_after_json_read_hook(move || {
            fs::remove_file(&replacement_path).unwrap();
            fs::write(replacement_path, b"foreign!").unwrap();
        });
        assert!(matches!(
            read_raw_json(&replaced, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::OpenRefused, _, _)
        ));
        assert_eq!(fs::read(replaced).unwrap(), b"foreign!");
    }
    {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let directory = path(fixture.path(), "directory");
        fs::create_dir(&directory).unwrap();
        assert!(matches!(
            read_raw_json(&directory, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::NotRegularFile, _, _)
        ));

        let target = path(fixture.path(), "target");
        let link = path(fixture.path(), "link");
        fs::write(&target, b"{}").unwrap();
        symlink(&target, &link).unwrap();
        assert!(matches!(
            read_raw_json(&link, bounded_json::MAX_LIBRARY_JSON_BYTES),
            RawJsonRead::Refused(StateRefusal::OpenRefused, _, _)
        ));

        let fifo = path(fixture.path(), "fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(status.success());
        let (sent, received) = std::sync::mpsc::channel();
        let reader_path = fifo.clone();
        let mut reader = Some(std::thread::spawn(move || {
            let result = read_raw_json(&reader_path, bounded_json::MAX_LIBRARY_JSON_BYTES);
            let _ = sent.send(result);
        }));
        let result = match received.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let panic = reader.take().unwrap().join().expect_err("reader panicked");
                std::panic::resume_unwind(panic);
            }
            Err(error @ std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let release_path = fifo.clone();
                let writer = std::thread::spawn(move || {
                    fs::OpenOptions::new().write(true).open(release_path)
                });
                let _ = reader.take().unwrap().join();
                let _ = writer.join();
                panic!("FIFO admission did not settle before the deadline: {error}");
            }
        };
        reader.take().unwrap().join().unwrap();
        assert!(matches!(
            result,
            RawJsonRead::Refused(StateRefusal::NotRegularFile, _, _)
        ));
    }
}

#[test]
fn typed_restore_bounds_collections_and_retains_the_admitted_read() {
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(fixture.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let state_path = state_path_with_resolver(&resolver);
        drop(app_state_store_with_resolver(&resolver));
        let valid =
            r#"{"version":1,"files":{"/book.pdf":{"page":1,"title":"Book","format":"pdf"}}}"#;
        fs::write(&state_path, valid).unwrap();
        make_private(&state_path);
        let before = deserialization_count_on_current_thread();
        let (_, loaded) = load_state_authority(&resolver);
        assert!(matches!(loaded, AuthorityLoad::Loaded(state) if state.files.len() == 1));
        assert_eq!(deserialization_count_on_current_thread() - before, 1);

        let mut library = String::from("{\"version\":1,\"files\":{");
        for index in 0..=bounded_json::MAX_LIBRARY_RECORDS {
            if index != 0 {
                library.push(',');
            }
            library.push_str(&format!("\"/{index}\":{{\"page\":0}}"));
        }
        library.push_str("}}");
        assert!(AppState::persist_deserialize(&library).is_err());
        fs::write(&state_path, library.as_bytes()).unwrap();
        let (store, outcome) = load_state_authority(&resolver);
        assert!(matches!(outcome, AuthorityLoad::Recovery { .. }));
        assert!(store.is_write_blocked());
        assert_eq!(fs::read(&state_path).unwrap(), library.as_bytes());

        let annotation = r#"{"id":1,"page":0,"kind":"note","text":"x"}"#;
        let annotations = format!(
            "{{\"version\":1,\"annotations\":[{}],\"bookmarks\":[]}}",
            std::iter::repeat_n(annotation, bounded_json::MAX_ANNOTATIONS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(AnnotationsFile::persist_deserialize(&annotations).is_err());

        let bookmark = r#"{"page":0}"#;
        let bookmarks = format!(
            "{{\"version\":1,\"annotations\":[],\"bookmarks\":[{}]}}",
            std::iter::repeat_n(bookmark, bounded_json::MAX_BOOKMARKS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(AnnotationsFile::persist_deserialize(&bookmarks).is_err());
    }
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(fixture.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let state_path = state_path_with_resolver(&resolver);
        drop(app_state_store_with_resolver(&resolver));
        let corrupt = b"{retained corrupt bytes";
        fs::write(&state_path, corrupt).unwrap();
        make_private(&state_path);
        let replaced = state_path.clone();
        arm_after_retained_read_hook(move || {
            fs::remove_file(&replaced).unwrap();
            fs::write(replaced, b"foreign replacement").unwrap();
        });

        let (store, outcome) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery { authority, .. } = outcome else {
            panic!("the retained corrupt body must require recovery");
        };
        let QuarantineStatus::Preserved { path, .. } = authority.quarantine else {
            panic!("the retained read must be preserved without reopening its path");
        };
        assert_eq!(fs::read(path).unwrap(), corrupt);
        assert_eq!(fs::read(&state_path).unwrap(), b"foreign replacement");
        assert!(store.is_write_blocked());
    }
}

#[test]
fn retained_and_serialized_state_limits_preserve_last_good_bytes() {
    {
        let long_path = "p".repeat(bounded_json::MAX_PATH_BYTES + 1);
        let state = format!("{{\"version\":1,\"files\":{{\"{long_path}\":{{\"page\":0}}}}}}");
        assert!(AppState::persist_deserialize(&state).is_err());

        let long_field = "f".repeat(bounded_json::MAX_FIELD_NAME_BYTES + 1);
        let state = format!("{{\"version\":1,\"files\":{{}},\"{long_field}\":null}}");
        assert!(AppState::persist_deserialize(&state).is_err());

        let long_text = "t".repeat(bounded_json::MAX_TEXT_BYTES + 1);
        let sidecar = format!(
            "{{\"version\":1,\"annotations\":[{{\"id\":1,\"page\":0,\"kind\":\"note\",\"text\":\"{long_text}\"}}]}}"
        );
        assert!(AnnotationsFile::persist_deserialize(&sidecar).is_err());

        let long_label = "b".repeat(bounded_json::MAX_TEXT_BYTES + 1);
        let bookmark = format!(
            "{{\"version\":1,\"annotations\":[],\"bookmarks\":[{{\"page\":0,\"label\":\"{long_label}\"}}]}}"
        );
        assert!(AnnotationsFile::persist_deserialize(&bookmark).is_err());

        let chunk = "a".repeat(bounded_json::MAX_TEXT_BYTES);
        let records = bounded_json::MAX_LIBRARY_RETAINED_BYTES / bounded_json::MAX_TEXT_BYTES + 1;
        let mut aggregate = String::from("{\"version\":1,\"files\":{");
        for index in 0..records {
            if index != 0 {
                aggregate.push(',');
            }
            aggregate.push_str(&format!(
                "\"/{index}\":{{\"page\":0,\"title\":\"{chunk}\"}}"
            ));
        }
        aggregate.push_str("}}");
        assert!(AppState::persist_deserialize(&aggregate).is_err());
    }
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(fixture.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let mut state = empty_app_state();
        state.files.insert(
            "/last-good.pdf".into(),
            ProgressRecord {
                page: 1,
                ..ProgressRecord::default()
            },
        );
        save_state_with_resolver(&state, &resolver).unwrap();
        let state_path = state_path_with_resolver(&resolver);
        let state_bytes = fs::read(&state_path).unwrap();
        let (state_store, _) = load_state_authority(&resolver);
        for index in 0..=bounded_json::MAX_LIBRARY_RECORDS {
            state.files.insert(
                format!("/too-many-{index}"),
                ProgressRecord {
                    page: index,
                    ..ProgressRecord::default()
                },
            );
        }
        assert!(state_store.save_blocking(&state).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), state_bytes);

        let mut aggregate = empty_app_state();
        let chunk = "a".repeat(bounded_json::MAX_TEXT_BYTES);
        let records = bounded_json::MAX_LIBRARY_RETAINED_BYTES / bounded_json::MAX_TEXT_BYTES + 1;
        for index in 0..records {
            aggregate.files.insert(
                format!("/aggregate-{index}"),
                ProgressRecord {
                    page: index,
                    title: chunk.clone(),
                    ..ProgressRecord::default()
                },
            );
        }
        assert!(state_store.save_blocking(&aggregate).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), state_bytes);

        let oversized_frame =
            "x".repeat(usize::try_from(bounded_json::MAX_LIBRARY_JSON_BYTES + 1).unwrap());
        assert!(write_serialized_state_with_resolver(&oversized_frame, &resolver).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), state_bytes);

        let source = fixture.path().join("book.pdf");
        fs::write(&source, b"book").unwrap();
        let source = source.to_string_lossy().into_owned();
        let annotations_root = fixture.path().join("annotations");
        let valid = empty_annotations(&source);
        save_annotations_with_roots(&source, &valid, &resolver, Some(&annotations_root)).unwrap();
        let sidecar = annotations_path_with_roots(&source, &resolver, Some(&annotations_root));
        let sidecar_bytes = fs::read(&sidecar).unwrap();
        let (sidecar_store, _) =
            load_annotations_authority_with_roots(&source, &resolver, Some(&annotations_root));
        let mut oversized = valid;
        oversized.annotations.push(Annotation {
            id: 1,
            page: 0,
            kind: "note".into(),
            text: "x".repeat(bounded_json::MAX_TEXT_BYTES + 1),
            color: String::new(),
            created: String::new(),
            position: String::new(),
            style: String::new(),
            note: String::new(),
            pdf_binding: None,
            quads: Vec::new(),
        });
        assert!(sidecar_store.save_blocking(&oversized).is_err());
        assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    }
}

#[test]
fn private_stores_reject_planted_roots_and_rotate_authority() {
    {
        let unavailable = XdgResolver::default();
        for resolver in [
            unavailable,
            XdgResolver::new(
                None::<PathBuf>,
                Some(PathBuf::from("relative-state")),
                None::<PathBuf>,
                Some(PathBuf::from("relative-home")),
            ),
        ] {
            let (store, outcome) = load_state_authority(&resolver);
            assert!(matches!(outcome, AuthorityLoad::Unavailable(_)));
            assert!(store.is_write_blocked());
        }

        let fixture = tempfile::tempdir().unwrap();
        let state_base = fixture.path().join("state");
        fs::create_dir(&state_base).unwrap();
        let app_root = state_base.join("reader-app");
        fs::write(&app_root, b"foreign root").unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(state_base.clone()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let (store, outcome) = load_state_authority(&resolver);
        assert!(matches!(outcome, AuthorityLoad::Unavailable(_)));
        assert!(store.is_write_blocked());
        assert_eq!(fs::read(&app_root).unwrap(), b"foreign root");

        fs::remove_file(&app_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};

            let target = fixture.path().join("foreign-directory");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("state.json"), b"foreign state").unwrap();
            symlink(&target, &app_root).unwrap();
            let (store, outcome) = load_state_authority(&resolver);
            assert!(matches!(outcome, AuthorityLoad::Unavailable(_)));
            assert!(store.is_write_blocked());
            assert_eq!(
                fs::read(target.join("state.json")).unwrap(),
                b"foreign state"
            );
            fs::remove_file(&app_root).unwrap();

            fs::create_dir(&app_root).unwrap();
            fs::set_permissions(&app_root, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(app_root.join("state.json"), b"exposed state").unwrap();
            // An own folder left at 0o755 is made private and adopted, not refused.
            let (store, outcome) = load_state_authority(&resolver);
            // The garbage it holds is offered for recovery, so saves wait.
            assert!(matches!(outcome, AuthorityLoad::Recovery { .. }));
            assert!(store.is_write_blocked());
            assert_eq!(
                fs::read(app_root.join("state.json")).unwrap(),
                b"exposed state"
            );
            assert_eq!(
                fs::metadata(&app_root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        let source_root = fixture.path().join("source-repository");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(source_root.join(".git")).unwrap();
        let source = source_root.join("book.pdf");
        fs::write(&source, b"book").unwrap();
        let source = source.to_string_lossy().into_owned();
        for data_resolver in [
            AppDataResolver::default(),
            AppDataResolver::new(Some(PathBuf::from("relative-data")), None::<PathBuf>),
        ] {
            let (store, outcome) =
                load_annotations_authority_with_data(&source, &resolver, &data_resolver, None);
            assert!(matches!(outcome, AuthorityLoad::Unavailable(_)));
            assert!(store.is_write_blocked());
            assert!(!source_root.join("book.pdf.annotations.json").exists());
        }

        let (store, outcome) = load_annotations_authority_with_data(
            &source,
            &resolver,
            &AppDataResolver::default(),
            Some(&app_root),
        );
        // The state load above made that root private, so it now holds
        // annotations too.
        assert!(!matches!(outcome, AuthorityLoad::Unavailable(_)));
        assert!(!store.is_write_blocked());
        assert_eq!(
            fs::read(app_root.join("state.json")).unwrap(),
            b"exposed state"
        );

        let data_base = fixture.path().join("data");
        let data_resolver = AppDataResolver::new(Some(data_base.clone()), None::<PathBuf>);
        let (store, outcome) =
            load_annotations_authority_with_data(&source, &resolver, &data_resolver, None);
        let AuthorityLoad::Missing(file) = outcome else {
            panic!("private data fallback must admit an absent sidecar");
        };
        store.save_blocking(&file).unwrap();
        let (_, reloaded) =
            load_annotations_authority_with_data(&source, &resolver, &data_resolver, None);
        assert!(matches!(reloaded, AuthorityLoad::Loaded(_)));
        let app_root = data_base.join("reader-app");
        assert!(app_root.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(app_root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
    {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = XdgResolver::new(
            None::<PathBuf>,
            Some(fixture.path().to_path_buf()),
            None::<PathBuf>,
            None::<PathBuf>,
        );
        let (state_store, missing) = load_state_authority(&resolver);
        assert!(matches!(missing, AuthorityLoad::Missing(_)));
        let mut first = empty_app_state();
        first.files.insert(
            "/first.pdf".into(),
            ProgressRecord {
                page: 1,
                ..ProgressRecord::default()
            },
        );
        state_store.save_blocking(&first).unwrap();
        let mut second = first.clone();
        second
            .files
            .get_mut("/first.pdf")
            .expect("the first record was inserted")
            .page = 2;
        state_store.save_blocking(&second).unwrap();
        let state_path = state_path_with_resolver(&resolver);
        assert_eq!(
            load_state_with_resolver(&resolver).files["/first.pdf"].page,
            2
        );

        fs::remove_file(&state_path).unwrap();
        fs::write(&state_path, b"foreign state").unwrap();
        assert!(state_store.save_blocking(&first).is_err());
        assert!(state_store.save_blocking(&second).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), b"foreign state");

        let source = fixture.path().join("book.pdf");
        fs::write(&source, b"book").unwrap();
        let source = source.to_string_lossy().into_owned();
        let annotations_root = fixture.path().join("annotations");
        let (sidecar_store, missing) =
            load_annotations_authority_with_roots(&source, &resolver, Some(&annotations_root));
        assert!(matches!(missing, AuthorityLoad::Missing(_)));
        let first = empty_annotations(&source);
        sidecar_store.save_blocking(&first).unwrap();
        let mut second = first.clone();
        second.bookmarks.push(BookmarkRecord {
            page: 3,
            label: "Page 4".into(),
            created: String::new(),
        });
        sidecar_store.save_blocking(&second).unwrap();
        let sidecar = annotations_path_with_roots(&source, &resolver, Some(&annotations_root));
        assert_eq!(
            load_annotations_with_roots(&source, &resolver, Some(&annotations_root))
                .bookmarks
                .len(),
            1
        );

        fs::remove_file(&sidecar).unwrap();
        fs::write(&sidecar, b"foreign annotations").unwrap();
        assert!(sidecar_store.save_blocking(&first).is_err());
        assert!(sidecar_store.save_blocking(&second).is_err());
        assert_eq!(fs::read(sidecar).unwrap(), b"foreign annotations");
    }
}
