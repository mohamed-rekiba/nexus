//! The phase rules of a running download: what the engine's raw [`TorrentStats`] mean for the
//! download, and what the core does about them. Pure: [`next`] reads and returns values, and
//! the subsystem runs the actions it returns.

use serde::{Deserialize, Serialize};

use crate::{TorrentStats, TransferState};

/// Why a finished download that checked its files again can't seed.
pub(crate) const FILES_CHANGED: &str = "The downloaded files were moved, changed or deleted, so this download can't seed. Remove it, or download it again.";

/// What a seed-only start says when a piece is missing.
const PIECES_MISSING: &str = "pieces are missing";

/// Where a download is, as the player sees it and as it is saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TorrentState {
    #[default]
    Resolving,
    Ready,
    Checking,
    Downloading,
    Paused,
    Complete,
    Failed,
    Cancelled,
}

impl TorrentState {
    pub fn active(self) -> bool {
        matches!(self, Self::Resolving | Self::Checking | Self::Downloading)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolving => "resolving",
            Self::Ready => "ready",
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What a torrent was started to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TorrentStart {
    /// Download what is missing, then seed if `seed` is on, else finish.
    Download { seed: bool },
    /// Check the pieces in the folder and seed them. A missing piece fails it: nothing
    /// downloads.
    SeedOnly,
}

/// The part of a download the phase rules read and change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Phase {
    pub state: TorrentState,
    pub done: u64,
    pub total: u64,
    pub seeding: bool,
    /// Checking a finished download's files before it seeds again.
    pub verifying: bool,
    pub error: String,
}

/// What the subsystem does after a poll, in this order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// The check found every piece: connect, to seed.
    Unpause,
    /// Stop the torrent; its files stay.
    Stop,
    /// The download just finished: it is complete for the first time.
    Finished,
    /// The download just failed, with the error in its phase.
    Stopped,
}

/// The download's next phase from its phase now, the engine's `stats`, how it was started,
/// and whether "Keep sharing finished downloads" is on (`sharing`).
pub(crate) fn next(phase: &Phase, stats: &TorrentStats, start: TorrentStart, sharing: bool) -> (Phase, Vec<Action>) {
    let mut next = phase.clone();
    let mut actions = Vec::new();
    if stats.total > 0 {
        next.total = stats.total;
    }
    let seed = sharing && matches!(start, TorrentStart::SeedOnly | TorrentStart::Download { seed: true });
    match (stats.state, stats.finished, start) {
        (TransferState::Error, ..) => fail(&mut next, &mut actions, &stats.error),
        (TransferState::Checking, ..) => {
            if !next.verifying {
                next.state = TorrentState::Checking;
                next.done = stats.done;
            }
        }
        // Checked and whole, to seed: connect.
        (TransferState::Paused, true, _) if seed => actions.push(Action::Unpause),
        (TransferState::Paused | TransferState::Live, true, _) => {
            if next.state != TorrentState::Complete {
                actions.push(Action::Finished);
            }
            next.state = TorrentState::Complete;
            next.done = next.total;
            next.verifying = false;
            next.seeding = stats.state == TransferState::Live && seed;
            if !next.seeding {
                actions.push(Action::Stop);
            }
        }
        // A seed-only start never downloads: a missing piece fails it.
        (_, false, TorrentStart::SeedOnly) => fail(&mut next, &mut actions, PIECES_MISSING),
        (TransferState::Live | TransferState::Paused, false, TorrentStart::Download { .. }) => {
            next.state = TorrentState::Downloading;
            next.done = stats.done;
        }
    }
    (next, actions)
}

