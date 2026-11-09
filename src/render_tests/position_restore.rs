use super::pdf_page_navigation::{drive_task, page, painted_frame, probe};
use super::*;
use enzui_iced_core::collect::visibility_probe::VisibilityVerdict;

#[test]
fn saved_pdf_position_scrolls_the_real_view_after_async_open() {
    let path = std::path::PathBuf::from("tests/fixtures/test-multipage.pdf");
    let mut state = crate::plumbing::persistence::AppState::default();
    crate::plumbing::persistence::set_progress(
        &mut state,
        &path.to_string_lossy(),
        crate::plumbing::persistence::ProgressRecord {
            page: 1,
            total: 5,
            format: "pdf".into(),
            ..Default::default()
        },
    );
    let mut app = app();
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    app.seed_app_state_for_test(state);
    let mut pending = std::collections::VecDeque::from([app.update(Msg::OpenPath(path))]);
    let mut restores = Vec::new();
    let mut turns = 0;
    while let Some(task) = pending.pop_front() {
        turns += 1;
        assert!(turns <= 32, "document startup did not settle");
        let (operation, messages) = drive_task(task);
        assert!(
            operation.is_none(),
            "scroll operation preceded its restore request"
        );
        for message in messages {
            match message {
                Msg::BeginPositionRestore { generation } => restores.push(generation),
                message => pending.push_back(app.update(message)),
            }
        }
    }
    assert_eq!(restores.len(), 1);
    assert_eq!(app.model().page_count, 5);
    assert_eq!(app.model().current_page, 1);
    assert!(app.model().error.is_none(), "{:?}", app.model().error);

    let initial = painted_frame(&app, None);
    assert_eq!(initial.pixels, painted_frame(&app, None).pixels);
    assert!(probe(&initial).check(page(&initial, 1).bounds).is_visible());
    assert_eq!(
        probe(&initial).check(page(&initial, 2).bounds),
        VisibilityVerdict::FullyClipped
    );

    let generation = restores[0];
    let (operation, messages) = drive_task(app.update(Msg::BeginPositionRestore { generation }));
    assert!(messages.is_empty());
    let restored = painted_frame(
        &app,
        Some(operation.expect("startup must emit its scroll operation")),
    );
    let target = page(&restored, 2).bounds;
    assert!(probe(&restored).check(target).is_visible());
    assert!(
        target.y >= 0.0 && target.y <= page(&initial, 1).bounds.y,
        "restored page is not at the viewport top: {target:?}"
    );
    let first = page(&restored, 1).bounds;
    assert!(first.y + first.height <= target.y);

    let (repeat, messages) = drive_task(app.update(Msg::BeginPositionRestore { generation }));
    assert!(messages.is_empty());
    let repeated = painted_frame(&app, Some(repeat.unwrap()));
    assert_eq!(restored.pixels, repeated.pixels);
}
