use super::*;
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata};
use crate::plumbing::persistence::{AppState, EpubProgressAnchor, ProgressRecord, set_progress};

fn chapter(index: usize, text: &str) -> EpubChapter {
    EpubChapter {
        title: format!("Chapter {}", index + 1),
        heading: format!("Chapter {}", index + 1),
        content_html: String::new(),
        plain_text: text.to_string(),
        index,
    }
}

fn epub(chapters: &[&str]) -> EpubLoadResult {
    EpubLoadResult {
        metadata: EpubMetadata::default(),
        chapters: chapters
            .iter()
            .enumerate()
            .map(|(index, text)| chapter(index, text))
            .collect(),
        toc: Vec::new(),
        footnotes: Vec::new(),
        footnote_refs: Vec::new(),
    }
}

fn position_restore_requests(task: Task<Msg>) -> Vec<Msg> {
    let observed = super::task_observer::observe(task);
    assert!(observed.closes.is_empty());
    assert_eq!(observed.other_actions, 0);
    assert_eq!(
        observed.image_allocations,
        observed
            .messages
            .iter()
            .filter(|message| matches!(message, Msg::PageRasterAllocated { .. }))
            .count()
    );
    observed
        .messages
        .into_iter()
        .filter_map(|message| match message {
            request @ Msg::BeginPositionRestore { .. } => Some(request),
            Msg::NativePdfAnnotationsLoaded { .. }
            | Msg::PdfNavigationLoaded { .. }
            | Msg::PageRasterAllocated { .. } => None,
            _ => panic!("unexpected document-startup message"),
        })
        .collect()
}

fn effect_count(task: Task<Msg>) -> usize {
    let observed = super::task_observer::observe(task);
    assert!(observed.closes.is_empty());
    assert_eq!(observed.other_actions, observed.widget_operations);
    assert!(
        observed
            .messages
            .iter()
            .all(|message| matches!(message, Msg::PageRasterAllocated { .. }))
    );
    observed.widget_operations
}

fn seed(app: &mut App, path: &Path, record: ProgressRecord) {
    let format = record.format.clone();
    let mut state = AppState::default();
    set_progress(&mut state, &path.to_string_lossy(), record);
    app.seed_app_state_for_test(state);
    app.model.file_path = Some(path.to_path_buf());
    app.model.file_name = services::file_name(path);
    app.model.format = if format.eq_ignore_ascii_case("epub") {
        Format::Epub
    } else if format.eq_ignore_ascii_case("cbz") {
        Format::Cbz
    } else {
        Format::Pdf
    };
}

fn page_bitmaps(count: usize) -> Vec<message::PageBitmap> {
    (0..count)
        .map(|index| message::PageBitmap {
            index,
            rgba: vec![0, 0, 0, 255],
            width: 1,
            height: 1,
        })
        .collect()
}

fn cbz_outcome(count: usize) -> LoadOutcome {
    LoadOutcome::Cbz(Box::new(plumbing::cbz_support::CbzLoadResult {
        entries: (0..count)
            .map(|index| plumbing::cbz_support::CbzPage {
                archive_name: format!("page-{index}.png"),
                extent: crate::raster::PageExtent {
                    width: 1,
                    height: 1,
                },
                encoded_bytes: 4,
            })
            .collect(),
        pages: page_bitmaps(count),
    }))
}

#[test]
fn pdf_and_epub_restore_the_persisted_reading_position() {
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.pdf");
        std::fs::write(&path, b"fixture").unwrap();
        let (mut app, _task) = App::test_new();
        seed(
            &mut app,
            &path,
            ProgressRecord {
                page: 8,
                total: 9,
                format: "pdf".into(),
                ..Default::default()
            },
        );

        let task = app.update(Msg::DocumentLoaded {
            attempt: app.model.document_load_attempt,
            path: path.clone(),
            format: Format::Pdf,
            outcome: LoadOutcome::Pages {
                pages: page_bitmaps(3),
                total_pages: 3,
                extents: vec![
                    crate::raster::PageExtent {
                        width: 1,
                        height: 1
                    };
                    3
                ],
                text: vec![crate::pdf::PdfPageText::default(); 3],
            },
        });

        assert_eq!(app.model.current_page, 2);
        assert!(app.model.pages_scroll_offset > 0.0);
        let generation = app.model.position_restore_generation;
        let target = app.model.pages_scroll_offset;
        let begin = position_restore_requests(task);
        assert_eq!(begin.len(), 1);
        let operation = app.update(begin.into_iter().next().unwrap());
        assert!(effect_count(operation) > 0);
        drop(app.update(Msg::PagesScrolled {
            generation,
            offset: target,
            viewport_height: 500.0,
            viewport_width: 0.0,
        }));
        assert_eq!(
            app.model.app_state.files[&persistence::canonical_key(&path.to_string_lossy())].page,
            2
        );
        assert_eq!(
            crate::live_feedback::announced(&app).as_deref(),
            Some("Reading position adjusted because the document changed.")
        );
    }
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, b"fixture").unwrap();
        let (mut app, _task) = App::test_new();
        seed(
            &mut app,
            &path,
            ProgressRecord {
                page: 1,
                total: 2,
                format: "epub".into(),
                epub_view_mode: Some("scrolled".into()),
                epub_anchor: Some(EpubProgressAnchor {
                    paragraph: 1,
                    char_offset: 3,
                }),
                ..Default::default()
            },
        );

        let task = app.finish_document_load(
            path.clone(),
            Format::Epub,
            LoadOutcome::Epub(Box::new(epub(&[
                "cover",
                "first paragraph\nsecond paragraph continues",
            ]))),
        );

        assert_eq!(app.model.epub_current_chapter, 1);
        assert_eq!(
            app.model.epub_page_offset,
            "first paragraph\nsec".chars().count()
        );
        assert!(app.model.epub_scroll_fraction > 0.0);
        let expected_fraction = app.model.epub_scroll_fraction;
        let generation = app.model.position_restore_generation;
        let key = persistence::canonical_key(&path.to_string_lossy());
        drop(app.update(Msg::EpubScrolled {
            generation,
            fraction: 0.0,
            offset: 0.0,
            viewport_height: 0.0,
        }));
        assert_eq!(
            app.model.app_state.files[&key].epub_anchor,
            Some(EpubProgressAnchor {
                paragraph: 1,
                char_offset: 3
            })
        );
        let begin = drain_task(task);
        assert_eq!(begin.len(), 1);
        let operation = app.update(begin.into_iter().next().unwrap());
        assert!(effect_count(operation) > 0);
        drop(app.update(Msg::EpubScrolled {
            generation,
            fraction: expected_fraction,
            offset: 0.0,
            viewport_height: 0.0,
        }));
        assert!(app.model.pending_position_restore.is_none());
        assert!(crate::live_feedback::announced(&app).is_none());

        drop(app.update(Msg::EpubScrolled {
            generation,
            fraction: 0.9,
            offset: 0.0,
            viewport_height: 0.0,
        }));
        let saved = app.model.app_state.files[&key]
            .epub_anchor
            .as_ref()
            .unwrap();
        assert_eq!(saved.paragraph, 1);
        assert!(saved.char_offset > 3);
    }
}

