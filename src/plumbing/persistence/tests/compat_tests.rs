//! Back-compat defaults and bindings.

use super::*;

#[test]
fn test_pdf_annotation_binding_roundtrips_and_legacy_rows_default_to_unbound() {
    let mut file = AnnotationsFile::default();
    let id = add_annotation_with_position(
        &mut file,
        2,
        "highlight",
        "stable words",
        "yellow",
        AnnotationPosition::PdfRect {
            page: 2,
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.04,
        },
    );
    file.annotations[0].pdf_binding = Some(PdfAnnotationBinding {
        native_id: "native-9".to_string(),
        document_fingerprint: "document-a".to_string(),
        page_fingerprint: "page-a".to_string(),
        native_fingerprint: "annotation-a".to_string(),
        text_fingerprint: "text-a".to_string(),
    });
    let encoded = serde_json::to_string(&file).unwrap();
    let decoded: AnnotationsFile = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.annotations[0].id, id);
    assert_eq!(
        decoded.annotations[0].pdf_binding,
        file.annotations[0].pdf_binding
    );

    let legacy: Annotation = serde_json::from_str(
        r#"{"id":1,"page":0,"kind":"note","text":"old","color":"","created":"","position":"","style":"","note":""}"#,
    )
    .unwrap();
    assert!(legacy.pdf_binding.is_none());
}
