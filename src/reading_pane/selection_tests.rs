//! Prose selection through the app's own `chapter_view`, so the `enz_prose_semantics` wrap
//! and the selection scope are both exercised.

use super::*;

use iced::Event;
use iced_test::Simulator;

#[derive(Debug, Clone)]
enum TMsg {
    Copied(String),
    SelectionChanged,
    Link,
}

fn colors() -> ReadingThemeColors {
    crate::model::ReadingTheme::Light.colors()
}

fn para(text: &str) -> ReadingParagraph {
    ReadingParagraph {
        chapter: 0,
        para_index: 0,
        text: text.to_string(),
        spans: vec![],
        char_start: 0,
        char_end: text.chars().count(),
    }
}

fn ctrl() -> iced::keyboard::Modifiers {
    iced::keyboard::Modifiers::COMMAND
}

fn key_char(c: &str, modifiers: iced::keyboard::Modifiers) -> Event {
    let k = iced::keyboard::Key::Character(c.into());
    Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: k.clone(),
        modified_key: k,
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

const FIRST_PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog.";

fn focus_and_select_all(ui: &mut Simulator<'_, TMsg>) {
    ui.click(FIRST_PARAGRAPH)
        .expect("the first paragraph must be on screen to focus the scope");
    let _ = ui.simulate([key_char("a", ctrl())]);
}

fn two_para_chapter() -> iced::Element<'static, TMsg> {
    let paras = vec![
        para(FIRST_PARAGRAPH),
        para("Pack my box with five dozen liquor jugs."),
    ];
    let paras: &'static [ReadingParagraph] = Box::leak(paras.into_boxed_slice());
    chapter_view(
        None,
        paras,
        &colors(),
        16.0,
        1.5,
        600.0,
        |id: &str| id.to_string(),
        |_| TMsg::Link,
        TMsg::Copied,
        |_| TMsg::SelectionChanged,
        0,
        None,
    )
}

#[test]
fn cross_paragraph_selection_works_through_chapter_view() {
    let mut ui = Simulator::new(two_para_chapter());
    focus_and_select_all(&mut ui);
    let _ = ui.simulate([key_char("c", ctrl())]);
    let copied: Vec<String> = ui
        .into_messages()
        .filter_map(|m| match m {
            TMsg::Copied(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(
        !copied.is_empty(),
        "Ctrl+A then Ctrl+C must copy text from the chapter view"
    );
    let text = &copied[0];
    assert!(
        text.contains("quick brown fox") && text.contains("liquor jugs"),
        "selection must span both paragraphs; got: {text:?}"
    );
}
