use super::*;
use crate::model::Overlay;
use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Physical};

fn char_key(c: &str, modifiers: Modifiers) -> iced::Event {
    iced::Event::Keyboard(KeyEvent::KeyPressed {
        key: Key::Character(c.into()),
        modified_key: Key::Character(c.into()),
        physical_key: Physical::Code(iced::keyboard::key::Code::KeyD),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

fn named_key(k: iced::keyboard::key::Named) -> iced::Event {
    iced::Event::Keyboard(KeyEvent::KeyPressed {
        key: Key::Named(k),
        modified_key: Key::Named(k),
        physical_key: Physical::Code(iced::keyboard::key::Code::PageDown),
        location: iced::keyboard::Location::Standard,
        modifiers: Modifiers::default(),
        text: None,
        repeat: false,
    })
}

#[test]
fn reader_shell_shortcuts_apply_and_respect_modals() {
    let (mut app, _task) = App::test_new();
    app.model.screen = crate::model::Screen::Reading;
    app.model.format = crate::model::Format::Pdf;
    let _ = app.update(Msg::WindowResized(iced::Size::new(1080.0, 760.0)));
    let sidebar = app.sidebar_presentation();
    let f9 = named_key(iced::keyboard::key::Named::F9);
    let _ = app.update(classify_event(&f9).expect("F9 binding"));
    assert_ne!(app.sidebar_presentation(), sidebar);
    let _ = app.update(classify_event(&char_key("2", Modifiers::CTRL)).expect("Ctrl+2 binding"));
    assert_eq!(app.model.zoom.mode(), crate::viewport::ZoomMode::FitPage);
    app.model.overlay = Some(Overlay::Shortcuts);
    let _ = app.update(classify_event(&char_key("1", Modifiers::CTRL)).expect("Ctrl+1 binding"));
    assert_eq!(app.model.zoom.mode(), crate::viewport::ZoomMode::FitPage);
    let sidebar = app.sidebar_presentation();
    let _ = app.update(classify_event(&f9).expect("F9 binding"));
    assert_eq!(app.sidebar_presentation(), sidebar);
    let _ = app.update(classify_event(&char_key("i", Modifiers::CTRL)).expect("Ctrl+I binding"));
    assert_eq!(app.model.overlay, Some(Overlay::Shortcuts));
}
