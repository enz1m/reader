//! Parse temporary footnote EPUBs and verify extraction and inline reference payloads.

use std::io::{Cursor, Write};

fn create_footnotes_epub_bytes() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);

        // EPUB requires mimetype to be the first entry, stored without compression.
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflate = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("META-INF/container.xml", deflate).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/content.opf", deflate).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">urn:uuid:footnote-test-001</dc:identifier>
    <dc:title>Footnote Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="ch1" href="chapter_1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="chapter_2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="chapter_3.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="toc.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
    <itemref idref="ch3"/>
  </spine>
</package>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/toc.xhtml", deflate).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Table of Contents</title></head>
<body>
  <nav epub:type="toc">
    <ol>
      <li><a href="chapter_1.xhtml">The Discovery</a></li>
      <li><a href="chapter_2.xhtml">Further Research</a></li>
      <li><a href="chapter_3.xhtml">Endnotes</a></li>
    </ol>
  </nav>
</body>
</html>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/chapter_1.xhtml", deflate).unwrap();
        zip.write_all(
            br##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>The Discovery</title></head>
<body>
  <h1>The Discovery</h1>
  <p>Professor Elena Vasquez knelt beside the riverbank, brushing away centuries of sediment. Beneath her careful fingers lay a curious mineral<a epub:type="noteref" href="#fn1">1</a> unlike anything previously catalogued.</p>
  <p>She placed the sample into a glass vial, noting how it caught the afternoon light with an iridescent shimmer that seemed almost alive.</p>
  <aside epub:type="footnote" id="fn1"><p>The mineral was later classified as Vasquezite, a previously unknown crystalline silicate with piezoelectric properties.</p></aside>
</body>
</html>"##,
        )
        .unwrap();

        zip.start_file("OEBPS/chapter_2.xhtml", deflate).unwrap();
        zip.write_all(
            br##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Further Research</title></head>
<body>
  <h1>Further Research</h1>
  <p>Back at the university laboratory, Professor Vasquez subjected the sample to a battery of tests. The results were extraordinary<a epub:type="noteref" href="chapter_3.xhtml#en1">2</a> and defied conventional mineralogy.</p>
  <p>She published her initial findings in the Journal of Applied Geochemistry, sparking immediate interest from research teams worldwide.</p>
</body>
</html>"##,
        )
        .unwrap();

        zip.start_file("OEBPS/chapter_3.xhtml", deflate).unwrap();
        zip.write_all(
            br##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Endnotes</title></head>
<body>
  <h1>Endnotes</h1>
  <aside epub:type="endnote" id="en1"><p>X-ray diffraction analysis revealed a hexagonal crystal system with an unusually large unit cell, containing trace amounts of rare earth elements.</p></aside>
</body>
</html>"##,
        )
        .unwrap();

        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn write_temp_fixture() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("reader-app-footnote-fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir
        .join(format!("footnotes-{}-{}.epub", std::process::id(), n))
        .to_string_lossy()
        .into_owned();
    let data = create_footnotes_epub_bytes();
    std::fs::write(&path, &data).expect("write temp footnotes.epub");
    path
}

#[test]
fn footnotes_produce_inline_decorations() {
    let fixture_path = write_temp_fixture();

    let result =
        reader_app::epub_support::load_epub(&fixture_path).expect("should load footnotes.epub");

    let clicked = result
        .resolve_footnote(1, "en1")
        .expect("cross-chapter endnote");
    assert_eq!(clicked.chapter, 2);
    assert!(clicked.text.contains("X-ray diffraction"));
    let ch1 = &result.chapters[0];
    let located =
        reader_app::epub_support::locate_footnote_refs(&ch1.plain_text, 0, &result.footnote_refs);
    assert!(
        !located.is_empty(),
        "expected located footnote refs in Chapter 1"
    );
    let loc1 = located.iter().find(|l| l.target_id == "fn1").unwrap();
    assert_eq!(loc1.label, "1");

    let paragraphs = reader_app::epub_support::build_chapter_reading_paragraphs(
        ch1,
        &[], // no highlights
        &located,
    );
    assert!(
        !paragraphs.is_empty(),
        "expected reading paragraphs from Chapter 1"
    );

    let decorated = paragraphs.iter().find(|p| !p.spans.is_empty());
    assert!(
        decorated.is_some(),
        "expected at least one paragraph with inline spans for the footnote ref"
    );
    let d = decorated.unwrap();
    assert!(
        d.text.contains("mineral"),
        "the decorated paragraph should contain 'mineral'"
    );

    use reader_app::epub_support::InlineSpanKind;
    assert!(
        d.spans.iter().any(|s| matches!(
            &s.kind,
            InlineSpanKind::FootnoteRef { target_id } if target_id == "fn1"
        )),
        "spans should contain a FootnoteRef with id 'fn1', got: {:?}",
        d.spans
    );
}
