//! The notices subsystem: the short messages the player sees until they close them, such as
//! "Download complete: Astro Bot. Install it from Downloads.".
//!
//! It follows the subsystem pattern (see [`crate::torrents`]): its own lock, `handle`,
//! `snapshot`, and its changes as [`Area::Notices`]. It keeps at most
//! [`nexus_ui_api::MAX_NOTICES`]: a new one drops the oldest. Each notice has its own id,
//! which only grows.

use std::sync::{Arc, Mutex, MutexGuard};

use nexus_api::GameKey;
use nexus_ui_api::NoticeKind;
use serde::{Deserialize, Serialize};

use crate::app::Route;
use crate::changes::{Area, Notifier};
use crate::{CoreError, CoreResult};

/// A short message for the player, until it is closed.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub message: String,
    pub kind: NoticeKind,
    /// Where the notice's subject is, for the player to open.
    pub route: Option<Route>,
    /// The game the notice is about: its card shows the game's art.
    pub game: Option<GameKey>,
}

/// What the player does with the notices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeCmd {
    /// Closes every notice.
    DismissAll,
    /// Closes the notice with this id (`NoticeVm::id`); the others stay.
    Dismiss(u64),
    /// The notice with this id was clicked: open what it is about, and close it.
    Open(u64),
}

#[derive(Default)]
struct State {
    /// Newest last. Shared with the snapshots, and copied only when it changes.
    notices: Arc<Vec<Notice>>,
    /// The id the last notice got.
    last_id: u64,
}

/// The notices. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct NoticesSubsystem {
    inner: Arc<Shared>,
}

struct Shared {
    notifier: Notifier,
    state: Mutex<State>,
}

impl NoticesSubsystem {
    pub fn new(notifier: Notifier) -> Self {
        Self { inner: Arc::new(Shared { notifier, state: Mutex::default() }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The notices, newest last.
    pub fn snapshot(&self) -> Arc<Vec<Notice>> {
        self.state().notices.clone()
    }

    /// Shows the player `message`. Clicking it opens `route`, when there is one. Only the
    /// core's readers learn of it: the change that goes with the notice tells the app.
    pub fn post(&self, kind: NoticeKind, message: String, route: Option<Route>, game: Option<GameKey>) {
        self.add(kind, message, route, game);
        self.inner.notifier.noted(Area::Notices);
    }

    /// Shows the player a problem outside the core, such as a theme's logic failing. Nothing
    /// else changes with it, so the app learns of it too.
    pub fn report(&self, message: String) {
        self.add(NoticeKind::Error, message, None, None);
        self.inner.notifier.changed(Area::Notices);
    }

    fn add(&self, kind: NoticeKind, message: String, route: Option<Route>, game: Option<GameKey>) {
        // Only a problem is a warning in the log: "Download complete" is news.
        if kind == NoticeKind::Error {
            tracing::warn!(%message, "notice");
        } else {
            tracing::info!(%message, kind = kind.as_str(), "notice");
        }
        let mut state = self.state();
        state.last_id += 1;
        let id = state.last_id;
        let notices = Arc::make_mut(&mut state.notices);
        notices.push(Notice { id, message, kind, route, game });
        if notices.len() > nexus_ui_api::MAX_NOTICES {
            notices.remove(0);
        }
    }

    /// Closes the notice `id` and returns it: the core opens its route.
    pub fn take(&self, id: u64) -> Option<Notice> {
        let notice = {
            let mut state = self.state();
            let at = state.notices.iter().position(|n| n.id == id)?;
            Arc::make_mut(&mut state.notices).remove(at)
        };
        self.inner.notifier.changed(Area::Notices);
        Some(notice)
    }

    /// Runs a command that is the subsystem's alone. The core runs [`NoticeCmd::Open`]: it
    /// navigates.
    pub fn handle(&self, command: NoticeCmd) -> CoreResult<()> {
        match command {
            NoticeCmd::DismissAll => Arc::make_mut(&mut self.state().notices).clear(),
            NoticeCmd::Dismiss(id) => Arc::make_mut(&mut self.state().notices).retain(|n| n.id != id),
            NoticeCmd::Open(_) => return Err(CoreError::Invalid("the core opens a notice".into())),
        }
        self.inner.notifier.changed(Area::Notices);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventBus;
    use crate::changes::Changes;

    fn notices() -> (NoticesSubsystem, tokio::sync::broadcast::Receiver<crate::Change>) {
        let changes = Changes::new();
        let rx = changes.subscribe();
        (NoticesSubsystem::new(Notifier::new(changes, EventBus::default())), rx)
    }

    fn texts(notices: &NoticesSubsystem) -> Vec<String> {
        notices.snapshot().iter().map(|n| n.message.clone()).collect()
    }

    #[test]
    fn only_the_newest_stay_each_with_a_higher_id() {
        let (notices, mut changes) = notices();
        for n in 1..=4 {
            notices.post(NoticeKind::Info, format!("News {n}"), None, None);
        }
        assert_eq!(texts(&notices), ["News 2", "News 3", "News 4"], "the oldest goes");
        let ids: Vec<u64> = notices.snapshot().iter().map(|n| n.id).collect();
        assert_eq!(ids, [2, 3, 4]);
        for _ in 1..=4 {
            assert_eq!(changes.try_recv().unwrap().area, Area::Notices);
        }
    }

    #[test]
    fn a_closed_notice_s_id_is_not_given_again() {
        let (notices, _changes) = notices();
        notices.post(NoticeKind::Info, "One".into(), None, None);
        notices.handle(NoticeCmd::DismissAll).unwrap();
        notices.post(NoticeKind::Info, "Two".into(), None, None);
        assert_eq!(notices.snapshot()[0].id, 2);
    }

    #[test]
    fn dismissing_one_leaves_the_others() {
        let (notices, _changes) = notices();
        for n in 1..=3 {
            notices.post(NoticeKind::Info, format!("News {n}"), None, None);
        }
        notices.handle(NoticeCmd::Dismiss(2)).unwrap();
        assert_eq!(texts(&notices), ["News 1", "News 3"]);
        notices.handle(NoticeCmd::DismissAll).unwrap();
        assert!(notices.snapshot().is_empty());
    }

    #[test]
    fn taking_a_notice_closes_it_and_gives_its_route() {
        let (notices, _changes) = notices();
        notices.post(NoticeKind::Success, "Done".into(), Some(Route::Downloads), None);
        notices.report("Broke".into());
        let taken = notices.take(1).expect("the notice");
        assert_eq!(taken.route, Some(Route::Downloads));
        assert_eq!(texts(&notices), ["Broke"]);
        assert_eq!(notices.snapshot()[0].kind, NoticeKind::Error, "a report is a problem");
        assert!(notices.take(1).is_none(), "taken once");
        assert!(notices.handle(NoticeCmd::Open(2)).is_err(), "the core opens a notice");
    }

    #[test]
    fn a_snapshot_stays_as_it_was_read() {
        let (notices, _changes) = notices();
        notices.post(NoticeKind::Info, "One".into(), None, None);
        let before = notices.snapshot();
        notices.post(NoticeKind::Info, "Two".into(), None, None);
        assert_eq!(before.len(), 1);
        assert_eq!(notices.snapshot().len(), 2);
    }
}
