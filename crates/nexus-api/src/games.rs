//! Installing games from packages and disc images the player has, into the player's game
//! folders. A plugin reads and decodes; the host owns the job: its progress, cancel, and the
//! clean-up after a crash.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{HostPath, PluginResult};

/// A slot action the host runs itself: installs the game package in `args` (a `HostPath`)
/// with the plugin's game installer, as a job in Downloads.
pub const HOST_INSTALL_GAME: &str = "host:install-game";

/// A package or image the plugin can install, found in a folder it may read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Installable {
    pub source: HostPath,
    /// What the player sees, for example "Astro Bot (PPSA01234)".
    pub label: String,
}

/// Where a game went.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledGame {
    pub dest: HostPath,
    pub title: String,
}

/// The host's side of one install, which a [`GameInstaller`] runs in the background.
pub trait InstallJob: Send + Sync {
    /// `done` of `total` bytes written.
    fn progress(&self, done: u64, total: u64);
    /// The player cancelled: stop at the next chance and call [`InstallJob::finished`] with an
    /// error.
    fn cancelled(&self) -> bool;
    /// Called exactly once, at the end. On an error the work folder must be gone.
    fn finished(&self, result: PluginResult<InstalledGame>);
    /// The folder the player picked for the game, in the `games` folders: the game goes in
    /// it. `None` leaves the choice to the plugin.
    fn games_folder(&self) -> Option<HostPath> {
        None
    }
    /// Moves the folder `from`, in the source, to `to`, in the `games` folders, in place of
    /// copying it: `true` when it moved. The host agrees only for a copy it made itself, such
    /// as a download's unpacked archives, on the same disk; otherwise copy as usual.
    fn take(&self, _from: &HostPath, _to: &HostPath) -> bool {
        false
    }
}

/// Installs games from packages or disc images.
#[async_trait]
pub trait GameInstaller: Send + Sync {
    /// The packages and images this plugin can install, in the folders it may read.
    async fn candidates(&self) -> PluginResult<Vec<Installable>>;

    /// Checks `source` and starts installing it in the background, then returns at once: a
    /// source that isn't a package fails here. The work writes into the returned work folder,
    /// moves it into place when it is done, and reports through `job`. The host removes the
    /// work folder if the app stops before the end.
    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath>;
}
