//! The core's change signal: which part of the state changed. It is for the core and its
//! tests. The app renders on [`nexus_api::Event::StateChanged`], which the same change
//! publishes, and plugins never see it.

use tokio::sync::broadcast;

/// A part of the core's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    /// The screen stack, the selection, and the grids' search, filters and pages.
    Navigation,
    /// The installed games, their details and store metadata.
    Library,
    /// The releases the catalogs offer.
    Catalogs,
    /// Emulator downloads, installs and updates, and the games a store installs.
    Downloads,
    /// Game installs.
    Installs,
    /// Torrent downloads, with the unpacking of what they bring.
    Torrents,
    /// Running games, and the questions before and after a game.
    Sessions,
    Notices,
    /// The app's own settings.
    Settings,
    /// Plugins: which are on, their settings, and what their actions changed.
    Plugins,
    /// The machine's settings, from the system plugin.
    System,
    /// The VPNs and the VPN rule for downloads.
    Vpn,
    /// What the "This PC" check read.
    Hardware,
    /// The launcher's own update.
    LauncherUpdate,
}

/// The core's state in `area` changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub area: Area,
}

/// Sends each [`Change`] to every reader. A reader that falls more than the buffer behind gets
/// `Lagged` and skips the oldest.
#[derive(Clone)]
pub(crate) struct Changes {
    tx: broadcast::Sender<Change>,
}

impl Changes {
    pub(crate) fn new() -> Self {
        Self { tx: broadcast::channel(1024).0 }
    }

    pub(crate) fn publish(&self, area: Area) {
        // No reader is fine.
        let _ = self.tx.send(Change { area });
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.tx.subscribe()
    }
}
