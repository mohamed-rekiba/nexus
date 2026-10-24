//! The launcher's microkernel.
//!
//! It owns plugin lifecycle, the registry, the event bus, app state and the public app API.
//! It knows no emulator and no UI toolkit: plugins come in through `nexus-api`, and the UI
//! reads headless view models and sends commands.

mod app;
mod archives;
mod bus;
mod config;
mod downloads;
mod error;
mod facades;
mod folders;
mod game_installs;
mod guard;
mod hardware;
mod kernel;
mod launch;
pub mod launcher_update;
mod order;
mod ports;
mod prefetch;
mod programs;
mod releases;
mod resolve;
mod storage;
pub mod system;
mod themes;
mod torrents;
mod unpack;
mod view;
mod vpn;

pub use app::{
    Command, Core, GRID_PAGE, Grid, HIDDEN_FILTER, INGAME_FILTER, INSTALLED_FILTER, MAX_SCALE, MIN_SCALE, Route,
};
pub use bus::EventBus;
pub use config::ConfigService;
pub use downloads::{Job, JobPhase};
pub use error::{CoreError, CoreResult, LoadError};
pub use kernel::{BootReport, HostConfig, Kernel, KernelBuilder, Loader, Origin, PluginInfo, PluginStatus, Rejection};
pub use ports::{
    ArchiveKind, Archives, Body, Desktop, Front, Gpu, Hardware, HttpTransport, Network, NoArchives, NoDesktop,
    NoHardware, NoNetwork, NoTorrents, NoVpn, Opened, PackageError, PackageManager, ProcessExit, ProcessHandle,
    ProcessSpawner, SEED_UPLOAD_LIMIT, SpawnCommand, Specs, TokioSpawner, TorrentFile, TorrentInfo, TorrentPhase,
    TorrentStart, TorrentStatus, Torrents, UnpackProgress, Vpn,
};
pub use storage::DirStorage;
pub use themes::{DataTheme, DirFiles};
pub use torrents::TorrentAction;
/// The path checks every unpacker runs before it writes.
pub use unpack::{inside, link_stays_inside, stays_inside};
