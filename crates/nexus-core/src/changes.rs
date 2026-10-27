//! The core's change signal: which part of the state changed. It is for the core and its
//! tests. The app renders on [`nexus_api::Event::StateChanged`], which the same change
//! publishes, and plugins never see it.

use nexus_api::Event;
use tokio::sync::broadcast;

use crate::EventBus;

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

/// How a subsystem says its state changed: the core's readers learn the area, and the app
/// learns that something changed ([`Event::StateChanged`]). Every subsystem publishes to the
/// core's one signal, so [`crate::Core::changes`] is the merge of theirs.
#[derive(Clone)]
pub(crate) struct Notifier {
    changes: Changes,
    bus: EventBus,
}

impl Notifier {
    pub(crate) fn new(changes: Changes, bus: EventBus) -> Self {
        Self { changes, bus }
    }

    pub(crate) fn changed(&self, area: Area) {
        self.changes.publish(area);
        self.bus.publish(Event::StateChanged);
    }

    /// Only the core's readers learn that `area` changed. For a change that comes with
    /// another one, which tells the app.
    pub(crate) fn noted(&self, area: Area) {
        self.changes.publish(area);
    }
}