#[test]
fn changed_documents_and_user_navigation_supersede_saved_anchors() {
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.epub");
        std::fs::write(&path, b"fixture").unwrap();
        let (mut app, _task) = App::test_new();
        seed(
            &mut app,
            &path,
            ProgressRecord {
                page: 9,
                total: 10,
                format: "epub".into(),
                epub_anchor: Some(EpubProgressAnchor {
                    paragraph: 8,
                    char_offset: 90,
                }),
                ..Default::default()
            },
        );

        let task = app.update(Msg::DocumentLoaded {
            attempt: app.model.document_load_attempt,
            path: path.clone(),
            format: Format::Epub,
            outcome: LoadOutcome::Epub(Box::new(epub(&["cover", "short\nlast"]))),
        });

        assert_eq!(app.model.epub_current_chapter, 1);
        assert_eq!(app.model.epub_page_offset, "short\nlas".chars().count());
        let generation = app.model.position_restore_generation;
        let expected_fraction = app.model.epub_scroll_fraction;
        let begin = drain_task(task);
        assert_eq!(begin.len(), 1);
        let operation = app.update(begin.into_iter().next().unwrap());
        assert!(effect_count(operation) > 0);
        drop(app.update(Msg::EpubScrolled {
            generation,
            fraction: expected_fraction,
            offset: 0.0,
            viewport_height: 0.0,
        }));
        assert_eq!(
            app.model.app_state.files[&persistence::canonical_key(&path.to_string_lossy())]
                .epub_anchor,
            Some(EpubProgressAnchor {
                paragraph: 1,
                char_offset: 3
            })
        );
        assert_eq!(
            crate::live_feedback::announced(&app).as_deref(),
            Some("Reading position adjusted because the document changed.")
        );
    }
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user-scroll.cbz");
        std::fs::write(&path, b"fixture").unwrap();
        let (mut app, _task) = App::test_new();
        seed(
            &mut app,
            &path,
            ProgressRecord {
                page: 1,
                total: 2,
                format: "cbz".into(),
                ..Default::default()
            },
        );
        let startup = app.finish_document_load(path.clone(), Format::Cbz, cbz_outcome(2));
        let restore_generation = app.model.position_restore_generation;
        let key = persistence::canonical_key(&path.to_string_lossy());
        let begin = drain_task(startup);
        let operation = app.update(begin.into_iter().next().unwrap());
        assert!(effect_count(operation) > 0);
        let stale_target = app.model.pages_scroll_offset;

        drop(app.update(Msg::PagesScrolled {
            generation: restore_generation,
            offset: 10.0,
            viewport_height: 100.0,
            viewport_width: 0.0,
        }));
        assert!(app.model.pending_position_restore.is_none());
        assert_ne!(app.model.position_restore_generation, restore_generation);
        assert_eq!(app.model.current_page, 0);
        assert_eq!(app.model.app_state.files[&key].page, 0);
        assert_eq!(
            crate::view::shell::indicator_text(&app.model),
            "Page 1 of 2"
        );

        drop(app.update(Msg::PagesScrolled {
            generation: restore_generation,
            offset: stale_target,
            viewport_height: 100.0,
            viewport_width: 2048.0,
        }));
        assert_eq!(app.model.current_page, 0);
        assert_eq!(app.model.app_state.files[&key].page, 0);
        assert_eq!(app.model.pages_viewport_width, 0.0);
    }
}
