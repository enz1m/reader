use super::*;
use enzui_iced_core::collect::visibility_probe::NodeVisibility;
use iced::advanced::widget::Operation;
use iced_test::runtime::Action;

pub(crate) struct PaintedFrame {
    pub(crate) nodes: Vec<CollectedNode>,
    pub(crate) pixels: Vec<u8>,
    pub(crate) background: iced::Color,
    size: iced::Size<u32>,
}

pub(crate) fn drive_task(task: iced::Task<Msg>) -> (Option<Box<dyn Operation>>, Vec<Msg>) {
    use iced_test::futures::futures::StreamExt;

    let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
        return (None, Vec::new());
    };
    iced_test::futures::futures::executor::block_on(async move {
        let mut operation = None;
        let mut messages = Vec::new();
        while let Some(action) = stream.next().await {
            match action {
                Action::Widget(op) if operation.is_none() => operation = Some(op),
                Action::Output(msg) => messages.push(msg),
                Action::Image(iced_test::runtime::image::Action::Allocate(_, sender)) => {
                    let _ = sender.send(Err(iced::widget::image::Error::Unsupported));
                }
                _ => {}
            }
        }
        (operation, messages)
    })
}

fn widget_operation(task: iced::Task<Msg>) -> Box<dyn Operation> {
    drive_task(task)
        .0
        .expect("GoToPage task must contain the pages-scroll operation")
}

pub(crate) fn painted_frame(app: &App, operation: Option<Box<dyn Operation>>) -> PaintedFrame {
    use iced::advanced::renderer::Headless;

    let theme = app.theme();
    let base = <iced::Theme as iced::advanced::graphics::core::theme::Base>::base(&theme);
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let _allocations = super::load_resident_page_images(app, &renderer);
    let size = iced::Size::new(app.window_width, app.window_height);
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        App::view(app),
        size,
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    if let Some(mut operation) = operation {
        ui.operate(&renderer, operation.as_mut());
        // Relayout turns the scrolled widget state into translated page bounds; the
        // pre-relayout tree still describes page 1.
        ui = ui.relayout(size, &mut renderer);
    }
    // Scrollable drawing uses the offsets settled by its redraw event.
    let mut messages = iced::advanced::shell::Bus::new();
    ui.update(
        &iced::window::Headless,
        &iced::advanced::graphics::core::shell::Waker::noop(),
        &[iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ))],
        iced::mouse::Cursor::Unavailable,
        &mut renderer,
        &mut messages,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let op: &mut dyn Operation = &mut collector;
        op
    });
    ui.draw(
        &mut renderer,
        &theme,
        &iced::advanced::renderer::Style {
            text_color: base.text_color,
        },
        iced::advanced::mouse::Cursor::Unavailable,
    );
    PaintedFrame {
        nodes: collector.into_collected(),
        pixels: renderer.screenshot(
            iced::Size::new(size.width as u32, size.height as u32),
            1.0,
            base.background_color,
        ),
        background: base.background_color,
        size: iced::Size::new(size.width as u32, size.height as u32),
    }
}

pub(crate) fn page(frame: &PaintedFrame, number: usize) -> &CollectedNode {
    let label = format!("Document page {number}");
    frame
        .nodes
        .iter()
        .find(|node| {
            node.semantics.role == Role::Group
                && node.semantics.label.as_deref() == Some(label.as_str())
        })
        .unwrap_or_else(|| panic!("real view must publish {label:?}"))
}

pub(crate) fn probe(frame: &PaintedFrame) -> NodeVisibility<'_> {
    NodeVisibility::new(
        &frame.pixels,
        frame.size.width,
        frame.size.height,
        frame.background,
    )
}

#[test]
fn pager_endpoint_controls_route_and_scroll_to_the_document_ends() {
    let mut app = app_reading_pdf();
    assert_eq!(
        app.model().sidebar_tab,
        crate::model::SidebarTab::Thumbnails
    );
    let nodes = painted_frame(&app, None).nodes;
    let tab = |name| {
        nodes
            .iter()
            .find(|node| {
                node.semantics.role == Role::Tab && node.semantics.label.as_deref() == Some(name)
            })
            .expect("document navigation tab")
    };
    assert!(tab("Pages").bounds.x < tab("Contents").bounds.x);
    let raster = app.model().pages.get(0).expect("PDF raster").clone();
    // Resident identical rasters isolate navigation from asynchronous raster delivery.
    for index in 0..app.model().page_count {
        let mut page = raster.clone();
        page.index = index;
        app.model_mut().pages.insert(page, 0..=4);
    }
    for fit_page in [false, true] {
        let _ = app.update(if fit_page {
            Msg::ZoomFitPage
        } else {
            Msg::ZoomFitWidth
        });
        for (id, target) in [("reader.page.last", 4), ("reader.page.first", 0)] {
            let mut simulator = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(DESKTOP_W, DESKTOP_H),
                app.view(),
            );
            simulator
                .click(iced_test::selector::id(iced::widget::Id::new(id)))
                .expect("endpoint control");
            let message = simulator
                .into_messages()
                .find(|message| matches!(message, Msg::GoToPage(_)))
                .expect("routed page navigation");
            let operation = widget_operation(app.update(message));
            assert_eq!(app.model().current_page, target);
            let frame = painted_frame(&app, Some(operation));
            assert!(
                probe(&frame)
                    .check(page(&frame, target + 1).bounds)
                    .is_visible(),
                "endpoint page is not painted"
            );
            if fit_page && target + 1 == app.model().page_count {
                let paper = page(&frame, target + 1).bounds;
                let pager = frame
                    .nodes
                    .iter()
                    .find(|node| node.id == Some(iced::widget::Id::new("reader.page.last")))
                    .expect("native endpoint button")
                    .bounds;
                assert!(
                    paper.y + paper.height + 8.0 <= pager.y,
                    "last page bottom {} overlaps floating pager at {}",
                    paper.y + paper.height,
                    pager.y
                );
            }
        }
    }
}
