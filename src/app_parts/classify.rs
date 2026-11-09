//! Event classification and blocking owners.

use super::*;

/// Map window and keyboard events to commands; document-focus checks remain in the reducer.
pub fn classify_event(event: &iced::Event) -> Option<Msg> {
    // Assumes nothing captured the key; capture-guard assertions use `classify_event_in`,
    // which the real subscription calls.
    classify_event_in(event, iced::event::Status::Ignored)
}

/// Map events while respecting widget capture; uncaptured document keys still require reducer
/// focus checks.
pub fn classify_event_in(event: &iced::Event, status: iced::event::Status) -> Option<Msg> {
    if let Some(traversal) = enzui_iced_core::focus::traversal_from_event(event, status) {
        return Some(match traversal {
            Traversal::Next => Msg::FocusNext,
            Traversal::Previous => Msg::FocusPrevious,
        });
    }

    let captured = matches!(status, iced::event::Status::Captured);
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => Some(Msg::WindowResized(*size)),
        iced::Event::Window(iced::window::Event::Closed) => Some(Msg::MainWindowClosed),
        // F1 opens help. Like Tab it is a navigation key, kept out of the `shortcuts`
        // table of document commands.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F1),
            ..
        }) => Some(Msg::OpenShortcuts),
        // Every command chord resolves against the one binding table through
        // `Shortcut::matches_event`.
        iced::Event::Keyboard(key_event @ iced::keyboard::Event::KeyPressed { .. })
            if !captured =>
        {
            let command = crate::shortcuts::dispatch(key_event)?;
            // Document commands need the model to know whether the document is focused,
            // so they go to the reducer as a gated message rather than `Msg::NextPage`,
            // whose contract is the clamp.
            if crate::shortcuts::is_document_command(command) {
                Some(Msg::DocumentKeyCommand(command))
            } else {
                Some(Msg::ShellKeyCommand(command))
            }
        }
        _ => None,
    }
}

// The recents shelf shows at most this many "Continue reading" cards; the rest
// of the store is reachable only through the Library browser.
const RECENTS_LIMIT: usize = 24;

// Paths probed per availability batch, bounding per-batch work and payload; successive
// batches cover the whole store, newest first.
pub(crate) const AVAILABILITY_BATCH: usize = 128;

// Unprobed paths remain provisionally available; asynchronous projection refresh preserves
// scroll position.
pub(crate) fn refresh_projection(model: &mut Model) {
    let library = persistence::recent_books(
        &model.app_state,
        usize::MAX,
        |path| model.bookmark_counts.get(path).copied().unwrap_or(0),
        |path| model.availability.get(path).copied().unwrap_or(true),
    );
    model.recents = library.iter().take(RECENTS_LIMIT).cloned().collect();
    model.library = library;
    // A full rebuild invalidates an in-flight scan's pinned order.
    model.order_revision = model.order_revision.wrapping_add(1);
    model.rebuild_library_projection();
}

pub(crate) fn probe_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

// Build one bounded blocking-task owner and its clonable dispatcher.
//
// At most one owner per `App`. A refused thread set degrades to the `unavailable` owner,
// whose submissions settle immediately as a terminal error without spawning a thread.
pub(crate) fn new_blocking_owner() -> (
    enzui_iced_core::blocking_task::BlockingTaskOwner,
    enzui_iced_core::blocking_task::BlockingTaskDispatcher,
) {
    let owner = enzui_iced_core::blocking_task::BlockingTaskOwner::with_defaults("reader-blocking")
        .unwrap_or_else(|_| {
            enzui_iced_core::blocking_task::BlockingTaskOwner::unavailable(
                enzui_iced_core::blocking_task::DEFAULT_QUEUE_CAPACITY,
            )
        });
    let dispatcher = owner.dispatcher();
    (owner, dispatcher)
}
