use std::num::NonZeroUsize;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use super::App;
use enzui_iced_core::blocking_task::{BlockingTaskDispatcher, BlockingTaskOwner};

const WAIT: Duration = Duration::from_secs(2);

pub(super) struct PoolHold {
    release: Arc<(Mutex<bool>, Condvar)>,
    expired: Arc<AtomicBool>,
    // Retaining settlement futures prevents cancellation of the queued capacity probe.
    tasks: Vec<enzui_iced_core::blocking_task::BlockingTask<()>>,
}

impl PoolHold {
    fn release(&self) {
        let (lock, wake) = &*self.release;
        *lock.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_all();
    }

    pub(super) fn finish(&self, dispatcher: &BlockingTaskDispatcher) {
        self.release();
        wait_pool_empty(dispatcher);
        assert!(
            !self.expired.load(Ordering::SeqCst),
            "hold expired before release"
        );
    }
}

impl Drop for PoolHold {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) fn wait_pool_empty(dispatcher: &BlockingTaskDispatcher) {
    let deadline = Instant::now() + WAIT;
    while dispatcher.snapshot().live() != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(dispatcher.snapshot().live(), 0, "held owner did not drain");
}

pub(super) fn held_app(mut app: App) -> (App, PoolHold, BlockingTaskDispatcher, BlockingTaskOwner) {
    let owner = BlockingTaskOwner::new(
        "reader-held-owner-test",
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(1).unwrap(),
    )
    .unwrap();
    let dispatcher = owner.dispatcher();
    let old = app.replace_blocking_owner_for_test(owner, dispatcher.clone());
    let mut hold = PoolHold {
        release: Arc::new((Mutex::new(false), Condvar::new())),
        expired: Arc::new(AtomicBool::new(false)),
        tasks: Vec::new(),
    };
    let release = hold.release.clone();
    let expired = hold.expired.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    hold.tasks.push(dispatcher.submit(100, move |_| {
        entered_tx.send(()).unwrap();
        let deadline = Instant::now() + WAIT;
        let (lock, wake) = &*release;
        let mut released = lock.lock().unwrap();
        while !*released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                expired.store(true, Ordering::SeqCst);
                break;
            }
            released = wake.wait_timeout(released, remaining).unwrap().0;
        }
    }));
    entered_rx.recv_timeout(WAIT).expect("held worker started");
    hold.tasks.push(dispatcher.submit(101, |_| ()));
    let full = dispatcher.snapshot();
    assert_eq!(
        (full.running, full.queued, full.live(), full.live_capacity()),
        (1, 1, 2, 2)
    );
    (app, hold, dispatcher, old)
}
