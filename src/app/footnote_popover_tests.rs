use super::*;
use crate::plumbing::footnotes::{Footnote, FootnoteRef};

fn chapter(index: usize, title: &str, text: &str) -> epub_support::EpubChapter {
    epub_support::EpubChapter {
        title: title.to_string(),
        heading: title.to_string(),
        content_html: format!("<p>{text}</p>"),
        plain_text: text.to_string(),
        index,
    }
}

fn app_with_footnotes(footnotes: Vec<Footnote>, footnote_refs: Vec<FootnoteRef>) -> App {
    let (mut app, _) = App::test_new();
    app.window_width = 1200.0;
    app.window_height = 800.0;
    app.model.screen = Screen::Reading;
    app.model.format = Format::Epub;
    app.model.file_path = Some(PathBuf::from("/generated/footnotes.epub"));
    app.model.file_name = "footnotes.epub".to_string();
    app.model.epub = Some(EpubDoc {
        load: EpubLoadResult {
            metadata: epub_support::EpubMetadata::default(),
            chapters: vec![
                chapter(0, "Reading", "A sentence with note 1 in its prose."),
                chapter(1, "Notes", "Collected notes."),
            ],
            toc: Vec::new(),
            footnotes,
            footnote_refs,
        },
        chapter_paragraphs: HashMap::new(),
        highlighted_chapter: None,
    });
    app.ensure_chapter_paragraphs();
    app
}

fn rendered_strings(app: &App) -> Vec<String> {
    use enzui_iced_core::collect::SemanticsCollector;
    use iced::advanced::renderer::Headless;
    use iced::advanced::widget::Operation;

    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        app.view(),
        iced::Size::new(1200.0, 800.0),
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let operation: &mut dyn Operation = &mut collector;
        operation
    });
    collector
        .into_collected()
        .into_iter()
        .flat_map(|node| [node.semantics.label, node.semantics.description])
        .flatten()
        .collect()
}

fn escape_messages(app: &App) -> Vec<Msg> {
    use iced_test::Simulator;

    let mut simulator = Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(1200.0, 800.0),
        app.view(),
    );
    let _ = simulator.tap_key(iced::keyboard::key::Named::Escape);
    simulator.into_messages().collect()
}

#[test]
fn cross_chapter_reference_opens_correct_body_and_preserves_position() {
    let mut app = app_with_footnotes(
        vec![
            Footnote {
                id: "shared".to_string(),
                label: "1".to_string(),
                text: "Wrong same-chapter body".to_string(),
                chapter: 0,
                source_href: None,
            },
            Footnote {
                id: "shared".to_string(),
                label: "1".to_string(),
                text: "Correct cross-chapter body".to_string(),
                chapter: 1,
                source_href: Some("notes.xhtml".to_string()),
            },
        ],
        vec![FootnoteRef {
            chapter: 0,
            target_id: "shared".to_string(),
            target_href: Some("notes.xhtml".to_string()),
            label: "1".to_string(),
        }],
    );
    app.model.epub_scroll_fraction = 0.625;
    app.model.epub_page_offset = 17;

    let _ = app.update(Msg::FootnoteClicked("shared".to_string()));

    assert_eq!(app.model.epub_current_chapter, 0);
    assert_eq!(app.model.epub_scroll_fraction, 0.625);
    assert_eq!(app.model.epub_page_offset, 17);
    assert_eq!(
        app.model
            .active_footnote
            .as_ref()
            .map(|note| note.text.as_str()),
        Some("Correct cross-chapter body")
    );
    let strings = rendered_strings(&app);
    assert!(
        strings
            .iter()
            .any(|text| text == "Correct cross-chapter body")
    );
    assert!(!strings.iter().any(|text| text == "Wrong same-chapter body"));
    assert!(strings.iter().any(|text| text == "Close footnote"));

    let dismissals = escape_messages(&app);
    assert!(
        dismissals
            .iter()
            .any(|message| matches!(message, Msg::DismissFootnote))
    );
    for message in dismissals {
        let _ = app.update(message);
    }
    assert!(app.model.active_footnote.is_none());
    assert_eq!(app.model.epub_current_chapter, 0);
    assert_eq!(app.model.epub_scroll_fraction, 0.625);
    assert_eq!(app.model.epub_page_offset, 17);
    assert!(
        !rendered_strings(&app)
            .iter()
            .any(|text| text == "Correct cross-chapter body")
    );
}
