use serde::{Deserialize, Serialize};

use crate::{ExitInfo, GameKey, JobId, PluginId, SessionId};

/// Everything that happens in the host. Plugins can subscribe to all events and publish
/// [`Event::Custom`] only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    PluginActivated {
        plugin: PluginId,
    },
    PluginDeactivated {
        plugin: PluginId,
    },
    PluginFailed {
        plugin: PluginId,
        error: String,
    },
    DownloadQueued {
        job: JobId,
        plugin: PluginId,
        label: String,
    },
    DownloadProgress {
        job: JobId,
        received: u64,
        total: Option<u64>,
    },
    DownloadFinished {
        job: JobId,
    },
    DownloadFailed {
        job: JobId,
        error: String,
    },
    DownloadPaused {
        job: JobId,
    },
    /// A paused download is queued again.
    DownloadResumed {
        job: JobId,
    },
    DownloadCancelled {
        job: JobId,
    },
    InstallChanged {
        plugin: PluginId,
        version: String,
        state: InstallPhase,
    },
    ProcessStarted {
        session: SessionId,
        game: GameKey,
    },
    ProcessExited {
        session: SessionId,
        game: GameKey,
        exit: ExitInfo,
    },
    LibraryChanged,
    /// Navigation, selection, search or settings changed.
    StateChanged,
    ThemeChanged {
        from: Option<PluginId>,
        to: PluginId,
    },
    Custom {
        plugin: PluginId,
        topic: String,
        payload: serde_json::Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Downloading,
    Installing,
    Installed,
    Uninstalled,
    Failed,
}
