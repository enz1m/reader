use super::*;
use std::io::{Read, Write};

const CHAPTER: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Small</title></head><body><p>Bounded chapter.</p></body></html>";

fn repeated_chapter_book(
    copies: usize,
    chapter: &str,
) -> (tempfile::TempDir, std::path::PathBuf, usize) {
    let sandbox = tempfile::tempdir().expect("EPUB sandbox");
    let path = sandbox.path().join("repeated.epub");
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open("tests/fixtures/footnotes.epub").unwrap())
            .unwrap();
    let chapter_bytes = chapter.len();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if entry.name() == "OEBPS/content.opf" {
            let mut opf = String::new();
            entry.read_to_string(&mut opf).unwrap();
            let start = opf.find("<spine>").unwrap() + "<spine>".len();
            let end = opf.find("</spine>").unwrap();
            opf.replace_range(start..end, &"<itemref idref=\"ch1\"/>".repeat(copies));
            writer
                .start_file(
                    "OEBPS/content.opf",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(opf.as_bytes()).unwrap();
        } else if entry.name() == "OEBPS/chapter_1.xhtml" {
            writer
                .start_file(entry.name(), zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(chapter.as_bytes()).unwrap();
        } else {
            writer.raw_copy_file(entry).unwrap();
        }
    }
    writer.finish().unwrap();
    (sandbox, path, chapter_bytes)
}

#[test]
fn epub_spine_loading_bounds_repeated_chapters_and_content() {
    {
        let (_sandbox, path, _) = repeated_chapter_book(8_193, CHAPTER);
        let result = load_epub(path.to_str().unwrap());
        assert!(
            matches!(result, Err(ref error) if error.contains("too many chapter references")),
            "default chapter-reference admission must refuse the book"
        );
    }
    {
        let prefix = "<html><head><title>Small</title></head><body><!--";
        let suffix = "--><p>Bounded chapter.</p></body></html>";
        let mut chapter = String::from(prefix);
        chapter.extend(std::iter::repeat_n(
            'x',
            32 * 1024 * 1024 - prefix.len() - suffix.len(),
        ));
        chapter.push_str(suffix);
        let (_sandbox, path, bytes) = repeated_chapter_book(9, &chapter);
        assert_eq!(bytes, 32 * 1024 * 1024);
        drop(chapter);
        let result = load_epub(path.to_str().unwrap());
        assert!(
            matches!(result, Err(ref error) if error.contains("decoded chapter-content limit")),
            "default decoded-content admission must count repeated resources"
        );
    }
}
