//! The core's clock in tests.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use nexus_core::{Clock, TokioClock};

/// The runtimes whose time a `FakeClock` paused: tokio refuses a second pause.
static PAUSED: Mutex<Option<HashSet<tokio::runtime::Id>>> = Mutex::new(None);

/// Tokio's time, paused: it moves only when a test moves it, or when every task waits for a
/// timer. Then tokio moves it at once to the next timer due (its auto-advance). A slow machine
/// doesn't move it, and work in `spawn_blocking` holds it still until that work ends.
///
/// The first use pauses the time of the runtime it runs on, which must be a current-thread
/// runtime (`#[tokio::test]`, not `start_paused` and not `multi_thread`). Every timer on that
/// runtime shares that one time. Once paused, [`Clock::now`] works off the runtime too: a
/// test's UI thread reads the view, and the view reads the time.
#[derive(Clone, Debug, Default)]
pub struct FakeClock {
    runtime: Arc<OnceLock<tokio::runtime::Handle>>,
}

impl FakeClock {
    /// Pauses this runtime's time, once.
    pub(crate) fn pause(&self) {
        let runtime = tokio::runtime::Handle::try_current().expect("a FakeClock starts inside a tokio runtime");
        assert_eq!(
            runtime.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::CurrentThread,
            "a FakeClock needs a current-thread runtime: tokio pauses time only there"
        );
        let mut paused = PAUSED.lock().unwrap_or_else(|p| p.into_inner());
        if paused.get_or_insert_with(HashSet::new).insert(runtime.id()) {
            tokio::time::pause();
        }
        drop(paused);
        // Set by the first use; every later use finds it set.
        let _ = self.runtime.set(runtime);
    }

    /// Moves the time on by `by`, firing each timer due on the way in order, then lets the
    /// work they woke run. Tokio's timers tick in milliseconds, so the time can end a few
    /// milliseconds past `by`.
    pub async fn advance(&self, by: Duration) {
        self.pause();
        tokio::time::sleep(by).await;
        self.run_until_idle().await;
    }

    /// Returns once every task waits, for a timer, a gate or a change, and no work runs in
    /// `spawn_blocking`. The time moves at most one tick of tokio's timers (a millisecond):
    /// tokio notices that every task waits only by moving the time to the next timer.
    pub async fn run_until_idle(&self) {
        self.pause();
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        if tokio::runtime::Handle::try_current().is_ok() {
            self.pause();
            return TokioClock.now();
        }
        let runtime = self.runtime.get().expect("a FakeClock is read off its runtime only once it started");
        let _inside = runtime.enter();
        TokioClock.now()
    }

    fn sleep(&self, wait: Duration) -> BoxFuture<'static, ()> {
        self.pause();
        TokioClock.sleep(wait)
    }

    fn interval(&self, every: Duration) -> BoxStream<'static, ()> {
        self.pause();
        TokioClock.interval(every)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// A task that sleeps `wait` on `clock`, and says when it woke.
    fn sleeper(clock: &FakeClock, wait: Duration) -> Arc<AtomicBool> {
        let woke = Arc::new(AtomicBool::new(false));
        let (sleep, flag) = (clock.sleep(wait), woke.clone());
        tokio::spawn(async move {
            sleep.await;
            flag.store(true, Ordering::SeqCst);
        });
        woke
    }

    #[tokio::test]
    async fn a_sleep_ends_on_advance_and_not_before() {
        let clock = FakeClock::default();
        let woke = sleeper(&clock, Duration::from_secs(10));
        clock.advance(Duration::from_secs(9)).await;
        assert!(!woke.load(Ordering::SeqCst), "9 s of 10");
        clock.advance(Duration::from_secs(1)).await;
        assert!(woke.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn the_time_moves_only_when_the_test_moves_it() {
        let clock = FakeClock::default();
        let start = clock.now();
        // Real time passes, and the thread works: the clock stays.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(clock.now(), start);
        clock.advance(Duration::from_millis(250)).await;
        let moved = clock.now() - start;
        // Tokio's timers tick in milliseconds: an advance can go a few past.
        assert!(moved >= Duration::from_millis(250) && moved <= Duration::from_millis(254), "{moved:?}");
    }

    #[tokio::test]
    async fn an_interval_ticks_once_for_each_period_the_clock_passes() {
        let clock = FakeClock::default();
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (mut interval, count) = (clock.interval(Duration::from_secs(1)), ticks.clone());
        tokio::spawn(async move {
            while futures::StreamExt::next(&mut interval).await.is_some() {
                count.fetch_add(1, Ordering::SeqCst);
            }
        });
        clock.run_until_idle().await;
        assert_eq!(ticks.load(Ordering::SeqCst), 0, "the first tick is a period away");
        clock.advance(Duration::from_millis(3500)).await;
        assert_eq!(ticks.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_timeout_fires_on_the_clock_only() {
        let clock = FakeClock::default();
        let limit = (&clock as &dyn Clock).timeout(Duration::from_millis(500), std::future::pending::<()>());
        assert_eq!(limit.await, None, "the clock moved itself to the limit: every task waited");
        let work = (&clock as &dyn Clock).timeout(Duration::from_millis(500), async { 7 });
        assert_eq!(work.await, Some(7));
    }

    #[tokio::test]
    async fn work_in_spawn_blocking_holds_the_clock() {
        let clock = FakeClock::default();
        let start = clock.now();
        let blocking = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(100)));
        let done = (&clock as &dyn Clock).timeout(Duration::from_millis(50), blocking).await;
        assert!(done.is_some(), "a 50 ms limit didn't cut 100 ms of real work");
        assert_eq!(clock.now(), start);
    }

    #[tokio::test]
    async fn run_until_idle_returns_once_the_work_ended() {
        let clock = FakeClock::default();
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        tokio::spawn(async move {
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(20))).await.unwrap();
            tokio::task::yield_now().await;
            flag.store(true, Ordering::SeqCst);
        });
        clock.run_until_idle().await;
        assert!(done.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn every_clock_on_a_runtime_has_its_time_and_reads_it_off_the_runtime() {
        let (first, second) = (FakeClock::default(), FakeClock::default());
        first.advance(Duration::from_secs(5)).await;
        assert_eq!(first.now(), second.now(), "one time for the runtime");
        let off = std::thread::spawn({
            let first = first.clone();
            move || first.now()
        });
        assert_eq!(off.join().unwrap(), first.now(), "a thread with no runtime reads the same time");
    }
}
