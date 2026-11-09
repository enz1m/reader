use super::*;

#[test]
fn annotation_destinations_cannot_alias_across_native_paths() {
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path();
    let invalid_one = docs.join(std::ffi::OsStr::from_bytes(b"one\xff.annotations.json"));
    let invalid_two = docs.join(std::ffi::OsStr::from_bytes(b"one\xfe.annotations.json"));
    assert_eq!(
        invalid_one.to_string_lossy(),
        invalid_two.to_string_lossy(),
        "the fixture is only discriminating while the lossy renderings match"
    );
    assert_ne!(
        annotations_destination_for(&invalid_one),
        annotations_destination_for(&invalid_two),
        "two paths differing only in invalid UTF-8 must not share a destination"
    );

    let unit_separator = docs.join(std::ffi::OsStr::from_bytes(b"a\x1fb.annotations.json"));
    let nested = docs.join("a").join("b.annotations.json");
    assert_ne!(
        annotations_destination_for(&unit_separator),
        annotations_destination_for(&nested),
        "a separator byte inside a component must not impersonate another path"
    );
    assert_ne!(
        annotations_destination_for(&unit_separator),
        annotations_destination_for(&invalid_one)
    );
    assert_eq!(
        annotations_destination_for(&nested),
        annotations_destination_for(&nested),
        "the same native path must always produce the same destination"
    );
    assert!(
        !annotations_destination_for(&unit_separator).contains('\u{1f}'),
        "a destination must not carry raw separator bytes"
    );
}
