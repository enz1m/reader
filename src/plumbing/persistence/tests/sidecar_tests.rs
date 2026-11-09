//! Sidecar disk roundtrips and fallbacks.

use super::*;

#[test]
fn test_mixed_positions_sidecar_roundtrip_via_disk() {
    let dir = tmpdir();
    let src = dir.join("mixed.pdf");
    fs::write(&src, b"%PDF-1.4\n").unwrap();
    let src_str = src.to_string_lossy().to_string();

    let mut f = load_annotations(&src_str);
    add_annotation_with_position(
        &mut f,
        3,
        "highlight",
        "quick brown fox",
        "yellow",
        AnnotationPosition::PdfRect {
            page: 3,
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.03,
        },
    );
    add_annotation_with_position(
        &mut f,
        1,
        "highlight",
        "insect",
        "orange",
        AnnotationPosition::EpubCfi {
            chapter: 1,
            char_start: 50,
            char_end: 56,
        },
    );
    add_annotation(&mut f, 5, "note", "chapter comment", "");
    save_annotations(&src_str, &f).unwrap();

    let f2 = load_annotations(&src_str);
    assert_eq!(f2.annotations.len(), 3);
    assert!(matches!(
        AnnotationPosition::parse(&f2.annotations[0].position, 0),
        AnnotationPosition::PdfRect { .. }
    ));
    assert!(matches!(
        AnnotationPosition::parse(&f2.annotations[1].position, 0),
        AnnotationPosition::EpubCfi { .. }
    ));
    assert!(matches!(
        AnnotationPosition::parse(&f2.annotations[2].position, 0),
        AnnotationPosition::None
    ));
}
