use super::*;
use crate::message::Msg;
use crate::model::{EpubDoc, Format, Screen};
use crate::pages::PageArrangement;
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata};
use crate::plumbing::pdf_navigation::PdfOutlineEntry;

fn tmpdir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "reader-pdf-navigation-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn build_outlined_pdf(path: &std::path::Path, pages: usize) {
    use lopdf::{Document, Object, dictionary};

    crate::perf_tests::build_pdf(path, pages, None);
    let mut document = Document::load(path).unwrap();
    let page_ids = document.get_pages();
    let outlines_id = document.new_object_id();
    let first_id = document.new_object_id();
    let second_id = document.new_object_id();
    let external_id = document.new_object_id();
    document.objects.insert(
        first_id,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Second page"),
            "Parent" => outlines_id,
            "Next" => second_id,
            "Dest" => vec![Object::Reference(page_ids[&2]), Object::Name(b"Fit".to_vec())],
        }),
    );
    document.objects.insert(
        second_id,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Last page"),
            "Parent" => outlines_id,
            "Prev" => first_id,
            "Next" => external_id,
            "A" => dictionary! {
                "S" => "GoTo",
                "D" => vec![Object::Reference(page_ids[&(pages as u32)]), Object::Name(b"Fit".to_vec())],
            },
        }),
    );
    document.objects.insert(
        external_id,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("External link"),
            "Parent" => outlines_id,
            "Prev" => second_id,
            "A" => dictionary! {
                "S" => "URI",
                "URI" => Object::string_literal("https://example.invalid"),
            },
        }),
    );
    document.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => first_id,
            "Last" => external_id,
            "Count" => 3,
        }),
    );
    document.catalog_mut().unwrap().set("Outlines", outlines_id);
    document.save(path).unwrap();
}

fn epub_with_two_chapters() -> EpubLoadResult {
    EpubLoadResult {
        metadata: EpubMetadata::default(),
        chapters: vec![
            EpubChapter {
                title: "One".into(),
                heading: String::new(),
                content_html: String::new(),
                plain_text: "one".into(),
                index: 0,
            },
            EpubChapter {
                title: "Two".into(),
                heading: String::new(),
                content_html: String::new(),
                plain_text: "two".into(),
                index: 1,
            },
        ],
        toc: Vec::new(),
        footnotes: Vec::new(),
        footnote_refs: Vec::new(),
    }
}

#[test]
fn pdf_outline_and_search_navigate_real_pages_with_stale_result_rejection() {
    {
        let dir = tmpdir("outline");
        let path = dir.join("outline.pdf");
        build_outlined_pdf(&path, 5);

        let data = crate::plumbing::pdf_navigation::load(&path, None).unwrap();

        assert_eq!(
            data.outline,
            vec![
                PdfOutlineEntry {
                    title: "Second page".into(),
                    page: 1,
                    level: 0,
                },
                PdfOutlineEntry {
                    title: "Last page".into(),
                    page: 4,
                    level: 0,
                },
            ]
        );
        assert!(matches!(
            crate::view::sidebar::pdf_outline_message(&data.outline[1]),
            Msg::GoToPage(4)
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let (mut app, _) = App::test_new();
        app.model.format = Format::Pdf;
        app.model.screen = Screen::Reading;
        app.model.page_count = 6;
        app.model.total_page_count = 6;
        app.model.page_extents = vec![
            crate::raster::PageExtent {
                width: 1000,
                height: 1200
            };
            6
        ];
        app.model.arrangement = PageArrangement::from_page_count(6);
        app.model.search_generation = 2;
        let hit = crate::search::PdfSearchHit {
            page: 4,
            line: 0,
            preview: "wanted".into(),
            rect: [10.0, 10.0, 20.0, 20.0],
        };

        let _ = app.update(Msg::PdfSearchCompleted {
            generation: 1,
            hits: vec![hit.clone()],
        });
        assert!(app.model.pdf_search_hits.is_empty());
        let _ = app.update(Msg::PdfSearchCompleted {
            generation: 2,
            hits: vec![hit],
        });

        assert_eq!(app.model.current_page, 4);
        assert_eq!(app.model.pdf_search_hits[0].page, 4);
        assert!(app.model.pages_scroll_offset > 0.0);
    }
}

#[test]
fn bookmark_restart_routes_only_to_the_matching_document_format() {
    let dir = tmpdir("bookmark-restart");
    let resolver = crate::app::test_xdg_resolver(&dir);
    let path = dir.join("book.pdf");
    crate::perf_tests::build_pdf(&path, 5, None);
    {
        let (mut app, _) = App::test_new();
        app.enable_persist(resolver.clone());
        app.model.file_path = Some(path.clone());
        app.model.format = Format::Pdf;
        app.model.screen = Screen::Reading;
        app.model.page_count = 5;
        app.model.total_page_count = 5;
        app.model.page_extents = vec![
            crate::raster::PageExtent {
                width: 1000,
                height: 1200
            };
            5
        ];
        app.model.arrangement = PageArrangement::from_page_count(5);
        app.load_annotations_for_document(&path);
        let _ = app.update(Msg::ToggleBookmark(3));
        let _ = app.update(Msg::CloseWindow(None));
        let _ = app.drain_persistence_debt_for_test();
    }

    let (mut reopened, _) = App::test_new();
    reopened.enable_persist(resolver);
    reopened.model.file_path = Some(path.clone());
    reopened.model.format = Format::Pdf;
    reopened.model.screen = Screen::Reading;
    reopened.model.page_count = 5;
    reopened.model.total_page_count = 5;
    reopened.model.page_extents = vec![
        crate::raster::PageExtent {
            width: 1000,
            height: 1200
        };
        5
    ];
    reopened.model.arrangement = PageArrangement::from_page_count(5);
    reopened.load_annotations_for_document(&path);
    assert_eq!(reopened.model.annotations.bookmarks[0].page, 3);
    let document_key = crate::plumbing::persistence::canonical_key(&path.to_string_lossy());

    let _ = reopened.update(Msg::GoToBookmark {
        format: Format::Pdf,
        document_key: document_key.clone(),
        position: 3,
    });
    assert_eq!(reopened.model.current_page, 3);
    let bookmark = reopened.model.annotations.bookmarks[0].clone();
    let _ = reopened.update(Msg::ToggleBookmark(3));
    assert!(reopened.model.annotations.bookmarks.is_empty());
    let _ = reopened.update(Msg::Undo);
    let restored = &reopened.model.annotations.bookmarks[0];
    assert_eq!(restored.page, bookmark.page);
    assert_eq!(restored.label, bookmark.label);
    assert_eq!(restored.created, bookmark.created);

    reopened.model.format = Format::Epub;
    reopened.model.epub = Some(EpubDoc {
        load: epub_with_two_chapters(),
        chapter_paragraphs: std::collections::HashMap::new(),
        highlighted_chapter: None,
    });
    reopened.model.epub_current_chapter = 0;
    let _ = reopened.update(Msg::GoToBookmark {
        format: Format::Pdf,
        document_key: document_key.clone(),
        position: 1,
    });
    assert_eq!(reopened.model.epub_current_chapter, 0);
    let _ = reopened.update(Msg::GoToBookmark {
        format: Format::Epub,
        document_key,
        position: 1,
    });
    assert_eq!(reopened.model.epub_current_chapter, 1);
    let _ = std::fs::remove_dir_all(dir);
}
