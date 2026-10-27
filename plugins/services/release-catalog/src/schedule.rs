//! When the fetch of a catalog address and the refresh of a catalog run: the due-time and
//! back-off rules, with no clock of their own. The plugin's task passes the time in, and runs
//! what the plan says.
//!
//! - A copy is used for [`Timing::fresh`] after it was made. Then its work is due again.
//! - A try that failed holds its work: it doesn't run again before the hold's time. The first
//!   hold is [`Timing::retry`], then each one is twice as long, at most the fresh time. A hold
//!   is of one thing (an address, a base copy): work of something else starts again.
//! - A refresh that waits for the VPN is held the same way, but each pass may run it: it asks
//!   the VPN gate again, which asks no forum. While it still waits before its time, the time
//!   stays.
//! - **Refresh now** runs a refresh that is fresh or held.

use std::time::{Duration, SystemTime};

/// How long a copy is used, and how long the first hold is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// How long a copy is used before its work is due again. Also the longest hold.
    pub fresh: Duration,
    /// How long the first hold of a try that failed or waits is.
    pub retry: Duration,
}

/// Work that failed or waits for the VPN, and when it tries again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
    /// What the work was of: the catalog address for a fetch, the base copy for a refresh.
    pub of: String,
    /// It waits for the VPN: a pass asks the VPN gate again before the hold's time.
    pub waiting: bool,
    /// When it tries again.
    pub at: SystemTime,
    /// How long it waited this time.
    pub delay: Duration,
}

/// One console's fetch or refresh, as the schedule sees it.
#[derive(Clone, Copy, Debug)]
pub struct Work<'a> {
    /// What the work is of: the catalog address for a fetch, the base copy for a refresh.
    pub of: &'a str,
    /// When the work made its copy of `of`. `None` with no such copy.
    pub made: Option<SystemTime>,
    /// The hold of the last try, if any. A hold of something other than `of` doesn't count.
    pub held: Option<&'a Hold>,
    /// **Refresh now** asked for it.
    pub forced: bool,
}

impl Work<'_> {
    /// The hold of this work's `of`.
    fn hold(&self) -> Option<&Hold> {
        self.held.filter(|h| h.of == self.of)
    }
}

/// What to do with one console's work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Run it in this pass.
    pub run: bool,
    /// When it is due with no ask: its hold's time, else when its copy gets too old. `None`
    /// with neither.
    pub wake: Option<SystemTime>,
}

/// How a try ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// It didn't run: the VPN isn't connected.
    Waiting,
    Failed,
}

/// Whether a copy made at `made` is still used at `now`.
pub fn fresh(now: SystemTime, made: Option<SystemTime>, timing: &Timing) -> bool {
    made.is_some_and(|made| now < made + timing.fresh)
}

/// What to do with `work` at `now`.
pub fn next(now: SystemTime, work: &Work, timing: &Timing) -> Plan {
    let hold = work.hold();
    let held = hold.is_some_and(|h| now < h.at && !h.waiting);
    let run = work.forced || !(fresh(now, work.made, timing) || held);
    // With no ask, it is due once its copy is too old and its hold is over.
    let stale = work.made.map(|made| made + timing.fresh);
    let wake = match (stale, hold) {
        (Some(stale), Some(hold)) => Some(stale.max(hold.at)),
        (stale, hold) => stale.or(hold.map(|h| h.at)),
    };
    Plan { run, wake }
}

/// The hold after a try of `work` at `now` that ended with `outcome`: none after a try that
/// worked.
pub fn hold_after(now: SystemTime, work: &Work, outcome: Outcome, timing: &Timing) -> Option<Hold> {
    let waiting = match outcome {
        Outcome::Done => return None,
        Outcome::Waiting => true,
        Outcome::Failed => false,
    };
    let before = work.hold();
    if waiting
        && !work.forced
        && let Some(before) = before.filter(|h| h.waiting && now < h.at)
    {
        return Some(before.clone());
    }
    let delay = match before {
        Some(before) => before.delay.saturating_mul(2).min(timing.fresh),
        None => timing.retry.min(timing.fresh),
    };
    Some(Hold { of: work.of.to_owned(), waiting, at: now + delay, delay })
}

