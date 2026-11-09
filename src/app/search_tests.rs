use super::*;
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata};
use crate::plumbing::persistence::AnnotationPosition;
use enzui_iced_core::text_selection::DocPosition;
use enzui_iced_widgets::selection_scope::ScopeSelection;
use iced::{Point, Rectangle, Size};

fn chapter(index: usize, title: &str, text: &str) -> EpubChapter {
    EpubChapter {
        title: title.to_string(),
        heading: String::new(),
        content_html: String::new(),
        plain_text: text.to_string(),
        index,
    }
}

fn app_with_epub(path: Option<PathBuf>) -> App {
    let (mut app, _t) = App::test_new();
    let chapters = vec![
        chapter(0, "One", "The quick brown fox jumps over the lazy dog."),
        chapter(1, "Two", "Another dog and another fox nearby."),
    ];
    let load = EpubLoadResult {
        metadata: EpubMetadata {
            title: "Test Book".into(),
            author: "A. Author".into(),
            language: "en".into(),
            description: String::new(),
            publisher: String::new(),
        },
        chapters,
        toc: Vec::new(),
        footnotes: Vec::new(),
        footnote_refs: Vec::new(),
    };
    // Built as `finish_epub` does, which this helper bypasses.
    app.model.search_index = crate::search::SearchIndex::build(&load.chapters);
    app.model.epub = Some(EpubDoc {
        load,
        chapter_paragraphs: HashMap::new(),
        highlighted_chapter: None,
    });
    app.model.format = Format::Epub;
    app.model.screen = Screen::Reading;
    app.model.page_count = 2;
    app.model.file_path = path;
    app.model.file_name = "book.epub".into();
    app
}

fn select_first_paragraph(app: &mut App, start: usize, end: usize) {
    app.ensure_chapter_paragraphs();
    let text = app.model.epub.as_ref().unwrap().load.chapters[0]
        .plain_text
        .chars()
        .skip(start)
        .take(end - start)
        .collect();
    let _ = app.update(Msg::SelectionChanged(Some(ScopeSelection {
        text,
        start: DocPosition::new(0, start),
        end: DocPosition::new(0, end),
        bounds: Rectangle::new(Point::new(320.0, 240.0), Size::new(140.0, 24.0)),
    })));
}

#[test]
fn epub_highlights_and_notes_keep_passage_anchors_through_editing() {
    {
        let mut app = app_with_epub(None);
        select_first_paragraph(&mut app, 4, 9); // "quick"

        let _ = app.update(Msg::HighlightSelection);
        assert!(
            app.model.active_selection.is_none(),
            "action surface dismissed"
        );
        let annotation = app.model.annotations.annotations.last().unwrap();
        assert_eq!(annotation.text, "quick");
        assert_eq!(
            AnnotationPosition::parse(&annotation.position, annotation.page),
            AnnotationPosition::EpubCfi {
                chapter: 0,
                char_start: 4,
                char_end: 9,
            }
        );

        let _ = app.update(Msg::FontSizeUp);
        app.model
            .epub
            .as_mut()
            .unwrap()
            .load
            .chapters
            .push(EpubChapter {
                title: "Later chapter".into(),
                heading: String::new(),
                content_html: String::new(),
                plain_text: "A later passage.".into(),
                index: 2,
            });
        app.enter_chapter(2);
        assert!(
            !app.model
                .epub
                .as_ref()
                .unwrap()
                .chapter_paragraphs
                .contains_key(&0)
        );
        app.enter_chapter(0);
        let paragraphs = &app.model.epub.as_ref().unwrap().chapter_paragraphs[&0];
        assert!(
            paragraphs[0].spans.iter().any(|span| {
                span.char_start == 4
                    && span.char_end == 9
                    && matches!(
                        span.kind,
                        crate::plumbing::epub_support::InlineSpanKind::Highlight { .. }
                    )
            }),
            "the persisted CFI must repaint the same passage after reflow"
        );

        let _ = app.update(Msg::Undo);
        assert!(
            app.model.epub.as_ref().unwrap().chapter_paragraphs[&0][0]
                .spans
                .iter()
                .all(|span| !matches!(
                    span.kind,
                    crate::plumbing::epub_support::InlineSpanKind::Highlight { .. }
                )),
            "undo must invalidate cached paragraphs so visible highlight paint disappears"
        );
        let _ = app.update(Msg::Redo);
        assert!(
            app.model.epub.as_ref().unwrap().chapter_paragraphs[&0][0]
                .spans
                .iter()
                .any(|span| matches!(
                    span.kind,
                    crate::plumbing::epub_support::InlineSpanKind::Highlight { .. }
                )),
            "redo must rebuild cached paragraphs so visible highlight paint returns"
        );
    }
    {
        let mut app = app_with_epub(None);
        select_first_paragraph(&mut app, 10, 15); // "brown"
        let _ = app.update(Msg::AddNoteToSelection);
        assert_eq!(app.model.overlay, Some(model::Overlay::NoteEditor));
        assert_eq!(
            app.model.note_editor_position,
            Some(AnnotationPosition::EpubCfi {
                chapter: 0,
                char_start: 10,
                char_end: 15,
            })
        );
        app.model.epub_current_chapter = 1; // the captured anchor must win
        let _ = app.type_into_note_composer_for_test("remember this");
        let _ = app.update(Msg::CommitNoteEditor);
        let note = app.model.annotations.annotations.last().unwrap();
        assert_eq!(note.page, 0);
        assert_eq!(note.text, "remember this");
        assert_eq!(
            AnnotationPosition::parse(&note.position, note.page),
            AnnotationPosition::EpubCfi {
                chapter: 0,
                char_start: 10,
                char_end: 15,
            }
        );
    }
}