/// The torrent stopped with `error`. A finished download that was checking its files keeps
/// its state, and says they changed.
fn fail(next: &mut Phase, actions: &mut Vec<Action>, error: &str) {
    if next.verifying {
        next.error = FILES_CHANGED.into();
    } else {
        // Once for each stop: the next polls see it failed already.
        if next.state != TorrentState::Failed {
            actions.push(Action::Stopped);
        }
        next.state = TorrentState::Failed;
        next.error = error.to_owned();
    }
    next.verifying = false;
    next.seeding = false;
    actions.push(Action::Stop);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: TorrentStart = TorrentStart::Download { seed: true };
    const NO_SEED: TorrentStart = TorrentStart::Download { seed: false };
    const CHECK: TorrentStart = TorrentStart::SeedOnly;

    fn stats(state: TransferState, finished: bool, done: u64) -> TorrentStats {
        TorrentStats {
            state,
            finished,
            done,
            total: 100,
            download_rate: 0,
            upload_rate: 0,
            peers: 0,
            error: String::new(),
        }
    }

    fn at(state: TorrentState) -> Phase {
        Phase { state, total: 100, ..Phase::default() }
    }

    /// A finished download checking its files before it seeds again.
    fn verifying() -> Phase {
        Phase { state: TorrentState::Complete, done: 100, total: 100, verifying: true, ..Phase::default() }
    }

    #[test]
    fn a_check_shows_its_progress() {
        let (next, actions) = next(&at(TorrentState::Checking), &stats(TransferState::Checking, false, 40), SEED, true);
        assert_eq!((next.state, next.done), (TorrentState::Checking, 40));
        assert!(actions.is_empty());
    }

    #[test]
    fn a_finished_download_that_checks_its_files_stays_complete() {
        let (next, actions) = next(&verifying(), &stats(TransferState::Checking, false, 40), CHECK, true);
        assert_eq!(next, verifying());
        assert!(actions.is_empty());
    }

    #[test]
    fn a_live_torrent_with_pieces_missing_downloads() {
        let (next, actions) = next(&at(TorrentState::Checking), &stats(TransferState::Live, false, 60), SEED, true);
        assert_eq!((next.state, next.done), (TorrentState::Downloading, 60));
        assert!(actions.is_empty());
    }

    #[test]
    fn the_engine_s_total_replaces_the_catalog_s() {
        let mut live = stats(TransferState::Live, false, 10);
        live.total = 250;
        assert_eq!(next(&at(TorrentState::Downloading), &live, SEED, true).0.total, 250);
        live.total = 0;
        assert_eq!(next(&at(TorrentState::Downloading), &live, SEED, true).0.total, 100, "0 says nothing");
    }

    #[test]
    fn a_finished_download_seeds_while_sharing_is_on() {
        let (next, actions) = next(&at(TorrentState::Downloading), &stats(TransferState::Live, true, 100), SEED, true);
        assert_eq!((next.state, next.done, next.seeding), (TorrentState::Complete, 100, true));
        assert_eq!(actions, [Action::Finished]);
    }

    #[test]
    fn finished_without_seeding_finishes_and_stops() {
        let (next, actions) =
            next(&at(TorrentState::Downloading), &stats(TransferState::Live, true, 100), NO_SEED, true);
        assert_eq!((next.state, next.seeding), (TorrentState::Complete, false));
        assert_eq!(actions, [Action::Finished, Action::Stop]);
    }

    #[test]
    fn sharing_turned_off_during_the_download_stops_it_when_it_finishes() {
        let (next, actions) = next(&at(TorrentState::Downloading), &stats(TransferState::Live, true, 100), SEED, false);
        assert_eq!((next.state, next.seeding), (TorrentState::Complete, false));
        assert_eq!(actions, [Action::Finished, Action::Stop]);
    }

    #[test]
    fn a_seeding_download_finishes_only_once() {
        let seeding = Phase { seeding: true, ..at(TorrentState::Complete) };
        let (next, actions) = next(&seeding, &stats(TransferState::Live, true, 100), SEED, true);
        assert!(next.seeding);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn a_seed_only_check_that_finds_every_piece_connects_to_seed() {
        let (next, actions) = next(&verifying(), &stats(TransferState::Paused, true, 100), CHECK, true);
        assert_eq!(next, verifying(), "still checking until it is live");
        assert_eq!(actions, [Action::Unpause]);
        let (seeding, actions) = super::next(&next, &stats(TransferState::Live, true, 100), CHECK, true);
        assert_eq!((seeding.verifying, seeding.seeding, seeding.state), (false, true, TorrentState::Complete));
        assert!(actions.is_empty(), "a finished download doesn't finish again: {actions:?}");
    }

    #[test]
    fn a_whole_seed_only_check_with_sharing_off_stops() {
        let (next, actions) = next(&verifying(), &stats(TransferState::Paused, true, 100), CHECK, false);
        assert_eq!((next.verifying, next.seeding), (false, false));
        assert_eq!(actions, [Action::Stop]);
    }

    #[test]
    fn seed_only_with_missing_pieces_fails_and_says_the_files_changed() {
        for state in [TransferState::Paused, TransferState::Live] {
            let (next, actions) = next(&verifying(), &stats(state, false, 30), CHECK, true);
            assert_eq!(next.state, TorrentState::Complete, "the download stays complete");
            assert_eq!(next.error, FILES_CHANGED);
            assert_eq!((next.verifying, next.seeding), (false, false));
            assert_eq!(actions, [Action::Stop], "it never downloads");
        }
    }

    #[test]
    fn an_engine_error_fails_the_download_once() {
        let mut broken = stats(TransferState::Error, false, 30);
        broken.error = "disk full".into();
        let (failed, actions) = next(&at(TorrentState::Downloading), &broken, SEED, true);
        assert_eq!((failed.state, failed.error.as_str()), (TorrentState::Failed, "disk full"));
        assert_eq!(actions, [Action::Stopped, Action::Stop]);
        let (_, again) = next(&failed, &broken, SEED, true);
        assert_eq!(again, [Action::Stop], "the player hears of it once");
    }

    #[test]
    fn an_engine_error_while_checking_finished_files_says_they_changed() {
        let (next, actions) = next(&verifying(), &stats(TransferState::Error, false, 0), CHECK, true);
        assert_eq!((next.state, next.error.as_str()), (TorrentState::Complete, FILES_CHANGED));
        assert_eq!(actions, [Action::Stop]);
    }

    #[test]
    fn a_paused_finished_download_that_seeds_connects_and_one_that_doesn_t_stops() {
        let (_, actions) = next(&at(TorrentState::Downloading), &stats(TransferState::Paused, true, 100), SEED, true);
        assert_eq!(actions, [Action::Unpause]);
        let (next, actions) =
            next(&at(TorrentState::Downloading), &stats(TransferState::Paused, true, 100), NO_SEED, true);
        assert_eq!(next.state, TorrentState::Complete);
        assert_eq!(actions, [Action::Finished, Action::Stop]);
    }
}
