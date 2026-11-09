//! A chapter of thousands of paragraphs scrolls without rebuilding all of
//! them: scroll mode builds a window around the viewport over geometry that
//! is shaped off the UI thread, and over a quick estimate until it lands.

use super::*;
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata, ReadingParagraph};
use crate::reading_pane::ChapterGeometry;
use std::collections::HashMap;

fn app_with_chapter(paragraphs: usize) -> App {
    let (mut app, _t) = App::test_new();
    let text = "The quick brown fox jumps over the lazy dog while reading a long novel. ".repeat(4);
    let paras = (0..paragraphs)
        .map(|i| ReadingParagraph {
            chapter: 0,
            para_index: i,
            text: text.clone(),
            spans: Vec::new(),
            char_start: i * text.len(),
            char_end: (i + 1) * text.len(),
        })
        .collect();
    app.model.epub = Some(EpubDoc {
        load: EpubLoadResult {
            metadata: EpubMetadata::default(),
            chapters: vec![EpubChapter {
                title: "Whole book".into(),
                heading: String::new(),
                content_html: String::new(),
                plain_text: String::new(),
                index: 0,
            }],
            toc: Vec::new(),
            footnotes: Vec::new(),
            footnote_refs: Vec::new(),
        },
        chapter_paragraphs: HashMap::from([(0, paras)]),
        highlighted_chapter: None,
    });
    app.model.format = Format::Epub;
    app.model.screen = Screen::Reading;
    app.model.epub_current_chapter = 0;
    app.window_width = 1280.0;
    app.window_height = 800.0;
    app
}

fn scrolled(app: &App, offset: f32) -> Msg {
    Msg::EpubScrolled {
        generation: app.model.position_restore_generation,
        fraction: 0.5,
        offset,
        viewport_height: 700.0,
    }
}

#[test]
fn a_long_chapter_is_shaped_off_thread_once_and_windowed_meanwhile() {
    let mut app = app_with_chapter(2000);
    let _ = app.update(scrolled(&app, 30_000.0));
    let column = app.epub_column_width();
    let key = app
        .model
        .epub_geometry_key(column)
        .expect("a long chapter is windowed");
    assert_eq!(
        app.model.epub_geometry_pending,
        Some(key),
        "the update asked the worker for the chapter's geometry"
    );
    let window = app
        .model
        .epub_paragraph_window(column)
        .expect("the view windows over the estimate before the worker lands");
    assert!(
        window.range.len() < 200 && window.range.start > 0,
        "only paragraphs near the viewport are built: {:?}",
        window.range
    );

    let _ = app.update(scrolled(&app, 31_000.0));
    assert_eq!(app.model.epub_geometry_pending, Some(key));

    let texts: Vec<String> = app.model.epub.as_ref().unwrap().chapter_paragraphs[&0]
        .iter()
        .map(|p| p.text.clone())
        .collect();
    let measured = ChapterGeometry::measure(
        key,
        texts.iter().map(String::as_str),
        app.model.epub_font_size as f32,
        app.model.epub_line_spacing,
        column,
    );
    let _ = app.update(Msg::EpubGeometryMeasured(
        key,
        Some(std::sync::Arc::new(measured)),
    ));
    assert_eq!(app.model.epub_geometry_pending, None);
    assert!(
        app.model
            .epub_geometry
            .as_ref()
            .is_some_and(|g| g.key() == key)
    );
    let _ = app.update(scrolled(&app, 32_000.0));
    assert_eq!(
        app.model.epub_geometry_pending, None,
        "geometry already shaped for these metrics is not asked for again"
    );

    app.model.epub_font_size += 2;
    let _ = app.update(scrolled(&app, 32_000.0));
    assert!(
        app.model
            .epub_geometry_pending
            .is_some_and(|pending| pending != key),
        "a font size change asks for the chapter at the new size"
    );
}
