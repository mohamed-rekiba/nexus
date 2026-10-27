//! What a game's end means for the player: the notice it earns, if any, and whether the core asks
//! "How far did it get?".
//!
//! The rule is pure. [`crate::launch`] reads the ending's facts, applies the answer, and does
//! the rest of what a game's end needs (the screen, the plugin's `on_exit`, the events).

use std::path::Path;
use std::time::Duration;

use nexus_api::ExitInfo;

use super::{PLAYED_AT_LEAST, Session};

/// A game that closes with an error sooner than this closed "right after it started".
const EARLY_EXIT: Duration = Duration::from_secs(10);

/// What is known when a game ends.
pub(crate) struct Ending<'a> {
    /// The session as it ended.
    pub session: &'a Session,
    /// `false`: nothing was played, as the game another program starts never showed.
    pub played_any: bool,
    pub exit: &'a ExitInfo,
    pub played: Duration,
    /// The player has answered "How far did it get?" for this game and build.
    pub answered: bool,
    /// How long the launcher waited for a game another program starts.
    pub hand_over: Duration,
}

/// What the ending asks of the core.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// A notice for the game, when it didn't start or it crashed.
    pub problem: Option<String>,
    /// Ask "How far did it get?".
    pub ask_result: bool,
}

pub(crate) fn outcome(ending: &Ending) -> Outcome {
    let Ending { session, played_any, exit, played, answered, hand_over } = ending;
    // A player's Stop is no crash and no failure to start.
    let problem = if session.stopping {
        None
    } else if !played_any {
        Some(format!("{} didn't start: its game didn't show up within {} seconds", session.title, hand_over.as_secs()))
    } else if !exit.success {
        Some(crash_notice(&session.title, exit, *played, &session.log))
    } else {
        None
    };
    // Once for each build, unless the player stopped it at once.
    let ask_result = *played_any && (!session.stopping || *played >= PLAYED_AT_LEAST) && !answered;
    Outcome { problem, ask_result }
}

/// "Astro Bot closed right after it started (code 1). Its log: …".
fn crash_notice(title: &str, exit: &ExitInfo, played: Duration, log: &Path) -> String {
    let how = match exit.code {
        Some(code) => format!("code {code}"),
        None => "stopped by the system".into(),
    };
    let when = if played < EARLY_EXIT { "right after it started" } else { "with an error" };
    format!("{title} closed {when} ({how}). Its log: {}", log.display())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use nexus_api::GameKey;

    use super::*;

    fn session(stopping: bool) -> Session {
        Session {
            game: "emu:astro".parse::<GameKey>().unwrap(),
            title: "Astro Bot".into(),
            stopping,
            shown: true,
            pid: None,
            started: Instant::now(),
            log: PathBuf::from("/logs/astro.log"),
            build: None,
        }
    }

    fn exit(code: Option<i32>) -> ExitInfo {
        ExitInfo { code, success: code == Some(0), duration_ms: 0 }
    }

    /// The outcome of an ending with the rest as a game that was played and not answered.
    fn ended(stopping: bool, played_any: bool, code: Option<i32>, secs: u64, answered: bool) -> Outcome {
        let session = session(stopping);
        let exit = exit(code);
        outcome(&Ending {
            session: &session,
            played_any,
            exit: &exit,
            played: Duration::from_secs(secs),
            answered,
            hand_over: Duration::from_secs(120),
        })
    }

    const ASK: Outcome = Outcome { problem: None, ask_result: true };

    #[test]
    fn a_game_that_closed_well_asks_how_far_it_got_once_for_each_build() {
        assert_eq!(ended(false, true, Some(0), 600, false), ASK);
        assert_eq!(ended(false, true, Some(0), 600, true), Outcome::default(), "already answered");
    }

    #[test]
    fn a_crash_says_how_soon_and_how_it_ended_with_the_log() {
        let early = ended(false, true, Some(1), 4, false);
        assert_eq!(
            early.problem.as_deref(),
            Some("Astro Bot closed right after it started (code 1). Its log: /logs/astro.log")
        );
        assert!(early.ask_result, "a crash still asks how far it got");
        let just_before = ended(false, true, Some(1), 9, false).problem.unwrap();
        assert!(just_before.contains("right after it started"), "{just_before}");
        let just_after = ended(false, true, Some(1), 10, false).problem.unwrap();
        assert!(just_after.contains("with an error"), "{just_after}");
        let late = ended(false, true, None, 90, false);
        assert_eq!(
            late.problem.as_deref(),
            Some("Astro Bot closed with an error (stopped by the system). Its log: /logs/astro.log")
        );
    }

    #[test]
    fn a_game_the_player_stopped_is_no_crash_and_asks_only_if_it_played_a_while() {
        assert_eq!(ended(true, true, Some(137), 1, false), Outcome::default(), "stopped at once: nothing to say");
        assert_eq!(ended(true, true, Some(137), 3, false), ASK, "three seconds count");
    }

    #[test]
    fn a_game_that_never_showed_says_so_unless_the_player_stopped_it_and_never_asks() {
        let never = ended(false, false, None, 0, false);
        assert_eq!(
            never,
            Outcome {
                problem: Some("Astro Bot didn't start: its game didn't show up within 120 seconds".into()),
                ask_result: false
            }
        );
        assert_eq!(ended(true, false, None, 0, false), Outcome::default(), "Stop before it shows ends it quietly");
    }
}
