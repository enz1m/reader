//! Rendered toast assertions shared by application workflow tests.

use crate::app::App;
use crate::model::TOAST_ID_PREFIX;

use enzui_iced_core::collect::{CollectedNode, SemanticsCollector};

const WIDTH: f32 = 1400.0;
const HEIGHT: f32 = 900.0;

pub(crate) fn nodes(app: &App) -> Vec<CollectedNode> {
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
        iced::Size::new(WIDTH, HEIGHT),
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

pub(crate) fn live_region(app: &App) -> Option<CollectedNode> {
    let wanted = iced::advanced::widget::Id::from(format!("{TOAST_ID_PREFIX}-live-region"));
    let matches: Vec<CollectedNode> = nodes(app)
        .into_iter()
        .filter(|node| node.id.as_ref() == Some(&wanted))
        .collect();
    assert!(
        matches.len() <= 1,
        "the transient host must publish at most one live region, got {}",
        matches.len(),
    );
    matches.into_iter().next()
}

pub(crate) fn announced(app: &App) -> Option<String> {
    live_region(app).and_then(|node| node.semantics.label)
}
