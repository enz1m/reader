use std::time::Instant;

use super::held_owner::{PoolHold, held_app};
use super::task_observer::observe;
use crate::app::{App, CloseState, PreviewSource};
use crate::message::Msg;
use enzui_iced_core::blocking_task::{BlockingTaskDispatcher, BlockingTaskOwner};

fn app_with_held_owner() -> (App, PoolHold, BlockingTaskDispatcher, BlockingTaskOwner) {
    held_app(App::test_new().0)
}

#[test]
fn close_drains_the_requested_window_and_requires_retry_after_timeout() {
    {
        let (mut app, hold, dispatcher, _old) = app_with_held_owner();
        let window = iced::window::Id::unique();
        let other_window = iced::window::Id::unique();
        app.push_preview_pending_for_test("/lib/held.pdf".into(), PreviewSource::FirstPageRaster);

        observe(app.update(Msg::CloseWindow(Some(window)))).assert_quiet();
        assert!(
            matches!(app.close_state_for_test(), CloseState::Draining { window: Some(id), .. } if *id == window)
        );
        assert!(app.preview_pending_snapshot().is_empty());
        assert!(!app.busy_retry_is_armed());
        assert_eq!(dispatcher.snapshot().live(), 2);

        observe(app.update(Msg::CloseWindow(Some(other_window)))).assert_quiet();
        assert!(
            matches!(app.close_state_for_test(), CloseState::Draining { window: Some(id), .. } if *id == window)
        );
        hold.finish(&dispatcher);

        let actions = observe(app.update(Msg::CloseDrainTick));
        assert_eq!(actions.closes, vec![window]);
        assert!(actions.messages.is_empty());
        assert_eq!(actions.other_actions, 0);
        assert!(
            matches!(app.close_state_for_test(), CloseState::Committed { window: Some(id) } if *id == window)
        );
        observe(app.update(Msg::CloseDrainTick)).assert_quiet();
    }
    {
        let (mut app, hold, dispatcher, _old) = app_with_held_owner();
        let window = iced::window::Id::unique();
        observe(app.update(Msg::CloseWindow(Some(window)))).assert_quiet();
        assert!(
            matches!(app.close_state_for_test(), CloseState::Draining { window: Some(id), .. } if *id == window)
        );
        app.force_close_state_for_test(CloseState::Draining {
            window: Some(window),
            deadline: Instant::now(),
        });

        observe(app.update(Msg::CloseDrainTick)).assert_quiet();
        let summary = match app.close_state_for_test() {
            CloseState::Vetoed {
                window: Some(id),
                summary,
            } => {
                assert_eq!(*id, window);
                assert!(!summary.is_empty());
                summary.clone()
            }
            state => panic!("expired held drain must veto close, got {state:?}"),
        };
        assert_eq!(app.model().error.as_deref(), Some(summary.as_str()));
        observe(app.update(Msg::CloseWindow(Some(iced::window::Id::unique())))).assert_quiet();
        assert!(
            matches!(app.close_state_for_test(), CloseState::Vetoed { window: Some(id), .. } if *id == window)
        );

        observe(app.update(Msg::RetryClose)).assert_quiet();
        assert!(
            matches!(app.close_state_for_test(), CloseState::Draining { window: Some(id), .. } if *id == window)
        );
        assert_eq!(app.model().error.as_deref(), Some(summary.as_str()));
        assert_eq!(dispatcher.snapshot().live(), 2);
        hold.finish(&dispatcher);
        let actions = observe(app.update(Msg::CloseDrainTick));
        assert_eq!(actions.closes, vec![window]);
        assert!(actions.messages.is_empty());
        assert_eq!(actions.other_actions, 0);
    }
}
