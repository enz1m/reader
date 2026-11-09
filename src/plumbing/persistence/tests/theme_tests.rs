//! Theme, anchor, and line-spacing persistence.

use super::*;

#[test]
fn epub_progress_anchor_roundtrips_and_legacy_records_default_to_none() {
    let anchored: AppState = serde_json::from_value(serde_json::json!({
        "version": 1,
        "files": {
            "/book.epub": {
                "page": 2,
                "total": 5,
                "epub_anchor": {
                    "paragraph": 7,
                    "char_offset": 19
                }
            }
        }
    }))
    .unwrap();
    let record = anchored.files.get("/book.epub").unwrap();
    assert_eq!(
        record.epub_anchor,
        Some(EpubProgressAnchor {
            paragraph: 7,
            char_offset: 19
        })
    );
    let roundtrip: AppState =
        serde_json::from_str(&serde_json::to_string(&anchored).unwrap()).unwrap();
    assert_eq!(
        roundtrip.files["/book.epub"].epub_anchor,
        record.epub_anchor
    );

    let legacy: AppState = serde_json::from_value(serde_json::json!({
        "version": 1,
        "files": {
            "/old.epub": {
                "page": 1,
                "total": 3
            }
        }
    }))
    .unwrap();
    assert_eq!(legacy.files["/old.epub"].epub_anchor, None);
}