/// How long the task waits for its next pass with no ask: until the soonest of `wakes`, and at
/// least a second. With the refresh on, at most the fresh time. `None` when only an ask wakes
/// it.
pub fn sleep(
    now: SystemTime,
    wakes: impl IntoIterator<Item = Option<SystemTime>>,
    refresh_on: bool,
    timing: &Timing,
) -> Option<Duration> {
    let fresh = refresh_on.then_some(now + timing.fresh);
    let soonest = wakes.into_iter().flatten().chain(fresh).min()?;
    Some(soonest.duration_since(now).unwrap_or_default().max(Duration::from_secs(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    const TIMING: Timing = Timing { fresh: Duration::from_secs(1000), retry: Duration::from_secs(300) };

    /// `secs` seconds after 1970.
    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// A case of [`next`]: what it is, when the copy was made, the hold, Refresh now, and the
    /// plan's `run` and `wake`. Times are in seconds since 1970.
    type DueCase = (&'static str, Option<u64>, Option<Hold>, bool, bool, Option<u64>);

    /// A case of [`hold_after`]: what it is, the hold before, Refresh now, how the try ended,
    /// and the hold after as `(waiting, at, delay)` in seconds.
    type HoldCase = (&'static str, Option<Hold>, bool, Outcome, Option<(bool, u64, u64)>);

    fn hold(of: &str, waiting: bool, secs: u64) -> Hold {
        Hold { of: of.into(), waiting, at: at(secs), delay: Duration::from_secs(300) }
    }

    #[test]
    fn work_runs_when_it_is_due() {
        let now = at(10_000);
        let cases: [DueCase; 12] = [
            ("no copy and no hold", None, None, false, true, None),
            ("a fresh copy", Some(9_500), None, false, false, Some(10_500)),
            ("a copy just the fresh time old", Some(9_000), None, false, true, Some(10_000)),
            ("a copy over the fresh time old", Some(8_000), None, false, true, Some(9_000)),
            ("a fresh copy and Refresh now", Some(9_500), None, true, true, Some(10_500)),
            ("held before its time", None, Some(hold("a", false, 10_200)), false, false, Some(10_200)),
            ("held at its time", None, Some(hold("a", false, 10_000)), false, true, Some(10_000)),
            ("held before its time and Refresh now", None, Some(hold("a", false, 10_200)), true, true, Some(10_200)),
            ("waiting for the VPN before its time", None, Some(hold("a", true, 10_200)), false, true, Some(10_200)),
            ("held for something else", None, Some(hold("b", false, 10_200)), false, true, None),
            (
                "held, with a copy over the fresh time old",
                Some(8_000),
                Some(hold("a", false, 10_200)),
                false,
                false,
                Some(10_200),
            ),
            ("held, with a fresh copy", Some(9_500), Some(hold("a", false, 9_900)), false, false, Some(10_500)),
        ];
        for (what, made, held, forced, run, wake) in cases {
            let work = Work { of: "a", made: made.map(at), held: held.as_ref(), forced };
            assert_eq!(next(now, &work, &TIMING), Plan { run, wake: wake.map(at) }, "{what}");
        }
    }

    #[test]
    fn a_held_try_waits_twice_as_long_each_time_at_most_the_fresh_time() {
        let (first, most, now) = (Duration::from_secs(300), Duration::from_secs(1000), at(10_000));
        let timing = Timing { fresh: most, retry: first };
        fn work<'a>(held: Option<&'a Hold>, of: &'a str) -> Work<'a> {
            Work { of, made: None, held, forced: false }
        }
        let one = hold_after(now, &work(None, "a"), Outcome::Failed, &timing).unwrap();
        assert_eq!((one.delay, one.at), (first, now + first));
        let two = hold_after(now, &work(Some(&one), "a"), Outcome::Failed, &timing).unwrap();
        assert_eq!(two.delay, Duration::from_secs(600));
        let three = hold_after(now, &work(Some(&two), "a"), Outcome::Waiting, &timing).unwrap();
        assert_eq!((three.delay, three.waiting), (most, true), "at most the fresh time");
        let other = hold_after(now, &work(Some(&three), "b"), Outcome::Failed, &timing).unwrap();
        assert_eq!(other.delay, first, "a new base copy starts again");
        let short = Timing { fresh: Duration::from_secs(1), retry: first };
        assert_eq!(hold_after(now, &work(None, "a"), Outcome::Failed, &short).unwrap().delay, Duration::from_secs(1));
    }

    #[test]
    fn a_try_sets_the_next_hold() {
        let now = at(10_000);
        let cases: [HoldCase; 7] = [
            ("a try that worked", Some(hold("a", false, 9_000)), false, Outcome::Done, None),
            ("the first failure", None, false, Outcome::Failed, Some((false, 10_300, 300))),
            (
                "a failure after a hold",
                Some(hold("a", false, 9_900)),
                false,
                Outcome::Failed,
                Some((false, 10_600, 600)),
            ),
            (
                "still waiting before its time",
                Some(hold("a", true, 10_200)),
                false,
                Outcome::Waiting,
                Some((true, 10_200, 300)),
            ),
            (
                "still waiting at its time",
                Some(hold("a", true, 10_000)),
                false,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
            (
                "waiting after Refresh now",
                Some(hold("a", true, 10_200)),
                true,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
            (
                "waiting after a failure",
                Some(hold("a", false, 10_200)),
                true,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
        ];
        for (what, held, forced, outcome, after) in cases {
            let work = Work { of: "a", made: None, held: held.as_ref(), forced };
            let want = after.map(|(waiting, secs, delay)| Hold {
                of: "a".into(),
                waiting,
                at: at(secs),
                delay: Duration::from_secs(delay),
            });
            assert_eq!(hold_after(now, &work, outcome, &TIMING), want, "{what}");
        }
    }

    #[test]
    fn the_task_sleeps_until_the_soonest_wake_and_at_least_a_second() {
        let now = at(10_000);
        let secs = Duration::from_secs;
        assert_eq!(sleep(now, [Some(at(10_500)), None, Some(at(10_200))], false, &TIMING), Some(secs(200)));
        assert_eq!(sleep(now, [Some(at(9_000))], false, &TIMING), Some(secs(1)), "a wake gone by");
        assert_eq!(sleep(now, [Some(at(10_000))], false, &TIMING), Some(secs(1)));
        assert_eq!(sleep(now, [None], false, &TIMING), None, "only an ask wakes it");
        assert_eq!(sleep(now, [], true, &TIMING), Some(secs(1000)), "with the refresh on, the fresh time");
        assert_eq!(sleep(now, [Some(at(12_000))], true, &TIMING), Some(secs(1000)));
        assert!(fresh(now, Some(at(9_001)), &TIMING));
        assert!(!fresh(now, Some(at(9_000)), &TIMING));
        assert!(!fresh(now, None, &TIMING));
    }
}
