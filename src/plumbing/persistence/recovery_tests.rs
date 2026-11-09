use super::state::quarantine_fault;
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub(crate) fn new(label: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "reader-recovery-{label}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn resolver(&self) -> XdgResolver {
        XdgResolver::new(
            None::<PathBuf>,
            Some(self.root.clone()),
            None::<PathBuf>,
            None::<PathBuf>,
        )
    }

    fn state_path(&self) -> PathBuf {
        state_path_with_resolver(&self.resolver())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn bookmark(page: usize) -> BookmarkRecord {
    BookmarkRecord {
        page,
        label: format!("Page {}", page + 1),
        created: String::new(),
    }
}

#[test]
fn corrupt_state_retirement_preserves_original_bytes_and_existing_quarantine() {
    {
        let fixture = Fixture::new("truncated");
        let resolver = fixture.resolver();
        let source = fixture.state_path();
        let corrupt = br#"{"version":1,"files":{"/book.pdf":"#;
        write_bytes(&source, corrupt);

        let (store, load) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery { authority, .. } = load else {
            panic!("truncated JSON must require recovery");
        };
        assert_eq!(authority.failure, StateFailureKind::Parse);
        assert!(authority.message().contains("could not parse"));
        let QuarantineStatus::Preserved { path, durable } = &authority.quarantine else {
            panic!("readable corrupt bytes must be quarantined");
        };
        assert!(*durable);
        assert_eq!(fs::read(&source).unwrap(), corrupt);
        assert_eq!(fs::read(path).unwrap(), corrupt);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert!(store.is_write_blocked());
        assert!(store.save_blocking(&empty_app_state()).is_err());
        assert_eq!(fs::read(&source).unwrap(), corrupt);

        let (_second_store, second) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery {
            authority: second_authority,
            ..
        } = second
        else {
            panic!("a second boot must still require recovery");
        };
        assert_eq!(second_authority.recovery_path(), Some(path.as_path()));
        assert_eq!(fs::read(path).unwrap(), corrupt);

        store.authorize_replacement();
        store.save_blocking(&empty_app_state()).unwrap();
        assert_ne!(fs::read(&source).unwrap(), corrupt);
        assert_eq!(fs::read(path).unwrap(), corrupt);
        assert!(matches!(
            load_state_authority(&resolver).1,
            AuthorityLoad::Loaded(_)
        ));
    }
    {
        let fixture = Fixture::new("collision");
        let resolver = fixture.resolver();
        let source = fixture.state_path();
        let corrupt = b"{collision fixture";
        write_bytes(&source, corrupt);

        let recovery = source.parent().unwrap().join("recovery");
        fs::create_dir_all(&recovery).unwrap();
        let hash = corrupt.iter().fold(1469598103934665603u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
        });
        let occupied = recovery.join(format!("library-{hash:016x}.corrupt"));
        write_bytes(&occupied, b"unrelated existing recovery");

        let (_, load) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery { authority, .. } = load else {
            panic!("corrupt bytes must require recovery");
        };
        let QuarantineStatus::Preserved { path, .. } = authority.quarantine else {
            panic!("corrupt bytes must be preserved");
        };
        assert!(path.ends_with(format!("library-{hash:016x}-1.corrupt")));
        assert_eq!(fs::read(occupied).unwrap(), b"unrelated existing recovery");
        assert_eq!(fs::read(path).unwrap(), corrupt);
        assert_eq!(fs::read(source).unwrap(), corrupt);
    }
}

