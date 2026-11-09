//! Captured menu targets, confirmation and live document authority.

use crate::app::App;
use crate::context_menu::{ContextTarget, message_for};
use crate::message::Msg;
use crate::model::SidebarTab;

pub(crate) fn menu_messages(app: &App, label: &str) -> Vec<Msg> {
    let mut sim = iced_test::Simulator::with_size(
        Default::default(),
        iced::Size::new(1400.0, 900.0),
        app.view(),
    );
    sim.click(label).expect("menu command is clickable");
    sim.into_messages().collect()
}

const DESKTOP_W: f32 = 1400.0;
const DESKTOP_H: f32 = 900.0;

fn fresh_annotation_root() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "reader-app-ctxmenu-annotations-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        n
    ));
    std::fs::create_dir_all(&dir).expect("annotation sandbox dir must be creatable");
    dir
}

fn app_with_annotations() -> App {
    crate::plumbing::persistence::set_thread_annotations_root(Some(fresh_annotation_root()));
    let (mut app, boot) = App::test_new();
    // Annotations stay unavailable until startup recovery settles on the blocking owner.
    for message in crate::app::drain_task(boot) {
        app.drive_pdf_mutation(message);
    }
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));

    let path = std::path::PathBuf::from("tests/fixtures/quiet-coast.epub");
    assert!(
        crate::plumbing::epub_support::load_epub(&path.to_string_lossy()).is_ok(),
        "the original EPUB fixture must load"
    );
    // Notes seeded before the annotation restore lands would be replaced by the empty set.
    app.drive_pdf_mutation(Msg::OpenPath(path));
    assert!(
        !app.model.annotations_loading,
        "the annotation restore landed"
    );
    for text in NOTE_TEXTS {
        let _ = app.seed_note_for_test(0, text);
    }
    let _ = app.update(Msg::SelectSidebarTab(SidebarTab::Annotations));
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    app
}

// Distinct bodies so each rendered row is found by its own text.
const NOTE_TEXTS: [&str; 3] = [
    "Zither remark alpha",
    "Zither remark beta",
    "Zither remark gamma",
];

fn annotation_ids(app: &App) -> Vec<u64> {
    app.model()
        .annotations
        .annotations
        .iter()
        .map(|a| a.id)
        .collect()
}

// Committed fixtures, so the `Open` row is offered; distinct stamps fix the rendered order.
#[test]
fn context_actions_apply_to_the_invoked_annotation_or_page() {
    {
        let mut app = app_with_annotations();
        let ids = annotation_ids(&app);
        let _ = app.update(Msg::BeginEditAnnotation(ids[0]));

        let _ = app.update(Msg::OpenContextMenu(
            ContextTarget::Annotation(ids[2]),
            iced::Point::new(300.0, 400.0),
        ));
        assert!(
            app.model().context_menu.is_some(),
            "the menu must be open before the row is activated"
        );

        let stale = menu_messages(&app, "Delete");
        app.model.pdf_annotation_generation += 1;
        for message in stale {
            let _ = app.update(message);
        }
        assert!(app.model.pending_page_action.is_none());
        assert_eq!(annotation_ids(&app), ids);
        let _ = app.update(Msg::OpenContextMenu(
            ContextTarget::Annotation(ids[2]),
            iced::Point::new(300.0, 400.0),
        ));
        for message in menu_messages(&app, "Delete") {
            let _ = app.update(message);
        }

        assert_eq!(
            annotation_ids(&app),
            ids,
            "Delete asks for confirmation before mutating the sidecar"
        );
        let _ = app.update(Msg::PageConfirmEvent(
            enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent::Resolved(
                enzui_iced_core::modal::DismissCause::DefaultAction,
            ),
        ));
        let left = annotation_ids(&app);
        assert_eq!(
            left,
            vec![ids[0], ids[1]],
            "Delete must remove the RIGHT-CLICKED annotation ({}), leaving the \
         other two. Removing the edited one ({}) is the mistarget",
            ids[2],
            ids[0]
        );
        assert_eq!(
            app.model().editing_annotation.as_ref().map(|(i, _)| *i),
            Some(ids[0]),
            "the inline edit on a different annotation must be untouched"
        );
        assert!(
            app.model().context_menu.is_none(),
            "activating a row closes the menu"
        );
    }
    {
        use crate::context_menu::ContextAction as A;
        let (mut app, dir) = app_on_pdf();
        let target = ContextTarget::Page(1);
        assert_eq!(
            page_actions(&app, 1),
            [
                A::AddBookmark,
                A::RotatePageLeft,
                A::RotatePageRight,
                A::ExtractPage,
                A::ArrangePages
            ]
        );

        let toggle = message_for(&target, A::AddBookmark, &app.model).unwrap();
        assert!(matches!(toggle, Msg::ToggleBookmark(1)));
        let _ = app.update(toggle);
        assert_eq!(
            page_actions(&app, 1)[0],
            A::RemoveBookmark,
            "a bookmarked page offers to remove it"
        );

        let entry = app
            .model
            .arrangement
            .entries()
            .iter()
            .find(|entry| entry.source == crate::pages::PageSource::Original(1))
            .unwrap()
            .id;
        let rotate = message_for(&target, A::RotatePageRight, &app.model).unwrap();
        assert!(matches!(rotate, Msg::RotatePageRight(id) if id == entry));
        let before = crate::view::reading::effective_page_rotation(&app.model, 1);
        let _ = app.update(rotate);
        assert_ne!(
            crate::view::reading::effective_page_rotation(&app.model, 1),
            before,
            "the right-clicked page turns"
        );
        assert_eq!(
            crate::view::reading::effective_page_rotation(&app.model, 0),
            before,
            "and no other page does"
        );
        assert!(matches!(
            message_for(&target, A::ExtractPage, &app.model),
            Some(Msg::RequestExtractPage(id)) if id == entry
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn app_on_pdf() -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "reader-page-menu-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("document.pdf");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf"),
        &path,
    )
    .unwrap();
    let render = crate::pdf::render_pdf(&path.to_string_lossy(), None).unwrap();
    let (mut app, _) = App::test_new();
    app.model.file_path = Some(path.clone());
    app.model.file_name = "document.pdf".to_string();
    app.model.screen = crate::model::Screen::Reading;
    let _ = app.finish_pages(
        path,
        render.pages,
        render.total_pages,
        render.extents,
        render.text,
    );
    app.window_width = DESKTOP_W;
    app.window_height = DESKTOP_H;
    (app, dir)
}

fn page_actions(app: &App, page: usize) -> Vec<crate::context_menu::ContextAction> {
    crate::context_menu::items_for(&ContextTarget::Page(page), &app.model)
}
