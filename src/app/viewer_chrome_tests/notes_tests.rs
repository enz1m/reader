//! Note composer and edits.

use super::*;

// Drives the real `app.view()` with a retained renderer and cache so focus and caret
// survive rebuilds; every event goes back through `app.update`, the only path that
// advances the editor `Content`.
struct ComposerHarness {
    renderer: iced_test::renderer::Renderer,
    cache: iced_test::runtime::user_interface::Cache,
    size: iced::Size,
}

impl ComposerHarness {
    fn new(width: f32, height: f32) -> Self {
        use iced::advanced::renderer::Headless;
        let renderer = iced_test::futures::futures::executor::block_on(
            <iced_test::renderer::Renderer as Headless>::new(
                Default::default(),
                std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
            ),
        )
        .expect("headless renderer");
        Self {
            renderer,
            cache: iced_test::runtime::user_interface::Cache::default(),
            size: iced::Size::new(width, height),
        }
    }

    fn dispatch(&mut self, app: &mut App, events: &[iced_test::core::Event]) -> Vec<Msg> {
        use iced::advanced::widget::{Id, Operation, operation};
        let mut messages = iced::advanced::shell::Bus::new();
        {
            let mut ui = iced_test::runtime::user_interface::UserInterface::build(
                app.view(),
                self.size,
                std::mem::take(&mut self.cache),
                &mut self.renderer,
            );
            let mut focus: Box<dyn Operation> = Box::new(operation::focusable::focus(Id::from(
                crate::view::note_editor::NOTE_FIELD_ID.to_owned(),
            )));
            ui.operate(&self.renderer, focus.as_mut());
            let _ = ui.update(
                &iced_test::core::window::Headless,
                &iced_test::core::shell::Waker::noop(),
                events,
                iced_test::core::mouse::Cursor::Unavailable,
                &mut self.renderer,
                &mut messages,
            );
            self.cache = ui.into_cache();
        }
        for m in &messages {
            let _ = app.update(m.clone());
        }
        messages.into_iter().collect()
    }

    fn type_char(&mut self, app: &mut App, c: char) -> Vec<Msg> {
        let s = iced_test::core::SmolStr::new_inline(&c.to_string());
        let key = iced::keyboard::Key::Character(s.clone());
        let events = [
            iced_test::simulator::press_key(key.clone(), Some(s)),
            iced_test::simulator::release_key(key),
        ];
        self.dispatch(app, &events)
    }

    fn press_enter(&mut self, app: &mut App) -> Vec<Msg> {
        let key = iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter);
        let events = [
            iced_test::simulator::press_key(key.clone(), None),
            iced_test::simulator::release_key(key),
        ];
        self.dispatch(app, &events)
    }

    fn type_text(&mut self, app: &mut App, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.press_enter(app);
            } else {
                self.type_char(app, c);
            }
        }
    }
}

fn composer_draft(app: &App) -> String {
    crate::view::note_editor::committed_note(&app.model).expect("composer is open")
}

fn click_composer_button(app: &App, label: &str) -> Vec<Msg> {
    use iced_test::Simulator;
    let mut ui: Simulator<'_, Msg> = Simulator::new(app.view());
    ui.click(label)
        .unwrap_or_else(|e| panic!("the composer's {label} button must be clickable: {e:?}"));
    ui.into_messages().collect()
}

#[test]
fn a_two_paragraph_note_saved_through_the_button_round_trips() {
    let dir = tmpdir("note-two-paragraphs");
    let resolver = crate::app::test_xdg_resolver(&dir);
    let pdf_path = dir.join("doc.pdf");
    std::fs::write(&pdf_path, b"%PDF-1.4\n").unwrap();
    let body = "First paragraph, with a comma.\n\nSecond paragraph:\n- a\n- b";

    {
        let (mut app, _t) = App::test_new();
        app.enable_persist(resolver.clone());
        app.model.file_path = Some(pdf_path.clone());
        app.model.file_name = "doc.pdf".to_string();
        app.model.format = Format::Pdf;
        app.model.screen = Screen::Reading;
        app.model.page_count = 9;
        app.model.current_page = 4;
        app.load_annotations_for_document(&pdf_path);

        let _ = app.update(Msg::OpenNoteEditor);
        let mut ui = ComposerHarness::new(1024.0, 768.0);
        ui.type_text(&mut app, body);
        assert_eq!(composer_draft(&app), body, "the typed draft matches");

        for m in click_composer_button(&app, "Save") {
            app.drive_pdf_mutation(m);
        }
        assert_eq!(app.model.overlay, None, "Save closes the composer");
        assert_eq!(app.model.annotations.annotations.len(), 1);
        assert_eq!(app.model.annotations.annotations[0].text, body);
        app.drive_pdf_mutation(Msg::CloseWindow(None));
        let _ = app.drain_persistence_debt_for_test();
    }

    {
        let (mut app2, _t) = App::test_new();
        app2.enable_persist(resolver.clone());
        app2.model.file_path = Some(pdf_path.clone());
        app2.load_annotations_for_document(&pdf_path);
        assert_eq!(
            app2.model.annotations.annotations.len(),
            1,
            "the note is on disk"
        );
        assert_eq!(app2.model.annotations.annotations[0].text, body);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
