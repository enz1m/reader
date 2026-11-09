//! Tests for EPUB parsing: spine order, TOC extraction, metadata, chapter rendering.

use reader_app::epub_support;

const QUIET_COAST: &str = "tests/fixtures/quiet-coast.epub";

#[test]
fn real_epub_loads_metadata_toc_and_ordered_chapters() {
    let result = epub_support::load_epub(QUIET_COAST).expect("load original EPUB fixture");

    assert!(
        !result.metadata.title.is_empty(),
        "Title should be non-empty"
    );

    assert!(!result.chapters.is_empty(), "Should have chapters");

    for (expected, ch) in result.chapters.iter().enumerate() {
        assert_eq!(ch.index, expected, "Chapter index mismatch");
    }

    assert!(!result.metadata.author.is_empty());
    assert!(!result.metadata.language.is_empty());
    assert!(
        result
            .chapters
            .iter()
            .any(|chapter| chapter.plain_text.len() > 50)
    );
    assert_eq!(
        result
            .toc
            .iter()
            .map(|entry| entry.chapter_index)
            .collect::<Vec<_>>(),
        (0..result.chapters.len()).collect::<Vec<_>>()
    );
    assert!(!result.toc.is_empty());
    for entry in &result.toc {
        assert!(!entry.title.is_empty());
        assert!(entry.chapter_index < result.chapters.len());
    }
}