#[test]
fn quarantine_reports_durability_uncertainty_and_keeps_future_state_read_only() {
    {
        let fixture = Fixture::new("parent-sync");
        let resolver = fixture.resolver();
        let source = fixture.state_path();
        let corrupt = b"{parent sync fixture";
        write_bytes(&source, corrupt);
        let _fault = quarantine_fault::arm_parent_sync_failure();

        let (_, load) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery { authority, .. } = load else {
            panic!("corrupt bytes must require recovery");
        };
        let QuarantineStatus::Preserved { path, durable } = authority.quarantine else {
            panic!("the file copy must survive a parent sync failure");
        };
        assert!(!durable);
        assert_eq!(fs::read(path).unwrap(), corrupt);
        assert_eq!(fs::read(source).unwrap(), corrupt);
    }
    {
        let fixture = Fixture::new("future");
        let resolver = fixture.resolver();
        let source = fixture.state_path();
        let future = br#"{"version":4294967295,"files":{"future":{"page":7}}}"#;
        write_bytes(&source, future);

        for _ in 0..2 {
            let (store, load) = load_state_authority(&resolver);
            let AuthorityLoad::Recovery { authority, .. } = load else {
                panic!("a future schema must be refused");
            };
            assert_eq!(authority.failure, StateFailureKind::FutureVersion);
            assert!(authority.message().contains("newer Reader version"));
            assert_eq!(authority.quarantine, QuarantineStatus::OriginalRetained);
            assert!(!authority.can_start_empty());
            assert!(store.is_write_blocked());
            assert!(store.save_blocking(&empty_app_state()).is_err());
            assert_eq!(fs::read(&source).unwrap(), future);
        }
        assert!(!source.parent().unwrap().join("recovery").exists());
    }
}

#[cfg(unix)]
#[test]
fn symlink_state_is_refused_without_copying_or_mutating_its_target() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("symlink");
    let resolver = fixture.resolver();
    let source = fixture.state_path();
    let target = fixture.root.join("outside.json");
    let bytes = b"{outside target";
    write_bytes(&target, bytes);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    // A private app directory, so the load reaches the link: the data root
    // refuses a group/other-readable directory before any state is read.
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(source.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    symlink(&target, &source).unwrap();

    let (store, load) = load_state_authority(&resolver);
    let AuthorityLoad::Recovery { authority, .. } = load else {
        panic!("a symlink must be refused, got {load:?}");
    };
    assert_eq!(authority.failure, StateFailureKind::Read);
    assert!(authority.message().contains("could not safely read"));
    assert_eq!(authority.quarantine, QuarantineStatus::OriginalRetained);
    assert!(store.is_write_blocked());
    assert_eq!(fs::read(target).unwrap(), bytes);
    assert!(!source.parent().unwrap().join("recovery").exists());
}

