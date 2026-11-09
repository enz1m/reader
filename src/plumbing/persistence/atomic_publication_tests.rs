use super::state_support::inject_raw_writer;
use super::*;
use std::os::unix::fs::MetadataExt;
use tempfile::TempDir;

fn publication_dir(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("reader-app-atomic-{tag}-"))
        .tempdir()
        .expect("claim a publication sandbox")
}

fn resolver_at(root: &Path) -> XdgResolver {
    XdgResolver::new(
        None::<PathBuf>,
        Some(root.to_path_buf()),
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

fn loaded_state(resolver: &XdgResolver) -> String {
    serialize_state_pretty(&load_state_with_resolver(resolver)).expect("serialize reloaded state")
}

fn state_with(path: &str, page: usize) -> AppState {
    let mut state = AppState::default();
    state.files.insert(
        path.to_string(),
        ProgressRecord {
            page,
            total: 400,
            ..ProgressRecord::default()
        },
    );
    state
}

fn annotations_with(source: &str, text: &str) -> AnnotationsFile {
    let mut file = AnnotationsFile {
        source_path: source.to_string(),
        version: default_version(),
        next_id: 1,
        ..AnnotationsFile::default()
    };
    add_annotation(&mut file, 2, "note", text, "yellow");
    file
}

// The staged prefix must differ from the original so partial in-place writes cannot pass unchanged.
fn first_differing_index(old: &[u8], new: &[u8]) -> usize {
    let index = old
        .iter()
        .zip(new)
        .position(|(a, b)| a != b)
        .expect("the two fixtures must differ inside their common prefix");
    assert_ne!(old[index], new[index]);
    index
}

fn inode(path: &Path) -> u64 {
    fs::metadata(path).expect("publication metadata").ino()
}

fn entries_besides(dir: &Path, keep: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .expect("read publication directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != keep)
        .collect()
}

#[test]
fn annotation_sidecar_publication_survives_an_interrupted_staged_write() {
    let sandbox = publication_dir("annotations");
    let dir = sandbox.path();
    let resolver = resolver_at(dir);
    let root = dir.join("sidecars");
    let source = dir.join("doc.pdf");
    fs::write(&source, b"%PDF-1.4\n").unwrap();
    let source_key = source.to_string_lossy().to_string();
    let path = annotations_path_with_roots(&source_key, &resolver, Some(&root));

    let old = annotations_with(&source_key, "seeded highlight");
    save_annotations_with_roots(&source_key, &old, &resolver, Some(&root))
        .expect("seed the previous sidecar");
    let old_bytes = fs::read(&path).expect("seeded bytes");
    let seeded_inode = inode(&path);

    let replacement = annotations_with(&source_key, "replacement highlight");
    let replacement_bytes = serialize_annotations_pretty(&replacement)
        .expect("serialize replacement")
        .into_bytes();
    let index = first_differing_index(&old_bytes, &replacement_bytes);
    let old_byte = old_bytes[index];
    let new_byte = replacement_bytes[index];
    inject_raw_writer(move |staged, bytes| {
        assert_eq!(bytes[index], new_byte);
        assert_ne!(bytes[index], old_byte);
        staged.write_all(&bytes[..=index])?;
        Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "injected after the staged prefix",
        ))
    });

    let error = write_serialized_annotations_with_roots(
        &source_key,
        std::str::from_utf8(&replacement_bytes).unwrap(),
        &resolver,
        Some(&root),
    )
    .expect_err("the interrupted write must be reported");
    assert!(error.starts_with("Cannot write annotations"), "{error}");
    assert_eq!(fs::read(&path).expect("old bytes"), old_bytes);
    assert_eq!(
        inode(&path),
        seeded_inode,
        "the old sidecar was replaced anyway"
    );
    assert_eq!(
        load_annotations_with_roots(&source_key, &resolver, Some(&root)).annotations[0].text,
        "seeded highlight"
    );
    assert!(
        entries_besides(&root, &path).is_empty(),
        "the interrupted write left staging residue"
    );

    save_annotations_with_roots(&source_key, &replacement, &resolver, Some(&root))
        .expect("the retry publishes");
    let published = fs::read(&path).expect("published bytes");
    assert_eq!(published, replacement_bytes);
    assert_ne!(
        inode(&path),
        seeded_inode,
        "publication must install a new inode rather than rewrite the old one"
    );
    assert_eq!(
        load_annotations_with_roots(&source_key, &resolver, Some(&root)).annotations[0].text,
        "replacement highlight"
    );
    assert!(!published.starts_with(b"enzui-iced "));
}

#[cfg(unix)]
#[test]
fn a_process_named_temp_symlink_cannot_redirect_a_publication() {
    use std::os::unix::fs::symlink;

    let sandbox = publication_dir("planted-temp");
    let dir = sandbox.path();
    let resolver = resolver_at(dir);
    let path = state_path_with_resolver(&resolver);
    let parent = path.parent().expect("state dir").to_path_buf();
    drop(app_state_store_with_resolver(&resolver));
    let victim = dir.join("victim");
    fs::write(&victim, b"precious").unwrap();
    let planted = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().expect("state file name").to_string_lossy(),
        std::process::id()
    ));
    symlink(&victim, &planted).unwrap();

    let state = state_with("/books/planted.pdf", 3);
    save_state_with_resolver(&state, &resolver).expect("publish over a planted temp name");

    assert_eq!(fs::read(&victim).expect("victim bytes"), b"precious");
    assert!(
        fs::symlink_metadata(&path)
            .expect("destination metadata")
            .file_type()
            .is_file()
    );
    assert_eq!(
        loaded_state(&resolver),
        serialize_state_pretty(&state).unwrap()
    );
}
