//! Persistence flush writes.

use super::*;

#[test]
fn deferred_persistence_flushes_atomically_and_keeps_the_latest_snapshot() {
    {
        let dir = tmpdir("a-plus-write");
        let resolver = crate::app::test_xdg_resolver(&dir);

        let mut app = heavy_app(&dir);
        app.enable_persist(resolver.clone());
        let state = state_file(&resolver);
        let _ = std::fs::remove_file(&state);

        for _ in 0..10 {
            let _ = app.update(Msg::FontSizeUp);
            let _ = app.update(Msg::FontSizeDown);
        }
        assert!(
            !state.exists(),
            "twenty font-size clicks wrote {state:?} — the JSON serialise + atomic \
         write is back on the click path"
        );
        assert!(
            app.persistence_pending(),
            "the write must be PENDING, not skipped: a deferred write that is \
         never owed is a lost reading position"
        );

        let serializations = persistence::serialization_count_on_current_thread();
        let task = app.update(Msg::FlushPersistence);
        assert_eq!(
            persistence::serialization_count_on_current_thread(),
            serializations,
            "flushing must serialize on the worker"
        );
        for message in crate::app::drain_task(task) {
            let _ = app.update(message);
        }
        assert!(
            state.exists(),
            "the returned flush task must write {state:?}; if it does not, the \
         assertion above is satisfied by persistence being broken outright"
        );
        assert!(!app.persistence_pending(), "the flush clears the debt");
        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        fn state(page: usize) -> AppState {
            let mut state = persistence_state(1);
            state.files.values_mut().next().unwrap().page = page;
            state
        }
        fn page_on_disk(resolver: &enzui_iced_core::persist::XdgResolver) -> usize {
            let text = std::fs::read_to_string(state_file(resolver)).expect("state file");
            let state: AppState = serde_json::from_str(&text).expect("complete, untorn JSON");
            state.files.values().next().unwrap().page
        }

        let dir = tmpdir("ordered-writes");
        let resolver = crate::app::test_xdg_resolver(&dir);

        // Reverse polling order: newer writes first, then older attempts to land.
        let queue = persistence::PersistenceQueue::default();
        let old = queue.register(persistence::LIBRARY_STATE_DESTINATION);
        let new = queue.register(persistence::LIBRARY_STATE_DESTINATION);
        let (new_done_tx, new_done_rx) = std::sync::mpsc::channel();
        let new_thread = {
            let queue = queue.clone();
            let resolver = resolver.clone();
            std::thread::spawn(move || {
                let outcome = queue.execute(&new, || {
                    persistence::save_state_with_resolver(&state(2), &resolver)
                });
                assert!(
                    matches!(outcome, persistence::PersistenceOutcome::Saved),
                    "the newer scheduled write must reach disk: {outcome:?}"
                );
                new_done_tx.send(()).unwrap();
            })
        };
        let old_thread = {
            let queue = queue.clone();
            let resolver = resolver.clone();
            std::thread::spawn(move || {
                new_done_rx.recv().unwrap();
                let outcome = queue.execute(&old, || {
                    persistence::save_state_with_resolver(&state(1), &resolver)
                });
                assert!(
                    matches!(outcome, persistence::PersistenceOutcome::Superseded),
                    "a superseded task must skip its write: {outcome:?}"
                );
            })
        };
        new_thread.join().unwrap();
        old_thread.join().unwrap();
        assert_eq!(page_on_disk(&resolver), 2);

        // Older is already writing when newer is scheduled. Newer waits on the
        // writer lock and must land last.
        let queue = persistence::PersistenceQueue::default();
        let old = queue.register(persistence::LIBRARY_STATE_DESTINATION);
        let (old_started_tx, old_started_rx) = std::sync::mpsc::channel();
        let (release_old_tx, release_old_rx) = std::sync::mpsc::channel();
        let old_thread = {
            let queue = queue.clone();
            let resolver = resolver.clone();
            std::thread::spawn(move || {
                let outcome = queue.execute(&old, || {
                    old_started_tx.send(()).unwrap();
                    release_old_rx.recv().unwrap();
                    persistence::save_state_with_resolver(&state(3), &resolver)
                });
                assert!(
                    matches!(outcome, persistence::PersistenceOutcome::Saved),
                    "the older write, already inside the writer lock, must land: {outcome:?}"
                );
            })
        };
        old_started_rx.recv().unwrap();
        let new = queue.register(persistence::LIBRARY_STATE_DESTINATION);
        let new_thread = {
            let queue = queue.clone();
            let resolver = resolver.clone();
            std::thread::spawn(move || {
                let outcome = queue.execute(&new, || {
                    persistence::save_state_with_resolver(&state(4), &resolver)
                });
                assert!(
                    matches!(outcome, persistence::PersistenceOutcome::Saved),
                    "the newer write must reach disk after the writer lock frees: {outcome:?}"
                );
            })
        };
        release_old_tx.send(()).unwrap();
        old_thread.join().unwrap();
        new_thread.join().unwrap();
        assert_eq!(page_on_disk(&resolver), 4);
        let _ = std::fs::remove_dir_all(dir);
    }
}