#[test]
fn sidecar_recovery_is_bound_to_its_document_and_does_not_block_other_stores() {
    {
        let fixture = Fixture::new("sidecar-identity");
        let resolver = fixture.resolver();
        let annotations_root = fixture.root.join("annotation-sidecars");
        let expected_source = "/books/expected.pdf";
        let sidecar =
            annotations_path_with_roots(expected_source, &resolver, Some(&annotations_root));
        let wrong_identity = br#"{"version":1,"source_path":"/books/other.pdf","annotations":[],"next_id":1,"bookmarks":[]}"#;
        write_bytes(&sidecar, wrong_identity);

        let (store, load) = load_annotations_authority_with_roots(
            expected_source,
            &resolver,
            Some(&annotations_root),
        );
        let AuthorityLoad::Recovery { authority, .. } = load else {
            panic!("a mismatched sidecar identity must require recovery");
        };
        assert_eq!(authority.failure, StateFailureKind::Read);
        assert!(store.is_write_blocked());
        store.authorize_replacement();
        assert!(
            store
                .save_blocking(&empty_annotations("/books/other.pdf"))
                .is_err()
        );
        let recovery = authority
            .recovery_path()
            .expect("mismatched bytes must be preserved");
        assert_eq!(fs::read(recovery).unwrap(), wrong_identity);
        assert_eq!(fs::read(sidecar).unwrap(), wrong_identity);
    }
    {
        let fixture = Fixture::new("sidecar-scope");
        let resolver = fixture.resolver();
        let annotations_root = fixture.root.join("annotation-sidecars");
        let first = "/books/first.pdf";
        let second = "/books/second.pdf";
        let first_path = annotations_path_with_roots(first, &resolver, Some(&annotations_root));
        let second_path = annotations_path_with_roots(second, &resolver, Some(&annotations_root));
        let corrupt = b"{first sidecar truncated";
        write_bytes(&first_path, corrupt);
        write_bytes(
        &second_path,
        br#"{"version":1,"source_path":"/books/second.pdf","annotations":[],"next_id":1,"bookmarks":[{"page":3,"label":"Page 4","created":""}]}"#,
    );

        let (first_store, first_load) =
            load_annotations_authority_with_roots(first, &resolver, Some(&annotations_root));
        let AuthorityLoad::Recovery { authority, .. } = first_load else {
            panic!("the first sidecar must require recovery");
        };
        assert_eq!(
            annotation_bookmark_count_with_roots(first, &resolver, Some(&annotations_root)),
            None
        );
        assert_eq!(
            authority.origin,
            StateOrigin::Annotations {
                source_path: first.into()
            }
        );
        assert!(first_store.is_write_blocked());

        let (second_store, second_load) =
            load_annotations_authority_with_roots(second, &resolver, Some(&annotations_root));
        let AuthorityLoad::Loaded(mut second_file) = second_load else {
            panic!("the unaffected sidecar must load");
        };
        assert_eq!(second_file.bookmarks, vec![bookmark(3)]);
        assert_eq!(
            annotation_bookmark_count_with_roots(second, &resolver, Some(&annotations_root)),
            Some(1)
        );
        second_file.bookmarks.push(bookmark(8));
        second_store.save_blocking(&second_file).unwrap();
        assert_eq!(
            load_annotations_with_roots(second, &resolver, Some(&annotations_root)).bookmarks,
            vec![bookmark(3), bookmark(8)]
        );
        assert_eq!(fs::read(first_path).unwrap(), corrupt);
    }
}

#[test]
fn hostile_recovery_locations_and_future_sidecars_remain_write_protected() {
    {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("hostile-recovery");
        let resolver = fixture.resolver();
        let source = fixture.state_path();
        let bytes = b"{must remain";
        write_bytes(&source, bytes);
        let outside = fixture.root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, source.parent().unwrap().join("recovery")).unwrap();

        let (store, load) = load_state_authority(&resolver);
        let AuthorityLoad::Recovery { authority, .. } = load else {
            panic!("corrupt bytes must require recovery");
        };
        assert_eq!(authority.quarantine, QuarantineStatus::Failed);
        assert!(store.save_blocking(&empty_app_state()).is_err());
        assert_eq!(fs::read(source).unwrap(), bytes);
        assert!(fs::read_dir(outside).unwrap().next().is_none());
    }
    {
        let fixture = Fixture::new("future-sidecar");
        let resolver = fixture.resolver();
        let annotations_root = fixture.root.join("annotation-sidecars");
        let source = "/books/future.pdf";
        let sidecar = annotations_path_with_roots(source, &resolver, Some(&annotations_root));
        let future = br#"{"version":2,"source_path":"/books/future.pdf","annotations":[],"next_id":1,"bookmarks":[]}"#;
        write_bytes(&sidecar, future);

        for _ in 0..2 {
            let (store, load) =
                load_annotations_authority_with_roots(source, &resolver, Some(&annotations_root));
            let AuthorityLoad::Recovery { authority, .. } = load else {
                panic!("a future sidecar must be refused");
            };
            assert_eq!(authority.failure, StateFailureKind::FutureVersion);
            assert!(!authority.can_start_empty());
            assert!(store.save_blocking(&empty_annotations(source)).is_err());
            assert_eq!(fs::read(&sidecar).unwrap(), future);
        }
        assert!(
            !state_path_with_resolver(&resolver)
                .parent()
                .unwrap()
                .join("recovery")
                .exists()
        );
    }
}
