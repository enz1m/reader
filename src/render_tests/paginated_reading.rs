use super::*;
use crate::model::EpubViewMode;

const W: f32 = 1400.0;
const H: f32 = 900.0;
fn continuous() -> App {
    let mut app = app_reading_epub();
    let _ = app.update(Msg::GoToChapter(5));
    let _ = app.update(Msg::WindowResized(iced::Size::new(W, H)));
    app
}

fn paginated() -> App {
    let mut app = continuous();
    let _ = app.update(Msg::ToggleEpubViewMode);
    assert_eq!(app.model().epub_view_mode, EpubViewMode::Paginated);
    assert!(
        app.model().epub_page_count() > 1,
        "the fixture chapter must span several pages for these gates to \
         mean anything, got {}",
        app.model().epub_page_count()
    );
    app
}

fn painted_text(app: &App) -> String {
    app.model()
        .epub_page_content
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn effect_count(task: iced::Task<Msg>) -> usize {
    match iced_test::runtime::task::into_stream(task) {
        Some(stream) => iced_test::futures::futures::executor::block_on(async move {
            use iced_test::futures::futures::StreamExt;
            stream.count().await
        }),
        None => 0,
    }
}

#[test]
fn page_turns_return_to_the_same_passage() {
    let mut app = paginated();
    let first_page_text = painted_text(&app);

    let _ = app.update(Msg::NextPage);
    assert_ne!(
        painted_text(&app),
        first_page_text,
        "the second page must carry different prose"
    );
    assert_eq!(
        app.model().epub_page_index(),
        1,
        "next advances exactly one page"
    );

    let _ = app.update(Msg::PrevPage);
    assert_eq!(painted_text(&app), first_page_text);
}

#[test]
fn the_mode_toggle_carries_the_reading_position_both_ways() {
    let mut app = continuous();
    let generation = app.model().position_restore_generation;
    let _ = app.update(Msg::EpubScrolled {
        generation,
        fraction: 0.66,
        offset: 0.0,
        viewport_height: 0.0,
    });
    let _ = app.update(Msg::ToggleEpubViewMode);
    let page = app.model().epub_page_index();
    assert!(
        page > 0,
        "paginating from two thirds down must not land on page one"
    );
    let offset = app.model().epub_page_offset;
    assert!(offset > 0);

    // The returned task must carry the `snap_to` effect; `Task::none()` would repaint from the
    // first line.
    let task = app.update(Msg::ToggleEpubViewMode);
    assert!(
        effect_count(task) > 0,
        "returning to the continuous surface must publish a scroll effect"
    );
    assert_eq!(app.model().epub_view_mode, EpubViewMode::Scrolled);
    assert!(
        (app.model().epub_scroll_fraction - 0.66).abs() < 0.05,
        "switching back must restore the scroll position, got {}",
        app.model().epub_scroll_fraction
    );
}
