//! The launcher's microkernel.
//!
//! It owns plugin lifecycle, the registry, the event bus, app state and the public app API.
//! It knows no emulator and no UI toolkit: plugins come in through `nexus-api`, and the UI
//! reads headless view models and sends commands.

mod app;
mod archive_plan;
mod archives;
mod bus;
mod changes;
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
mod library;
mod library_reads;
mod order;
mod ports;
mod prefetch;
mod programs;
mod release_downloads;
mod releases;
mod resolve;
mod running;
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
pub use archive_plan::{ArchiveEntry, EntryKind, Plan, Planned, UNPACK_SPARE, plan_unpack, room};
pub use archives::embedded_archive;
pub use bus::EventBus;
pub use changes::{Area, Change};
pub use config::ConfigService;
pub use downloads::{Job, JobPhase};
pub use error::{CoreError, CoreResult, LoadError};
pub use kernel::{BootReport, HostConfig, Kernel, KernelBuilder, Loader, Origin, PluginInfo, PluginStatus, Rejection};
pub use library::LibraryCmd;
pub use ports::{ArchiveError, Escape};
pub use ports::{
    ArchiveKind, Archives, Body, Clock, Desktop, Front, Gpu, GroupTree, Hardware, HttpTransport, Network, NoArchives,
    NoDesktop, NoHardware, NoNetwork, NoTorrents, NoVpn, Opened, PackageError, PackageManager, ProcessExit,
    ProcessHandle, ProcessSpawner, ProcessTree, SEED_UPLOAD_LIMIT, SpawnCommand, Specs, TokioClock, TokioSpawner,
    TorrentFile, TorrentInfo, TorrentStats, Torrents, TransferState, UnpackProgress, Vpn,
};
pub use storage::DirStorage;
pub use themes::{DataTheme, DirFiles};
pub use torrents::{TorrentAction, TorrentCmd};
/// The path checks every unpacker runs before it writes.
pub use unpack::{inside, link_stays_inside, stays_inside};
