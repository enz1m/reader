use super::*;
use crate::message::Msg;
use crate::model::{EpubViewMode, Format, Overlay, Screen};
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata};

mod notes_tests;
mod recents_tests;
mod unlock_tests;
mod zoom_tests;

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-chrome-{}-{}-{:?}",
        label,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn encrypted_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-encrypted.pdf")
}

fn synthetic_epub() -> EpubLoadResult {
    EpubLoadResult {
        metadata: EpubMetadata {
            title: "View Mode Book".into(),
            author: "A. Reader".into(),
            language: "en".into(),
            description: String::new(),
            publisher: String::new(),
        },
        chapters: vec![EpubChapter {
            title: "Chapter One".into(),
            heading: String::new(),
            content_html: "<p>A chapter.</p>".into(),
            plain_text: "A chapter.".into(),
            index: 0,
        }],
        toc: Vec::new(),
        footnotes: Vec::new(),
        footnote_refs: Vec::new(),
    }
}

fn finish_test_epub(app: &mut App, path: &std::path::Path) {
    app.model.file_path = Some(path.to_path_buf());
    app.model.file_name = "book.epub".into();
    app.model.format = Format::Epub;
    app.finish_epub(path.to_path_buf(), synthetic_epub());
}
