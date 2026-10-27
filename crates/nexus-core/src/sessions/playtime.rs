//! Play time: each game's total, saved under `app.playtime.<game>`.

use std::time::Duration;

use nexus_api::GameKey;

use crate::config::ConfigService;

/// Where play time is saved in the config.
const PLAYTIME: [&str; 2] = ["app", "playtime"];

/// A session shorter than this didn't really play.
pub(crate) const PLAYED_AT_LEAST: Duration = Duration::from_secs(3);

/// Play time of a game, saved under `app.playtime.<game>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PlayStats {
    /// Seconds played, in all.
    pub secs: u64,
    pub count: u64,
    /// When it last closed, in seconds since 1970.
    pub last: u64,
}

impl PlayStats {
    /// The stats after a session of `played` that closed at `now` (seconds since 1970). `None`
    /// when it was too short to count.
    pub fn played(self, played: Duration, now: u64) -> Option<Self> {
        (played >= PLAYED_AT_LEAST).then(|| Self {
            secs: self.secs + played.as_secs(),
            count: self.count + 1,
            last: now,
        })
    }
}

/// The game's play time, as saved. The config is read each time: nothing else keeps a copy.
pub(crate) fn play_stats(config: &ConfigService, game: &GameKey) -> PlayStats {
    let value = config.get(&[PLAYTIME[0], PLAYTIME[1], &game.to_string()]);
    serde_json::from_value(value).unwrap_or_default()
}

/// Adds a session of `played` to the game's play time, when it was long enough to count.
pub(crate) fn add_play_time(config: &ConfigService, game: &GameKey, played: Duration) {
    let Some(stats) = play_stats(config, game).played(played, now_secs()) else { return };
    let value = serde_json::to_value(stats).expect("play stats serialize");
    if let Err(e) = config.set(&[PLAYTIME[0], PLAYTIME[1], &game.to_string()], value) {
        tracing::warn!(error = %e, %game, "could not save the play time");
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whole days, of 24 hours, from `secs` (since 1970) to now. The core knows no time zone,
/// so 0 means "in the last 24 hours", not "since midnight".
pub(crate) fn days_since(secs: u64) -> u64 {
    days_between(secs, now_secs())
}

fn days_between(secs: u64, now: u64) -> u64 {
    now.saturating_sub(secs) / 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_shorter_than_three_seconds_did_not_play() {
        assert_eq!(PlayStats::default().played(Duration::from_millis(2999), 100), None);
    }

    #[test]
    fn a_session_adds_its_seconds_counts_once_and_is_the_last() {
        let before = PlayStats { secs: 60, count: 2, last: 10 };
        let after = before.played(Duration::from_millis(3500), 100).unwrap();
        assert_eq!(after, PlayStats { secs: 63, count: 3, last: 100 });
    }

    #[test]
    fn days_are_whole_24_hour_days_back_from_now() {
        let now = 10 * 86_400;
        assert_eq!(days_between(now - 1, now), 0, "within the last 24 hours");
        assert_eq!(days_between(now - 86_400, now), 1);
        assert_eq!(days_between(now + 5, now), 0, "a clock that went back is today");
    }
}
