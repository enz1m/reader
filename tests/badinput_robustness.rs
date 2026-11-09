//! Malformed-input robustness: the loaders must return Err, never panic.
use reader_app::epub_support;
use reader_app::pdf;

#[test]
fn garbage_pdf_errors_not_panics() {
    let dir = std::env::temp_dir().join(format!("reader-bad3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("garbage.pdf");
    std::fs::write(&p, b"%PDF-1.5\nnot a real pdf body at all\n").unwrap();
    let r = pdf::render_pdf(p.to_str().unwrap(), None);
    assert!(r.is_err(), "garbage pdf should Err, got Ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_spine_resource_is_reported_instead_of_shifting_chapter_indices() {
    let temp = tempfile::tempdir().expect("archive sandbox");
    let path = temp.path().join("incomplete.epub");
    let source = std::fs::File::open("tests/fixtures/footnotes.epub").expect("fixture");
    let mut archive = zip::ZipArchive::new(source).expect("fixture zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).expect("output"));
    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("entry");
        if entry.name() != "OEBPS/chapter_2.xhtml" {
            writer.raw_copy_file(entry).expect("copy entry");
        }
    }
    writer.finish().expect("finish zip");
    let complete = epub_support::load_epub("tests/fixtures/footnotes.epub").expect("complete book");
    assert!(
        complete.chapters.len() >= 3,
        "missing middle chapter has a successor"
    );
    let error = epub_support::load_epub(path.to_str().expect("UTF-8 path"))
        .expect_err("silently skipped chapters invalidate navigation and annotation anchors");
    assert!(
        error.contains("chapter 2"),
        "missing chapter must be identified: {error}"
    );
}

#[test]
fn oversized_epub_is_refused_before_parsing() {
    let temp = tempfile::tempdir().expect("archive sandbox");
    let path = temp.path().join("oversized.epub");
    let file = std::fs::File::create(&path).expect("sparse fixture");
    file.set_len(128 * 1024 * 1024 + 1).expect("sparse length");
    let error = epub_support::load_epub(path.to_str().expect("UTF-8 path"))
        .expect_err("oversized archive must be refused");
    assert!(
        error.contains("128 MiB"),
        "archive must be refused before ZIP parsing: {error}"
    );
    assert!(epub_support::cover_thumbnail(path.to_str().unwrap(), 112).is_none());
}
