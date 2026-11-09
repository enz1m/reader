//! Bounded off-thread availability coverage and reducer responsiveness.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::app::{App, MetadataBackend};
use crate::message::Msg;
use crate::plumbing::persistence::{AppState, ProgressRecord, set_progress};

const WATCHDOG: Duration = Duration::from_secs(20);

// Blocks every probe on a shared gate until released, counting entries on a
// cross-thread atomic. Entries while wedged equal the number of concurrently
// admitted scan threads.
struct BarrierBackend {
    entered: Arc<AtomicU64>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    answer: bool,
}

impl MetadataBackend for BarrierBackend {
    fn exists(&self, _path: &str) -> bool {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (lock, cv) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = cv.wait(released).unwrap();
        }
        self.answer
    }
}

fn new_gate() -> Arc<(Mutex<bool>, Condvar)> {
    Arc::new((Mutex::new(false), Condvar::new()))
}

fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, cv) = &**gate;
    *lock.lock().unwrap() = true;
    cv.notify_all();
}

// Opens the gate no later than [`WATCHDOG`] so a reducer that wrongly probes
// inline (or a leaked scan thread) cannot wedge the suite; an early manual
// [`release`] wakes it so the caller's join is cheap.
fn spawn_release_watchdog(gate: Arc<(Mutex<bool>, Condvar)>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let (lock, cv) = &*gate;
        let mut released = lock.lock().unwrap();
        let deadline = Instant::now() + WATCHDOG;
        while !*released {
            let now = Instant::now();
            if now >= deadline {
                *released = true;
                cv.notify_all();
                break;
            }
            let (guard, _) = cv.wait_timeout(released, deadline - now).unwrap();
            released = guard;
        }
    })
}

fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + WATCHDOG;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

// Path `i` sorts to library index `i` (newest-first), and is present iff `i`
// is in `present`. Paths do not exist on disk, so the real backend used while
// seeding reports every one missing.
fn library_state(root: &str, n: usize, present: &[usize]) -> AppState {
    let present: HashSet<usize> = present.iter().copied().collect();
    let mut state = AppState::default();
    for i in 0..n {
        let tag = if present.contains(&i) {
            "present"
        } else {
            "missing"
        };
        let path = format!("{root}/{tag}-{i:05}.epub");
        set_progress(
            &mut state,
            &path,
            ProgressRecord {
                total: 100,
                title: format!("Book {i:05}"),
                author: "A. Reader".into(),
                format: "epub".into(),
                last_opened: format!("t{:06}", 999_999 - i),
                ..Default::default()
            },
        );
    }
    state
}

// Drive every task the scan chain produces to a fixed point through the real
// reducer, so results reach the model exactly as the runtime would deliver
// them. Assumes the gate is already open.
fn drive_to_fixpoint(app: &mut App, mut task: iced::Task<Msg>) {
    let mut budget = 100_000usize;
    loop {
        budget = budget.checked_sub(1).expect("scan chain did not terminate");
        let msgs = crate::app::drain_task(task);
        if msgs.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for m in msgs {
            next.push(app.update(m));
        }
        task = iced::Task::batch(next);
    }
}

#[test]
fn go_library_stays_responsive_while_a_scan_is_blocked() {
    let (mut app, _t) = App::test_new();
    app.seed_app_state_for_test(library_state("/nonexistent/resp", 20, &[]));

    let entered = Arc::new(AtomicU64::new(0));
    let gate = new_gate();
    app.set_metadata_backend(Arc::new(BarrierBackend {
        entered: Arc::clone(&entered),
        gate: Arc::clone(&gate),
        answer: true,
    }));
    let watchdog = spawn_release_watchdog(Arc::clone(&gate));

    let first = app.update(Msg::GoLibrary);
    assert_eq!(app.model().screen, crate::model::Screen::Library);
    wait_until(|| entered.load(Ordering::SeqCst) >= 1, "the scan to block");
    assert!(app.availability_active(), "a scan chain must be active");

    // A navigation burst must admit no second scan thread: one probe stays
    // outstanding.
    for _ in 0..5 {
        let _ = app.update(Msg::GoLibrary);
        let _ = app.update(Msg::FontSizeUp);
    }
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "a navigation burst admitted more than one concurrent scan thread"
    );
    assert!(app.availability_active(), "the single chain must still own");

    release(&gate);
    drive_to_fixpoint(&mut app, first);
    assert!(
        !app.availability_active(),
        "the chain must retire when done"
    );
    assert!(
        app.model().library.iter().all(|b| !b.file_missing),
        "the released scan reported every file present"
    );
    watchdog.join().ok();
}
