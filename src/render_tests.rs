//! Reading and editing workflows through rendered application views.

use crate::app::App;
use crate::message::Msg;

use enzui_iced_core::collect::{CollectedNode, SemanticsCollector};
use enzui_iced_core::semantics::Role;

// Wide enough that `enz_navigation_split_view` lays the sidebar inline and the titlebar
// shows its full control set.
const DESKTOP_W: f32 = 1400.0;
const DESKTOP_H: f32 = 900.0;

// Hold GPU image allocations until the capture completes.
fn load_resident_page_images(
    app: &App,
    renderer: &iced_test::renderer::Renderer,
) -> Vec<iced::widget::image::Allocation> {
    use iced::advanced::image::Renderer as _;
    app.model()
        .pages
        .resident()
        .into_iter()
        .map(|index| {
            renderer
                .load_image(&app.model().pages.get(index).expect("resident page").handle)
                .expect("load page image before capture")
        })
        .collect()
}

fn nodes(app: &App) -> Vec<CollectedNode> {
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
        App::view(app),
        iced::Size::new(DESKTOP_W, DESKTOP_H),
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let op: &mut dyn Operation = &mut collector;
        op
    });
    collector.into_collected()
}

fn role_count(nodes: &[CollectedNode], role: Role, needle: &str) -> usize {
    nodes
        .iter()
        .filter(|n| n.semantics.role == role && n.semantics.label.as_deref() == Some(needle))
        .count()
}

// Annotation roots are unique per call because production sidecars otherwise write beside shared fixtures.
fn fresh_annotation_root() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "reader-app-render-annotations-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        n
    ));
    std::fs::create_dir_all(&dir).expect("annotation sandbox dir must be creatable");
    dir
}

fn app() -> App {
    crate::plumbing::persistence::set_thread_annotations_root(Some(fresh_annotation_root()));
    let mut app = App::test_new_settled();
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    app
}

fn app_reading_epub() -> App {
    let path = std::path::PathBuf::from("tests/fixtures/quiet-coast.epub");
    crate::plumbing::epub_support::load_epub(&path.to_string_lossy())
        .expect("the original EPUB fixture must load");
    let mut app = app();
    app.open_settled_for_test(path);
    // Keeps the sidebar inline regardless of `App::new`'s initial size.
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    app
}

fn app_reading_pdf() -> App {
    let path = std::path::PathBuf::from("tests/fixtures/test-multipage.pdf");
    let mut app = app();
    // Annotation writes are refused until the native annotation scan settles.
    app.open_settled_for_test(path.clone());
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    app
}

fn app_searching(query: &str) -> App {
    let mut app = app_reading_epub();
    let _ = app.update(Msg::OpenSearch);
    app.run_search_to_completion(query);
    app
}

#[cfg(test)]
mod paginated_reading;
#[cfg(test)]
mod pdf_page_navigation;

#[cfg(test)]
mod pdf_selection_gesture;
mod position_restore;
mod search_tests;

fn frame_of_sized(app: &App, w: f32, h: f32) -> ((Vec<u8>, u32, Vec<CollectedNode>), f32) {
    use iced::advanced::renderer::Headless;
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let _allocations = load_resident_page_images(app, &renderer);
    let itheme = iced::Theme::Dark;
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        App::view(app),
        iced::Size::new(w, h),
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let base = <iced::Theme as iced::advanced::graphics::core::theme::Base>::base(&itheme);
    let mut collector = enzui_iced_core::collect::SemanticsCollector::new();
    ui.operate(
        &renderer,
        &mut collector as &mut dyn iced::advanced::widget::Operation,
    );
    let nodes = collector.into_collected();
    ui.draw(
        &mut renderer,
        &itheme,
        &iced::advanced::renderer::Style {
            text_color: base.text_color,
        },
        iced::advanced::mouse::Cursor::Unavailable,
    );
    let wi = w as u32;
    let buf = renderer.screenshot(iced::Size::new(wi, h as u32), 1.0, base.background_color);
    ((buf, wi, nodes), h)
}
