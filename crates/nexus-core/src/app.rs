//! The public app API: commands in, view models and events out.
//!
//! All state lives here, never in a theme, so the UI can be rebuilt (or a theme swapped) at
//! any time without losing navigation, selection, downloads or running games.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::stream::BoxStream;
use nexus_api::{
    Capability, ConfigScope, DownloadRequest, Event, GameDetails, GameKey, GameSummary, InstallPhase, JobId,
    PageRequest, PluginId, PluginKind, Provided, SessionId, SlotId, SlotRequest,
};
use nexus_ui_api::{AppView, CompatVm, NoticeKind, SettingsPanelVm, SlotVm};
use serde_json::Value;

use crate::downloads::Downloads;
use crate::error::require;
use crate::view::{self, slot_vms};
use crate::{CoreError, CoreResult, Kernel};

/// Where the player is. The navigation stack is a list of routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Home,
    GameDetail(GameKey),
    Catalog,
    Downloads,
    Settings,
    /// The emulator plugins, to open one.
    Emulators,
    /// The theme plugins, to open one.
    Themes,
    Plugin(PluginId),
    /// One game's own settings.
    GameSettings(GameKey),
    Running(SessionId),
}

/// A grid of game cards that tells the core how many columns it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grid {
    /// The Library's grid, of the installed games.
    Library,
    /// The Catalog's grid, of the games the catalogs offer.
    Catalog,
}

/// Everything the UI can ask the core to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Navigate(Route),
    Back,
    Select(GameKey),
    OpenGame(GameKey),
    RefreshLibrary,
    Search(String),
    /// Queues a download and returns. Progress comes as events.
    Download {
        plugin: PluginId,
        request: DownloadRequest,
    },
    PauseDownload(JobId),
    ResumeDownload(JobId),
    /// Stops a download and deletes what it fetched.
    CancelDownload(JobId),
    /// `None` installs the newest available version.
    InstallEmulator {
        plugin: PluginId,
        version: Option<String>,
    },
    UpdateEmulator {
        plugin: PluginId,
    },
    UninstallEmulator {
        plugin: PluginId,
        version: String,
    },
    /// Deletes an installed game's folder, and takes it out of the Library. Not while it runs
    /// or its emulator installs a game.
    UninstallGame(GameKey),
    /// Starts a game. One game runs at a time.
    Launch(GameKey),
    /// Asks the game to close, and kills it if it doesn't.
    Stop(SessionId),
    /// Brings the running game's window back to the front.
    ShowGame(SessionId),
    /// The PS button while a game runs: from the game to the launcher, or back to the game.
    ToggleFocus,
    /// Deletes the emulator's compiled shaders for a game.
    ClearShaderCache(GameKey),
    SetConfig {
        plugin: PluginId,
        scope: ConfigScope,
        value: Value,
    },
    /// Sets one field of a settings scope from text, typed by the plugin's schema.
    SetConfigField {
        plugin: PluginId,
        scope: ConfigScope,
        key: String,
        value: String,
    },
    /// Sets one option of the current theme.
    SetThemeOption {
        key: String,
        value: String,
    },
    RunAction {
        plugin: PluginId,
        action: String,
        game: Option<GameKey>,
        args: Value,
    },
    /// Turns a plugin on or off. A theme stays on, and an emulator that is busy can't be
    /// turned off.
    SetPluginEnabled {
        plugin: PluginId,
        enabled: bool,
    },
    /// Maps a named host folder to these folders. Empty goes back to the plugins' default.
    SetFolders {
        name: String,
        paths: Vec<std::path::PathBuf>,
    },
    /// Theme sounds on or off.
    SetSounds(bool),
    /// The launcher's window full screen, or a normal window.
    SetFullscreen(bool),
    /// Whether the launcher comes back to the front when a game closes.
    SetReturnOnExit(bool),
    /// The size of everything the theme draws, from 0.8 to 1.4 (1 is the theme's own size).
    /// A value outside that range goes to its nearest end.
    SetScale(f32),
    /// The launcher starts when the player logs in, or not. When the system refuses, the
    /// setting stays as it was and says why.
    SetStartAtLogin(bool),
    /// The UI host finished swapping to this theme.
    ThemeSwitched(PluginId),
    /// Closes every notice.
    DismissNotices,
    /// Closes the notice with this id (`NoticeVm::id`); the others stay.
    DismissNotice(u64),
    /// Looks for emulator updates and, if automatic updates are on, installs them. An update
    /// for an emulator whose game is running waits until the game closes. The core runs this
    /// every few hours by itself.
    CheckUpdates,
    /// Automatic emulator updates on or off.
    SetAutoUpdate(bool),
    /// The player pressed Start on the Welcome screen: it doesn't show again.
    FinishWelcome,
    /// The monitor the launcher goes on, by name; `None` for the one it is on.
    SetMonitor(Option<String>),
    /// The notice with this id was clicked: open what it is about, and dismiss it.
    OpenNotice(u64),
    /// Reads the newest release of the launcher (`app.update_repo`).
    CheckLauncherUpdate,
    /// Downloads the newer launcher the check found, for the next start.
    DownloadLauncherUpdate,
    /// "Rate how it runs": asks "How far did it get?" for this game.
    AskResult(GameKey),
    /// The player's answer to "How far did it get?" after playing: `ingame`, `menus`,
    /// `boots` or `nothing`, or `None` for "Not now". It is the player's own result for the
    /// game, saved, and shown with its compatibility.
    ReportResult(Option<String>),
    /// The Library's search text.
    LibrarySearch(String),
    /// The Library's system (a platform label, such as `PS5`), or every system. `INGAME_FILTER`
    /// turns the "In-game" filter on or off instead; `None` clears both.
    LibrarySystem(Option<String>),
    /// The Library's genre, or every genre.
    LibraryGenre(Option<String>),
    /// How the Library is sorted: `title`, `played`, `size` (largest first), `smallest`,
    /// `rating`, `release` (newest first), `compat` or `added`.
    LibrarySort(String),
    /// The Library shows its next page of games ([`GRID_PAGE`] more).
    LibraryMore,
    /// The Catalog's search text, system, genre and sort, as the Library's.
    CatalogSearch(String),
    CatalogSystem(Option<String>),
    /// Hides a game the Catalog offers (`hidden`), or shows it again. Hidden games leave the
    /// Catalog, but for the "Hidden" filter (`HIDDEN_FILTER`). Kept in the config.
    HideGame {
        game: GameKey,
        hidden: bool,
    },
    CatalogGenre(Option<String>),
    CatalogSort(String),
    /// The Catalog shows its next page of games.
    CatalogMore,
    /// How many columns a grid shows, so the core knows the games above and below the
    /// selection.
    GridColumns {
        grid: Grid,
        columns: usize,
    },
    /// Downloads a release of a game the catalogs offer: the player agreed. It reads the
    /// torrent's files first and waits for the player to start it.
    DownloadRelease {
        offer: GameKey,
        /// The release's id, its torrent's info hash.
        release: String,
        /// The folder the player picked, inside a packages folder (`~/` for the home folder).
        /// `None`: the last one picked for the console, or else the console's folder in the
        /// first packages folder.
        folder: Option<std::path::PathBuf>,
    },
    /// Installs a finished download into the game folder the player picked on the install
    /// sheet (`~/` for the home folder). `None`: the last one picked for the console, or else
    /// the console's folder in the first game folder. The folder must be inside a game folder.
    InstallDownload {
        id: String,
        folder: Option<std::path::PathBuf>,
    },
    /// The player's action on a download, by its id.
    Torrent {
        id: String,
        action: crate::TorrentAction,
    },
    /// "Keep sharing finished downloads": on seeds them; off stops every one.
    SetSeedDownloads(bool),
    /// "Download only through a VPN". On is refused where the system can't bind downloads to
    /// a VPN, and says why.
    SetVpnOnly(bool),
    /// The VPN downloads use, by its name. Empty: the first connected one, where the system
    /// finds VPNs.
    SetVpn(String),
    /// Connects or disconnects a VPN the system can switch, by its name.
    ConnectVpn(String),
    DisconnectVpn(String),
    /// Adds a VPN from a provider's file (WireGuard `.conf`, OpenVPN `.ovpn`) and connects it.
    /// `~/` is the home folder.
    ImportVpn(std::path::PathBuf),
    /// Something the player asked of the machine: Wi-Fi, Bluetooth, sound, power, Nexus OS.
    System(crate::system::SystemCommand),
    /// The player agreed to install the emulator a game needs: install it, then play.
    ConfirmInstall,
    /// The player said "not now" to that.
    CancelInstall,
}

pub use nexus_api::HOST_INSTALL_GAME;

/// The smallest and the largest size of everything ([`Command::SetScale`]).
pub const MIN_SCALE: f32 = 0.8;
pub const MAX_SCALE: f32 = 1.4;

/// How often the core looks for emulator updates by itself, and how long after start it
/// looks first.
pub const UPDATE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);
const FIRST_UPDATE_CHECK: std::time::Duration = std::time::Duration::from_secs(60);

/// A first play waiting for the player to agree to install its emulator.
#[derive(Clone, Debug)]
pub(crate) struct InstallPrompt {
    pub game: GameKey,
    pub emulator: String,
    pub version: String,
    pub title: String,
    pub size_bytes: Option<u64>,
}

/// An install in progress, saved under `app.installs.<plugin>` so a crash resumes it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct InstallRecord {
    version: String,
    from: Option<String>,
    phase: InstallPhase,
}

/// A download in progress, saved under `app.downloads` so a crash resumes it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct DownloadRecord {
    plugin: PluginId,
    request: DownloadRequest,
}

impl DownloadRecord {
    /// The same request for the same plugin is saved once.
    fn key(&self) -> String {
        serde_json::to_string(self).expect("records serialize")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub key: GameKey,
    pub summary: GameSummary,
}

/// Where a game's folder is on disk, through its plugin's folder grants; `None` when the
/// grants refuse it or it isn't a folder.
/// The player's folders as `game`'s plugin may use them.
fn game_folders(kernel: &Kernel, game: &GameKey) -> Option<crate::folders::HostFolders> {
    plugin_folders(kernel, &game.plugin)
}

/// The player's folders as `plugin` may use them.
pub(crate) fn plugin_folders(kernel: &Kernel, plugin: &PluginId) -> Option<crate::folders::HostFolders> {
    Some(crate::folders::HostFolders {
        plugin: plugin.clone(),
        permissions: kernel.manifest(plugin).ok()?.permissions.clone(),
        config: kernel.config().clone(),
        home: kernel.host().home.clone(),
        platform: kernel.host().platform,
    })
}

pub(crate) fn resolve_game_folder(
    kernel: &Kernel,
    game: &GameKey,
    folder: &nexus_api::HostPath,
) -> Option<std::path::PathBuf> {
    game_folders(kernel, game)?.resolve(folder).ok().filter(|p| p.is_dir())
}

/// The Library's "In-game" filter, among the systems: `Command::LibrarySystem` with this id.
pub const INGAME_FILTER: &str = "ingame";

/// The Catalog's "Installed" filter, among the systems: `Command::CatalogSystem` with this id.
pub const INSTALLED_FILTER: &str = "installed";

/// The Catalog's "Hidden" filter, among the systems: only the games the player hid.
pub const HIDDEN_FILTER: &str = "hidden";

#[derive(Clone, Debug, Default)]
pub(crate) struct Detail {
    pub details: Option<GameDetails>,
    pub compat: Option<CompatVm>,
    /// The community list's page, from the compatibility answer.
    pub compat_page: String,
    pub actions: Vec<SlotVm>,
    pub sections: Vec<SlotVm>,
    pub shaders: Option<nexus_api::ShaderCacheInfo>,
    /// The game's own settings, when its emulator has any.
    pub settings: Option<SettingsPanelVm>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct EmulatorState {
    pub installed: Vec<String>,
    /// The installed build is the player's own: it never updates by itself.
    pub own: bool,
    pub available: Vec<String>,
    pub busy: bool,
    pub panels: Vec<SlotVm>,
    pub controllers: Vec<nexus_api::ControllerProfile>,
}

/// The Library's search, filters and sort.
#[derive(Clone, Debug)]
pub(crate) struct LibraryView {
    pub query: String,
    pub system: Option<String>,
    /// Only games that reach gameplay: the community's result or the player's own.
    pub ingame: bool,
    /// Only the games the player has: in the Catalog, the offers of installed games.
    pub installed: bool,
    /// Only the games the player hid; else the Catalog leaves them out.
    pub hidden: bool,
    pub genre: Option<String>,
    pub sort: String,
    /// How many games show: pages of [`GRID_PAGE`], more as the player scrolls.
    pub shown: usize,
}

/// How many more games a Library or Catalog page shows each time.
pub const GRID_PAGE: usize = 60;

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            query: String::new(),
            system: None,
            ingame: false,
            installed: false,
            hidden: false,
            genre: None,
            sort: "title".into(),
            shown: GRID_PAGE,
        }
    }
}

impl LibraryView {
    /// The Catalog's: newest added first.
    fn catalog() -> Self {
        Self { sort: "added".into(), ..Self::default() }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Session {
    pub game: GameKey,
    pub title: String,
    pub stopping: bool,
    /// The game's window came to the front, or the launcher stopped waiting for one.
    pub shown: bool,
    /// The system's id for the game's process.
    pub pid: Option<u32>,
    pub started: std::time::Instant,
    /// Where the game's output goes.
    pub log: std::path::PathBuf,
    /// The emulator build the game runs on, when the emulator has builds.
    pub build: Option<String>,
}

/// How long a selection must stay before its details load.
const SELECT_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
/// How many games the store is asked about at once.
const DESCRIBE_AT_ONCE: usize = 6;
/// The longest the boot waits for the store's descriptions.
const BOOT_DESCRIBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) struct AppState {
    pub nav: Vec<Route>,
    pub selected: Option<GameKey>,
    pub library: Vec<Entry>,
    pub badges: HashMap<GameKey, Vec<SlotVm>>,
    pub details: HashMap<GameKey, Detail>,
    /// Games whose details failed to load on select. They load again on open.
    pub detail_failed: HashSet<GameKey>,
    /// Each emulator's builds and blocks, and a service plugin's blocks once its page opens.
    pub emulators: BTreeMap<PluginId, EmulatorState>,
    pub settings: Vec<SettingsPanelVm>,
    pub theme: Option<PluginId>,
    pub sessions: BTreeMap<SessionId, Session>,
    /// The last window the launcher brought to the front was the game's. It stands in for
    /// the desktop when the desktop doesn't say which window is in front.
    pub game_in_front: bool,
    /// A launch is between its checks and its session: no other game may start.
    pub launching: bool,
    /// The first library load, with the first game's details, is done.
    pub booted: bool,
    /// What a store's catalog says about each game, and the plugin that said it.
    pub meta: HashMap<GameKey, (PluginId, nexus_api::GameMetadata)>,
    /// Work in the background, as the old launcher's status pill: "Downloading game art 3/40".
    pub task: Option<String>,
    /// The Library's search, filters and sort.
    pub library_view: LibraryView,
    /// The Catalog's.
    pub catalog_view: LibraryView,
    /// A game that just closed, to ask the player how far it got, and the emulator build it
    /// ran on.
    pub result_prompt: Option<(GameKey, Option<String>)>,
    /// This PC, once the check ran.
    pub hardware: Option<crate::Specs>,
    /// The launcher's own update.
    pub launcher_update: crate::launcher_update::UpdateState,
    /// The machine's settings, from the system plugins.
    pub system: crate::system::SystemState,
    /// Why the system refused the last change to "Start at login".
    pub login_error: Option<String>,
    /// The store's descriptions: how many of how many are in. `None` without a store.
    pub describing: Option<(usize, usize)>,
    /// The store has been asked about every game of the library.
    pub described: bool,
    /// What the player is told, newest last: at most `MAX_NOTICES`.
    pub notices: Vec<Notice>,
    /// The id the next notice gets.
    pub next_notice: u64,
    pub loading: bool,
    pub install_prompt: Option<InstallPrompt>,
    /// Emulators with an update that waits for their game to close.
    pub pending_updates: BTreeSet<PluginId>,
    pub game_installs: BTreeMap<JobId, crate::game_installs::GameJob>,
    /// Each emulator's catalog, from each catalog plugin.
    pub catalogs: Vec<crate::releases::CatalogCopy>,
    /// The catalog plugins answered once: until then the Catalog says it is loading.
    pub catalogs_read: bool,
    /// The catalog plugins' Catalog buttons and badges (`catalog.action`), as read with the
    /// catalogs.
    pub catalog_buttons: Vec<SlotVm>,
    /// The games the catalogs offer, newest first.
    pub offers: Vec<crate::releases::Offer>,
    /// The cover of the offer each installed game is, with the catalog plugin it is from: the
    /// game's art when it has none of its own (a PC game). Kept in `app.game_art`, so it stays
    /// with the catalog off.
    pub offer_art: HashMap<GameKey, (PluginId, nexus_api::ImageSource)>,
    /// The offer each installed game is, by the game: its Game Hub has the offer's Steam data.
    pub installed_offers: HashMap<GameKey, GameKey>,
    /// The catalogs' downloads, by info hash.
    pub torrents: BTreeMap<String, crate::torrents::TorrentJob>,
    /// The running downloads are being read.
    pub torrent_watch: bool,
    /// Why the last download the player asked for on a game's Game Hub didn't start, for its
    /// download sheet; gone at the next try or when a Game Hub opens.
    pub download_error: Option<(GameKey, String)>,
    /// The VPNs, as last read.
    pub vpns: Vec<crate::Vpn>,
    /// The interface the torrent port is bound to; `None` for any.
    pub vpn_bound: Option<String>,
    /// The kill switch is looking at the VPN.
    pub vpn_checking: bool,
    /// Why the last connect, disconnect or import of a VPN failed.
    pub vpn_error: Option<String>,
    /// The game selected in the Catalog. Home's selection stays a library game.
    pub catalog_selected: Option<GameKey>,
    /// The Catalog's grid as last built: on another screen, each view shows this one instead
    /// of searching and sorting every offer again.
    pub catalog_grid: Option<nexus_ui_api::LibraryVm>,
    /// The games the stores were asked about this run, so none is asked twice.
    pub asked: HashSet<GameKey>,
    /// Each grid's columns.
    pub grid_columns: HashMap<Grid, usize>,
    /// The art being fetched around the selection; the next select stops it.
    pub prefetch: Option<tokio::task::AbortHandle>,
    /// The image addresses already fetched ahead this run, so none is fetched twice.
    pub prefetched: HashSet<String>,
}

pub(crate) struct Inner {
    pub kernel: Kernel,
    pub state: Mutex<AppState>,
    pub downloads: Downloads,
    pub next_session: AtomicU64,
    pub stops: Mutex<HashMap<SessionId, tokio::sync::oneshot::Sender<()>>>,
    /// The desktop was asked what it is missing, at the first game of this run.
    pub desktop_checked: std::sync::atomic::AtomicBool,
}

/// The app. Cheap to clone; every clone is the same app.
#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
}

/// A short message for the player, until it is dismissed.
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

impl AppState {
    /// Shows the player `message`. Clicking it opens `route`, when there is one. The oldest
    /// notice goes when there are more than `MAX_NOTICES`.
    pub(crate) fn post_notice(
        &mut self,
        kind: NoticeKind,
        message: String,
        route: Option<Route>,
        game: Option<GameKey>,
    ) {
        // Only a problem is a warning in the log: "Download complete" is news.
        if kind == NoticeKind::Error {
            tracing::warn!(%message, "notice");
        } else {
            tracing::info!(%message, kind = kind.as_str(), "notice");
        }
        self.next_notice += 1;
        self.notices.push(Notice { id: self.next_notice, message, kind, route, game });
        if self.notices.len() > nexus_ui_api::MAX_NOTICES {
            self.notices.remove(0);
        }
    }
}

impl Core {
    /// Starts the app on a booted kernel and resumes installs a crash interrupted.
    pub async fn start(kernel: Kernel) -> Core {
        let theme = kernel.config().get(&["app", "theme"]).as_str().and_then(|s| PluginId::new(s).ok());
        let state = AppState {
            nav: vec![Route::Home],
            selected: None,
            library: Vec::new(),
            badges: HashMap::new(),
            details: HashMap::new(),
            detail_failed: HashSet::new(),
            emulators: BTreeMap::new(),
            settings: Vec::new(),
            theme,
            sessions: BTreeMap::new(),
            game_in_front: false,
            launching: false,
            booted: false,
            meta: HashMap::new(),
            offer_art: Self::kept_offer_art(&kernel),
            installed_offers: HashMap::new(),
            task: None,
            library_view: LibraryView::default(),
            catalog_view: LibraryView::catalog(),
            result_prompt: None,
            hardware: None,
            launcher_update: crate::launcher_update::UpdateState::Idle,
            system: Default::default(),
            login_error: None,
            describing: None,
            described: false,
            notices: Vec::new(),
            next_notice: 0,
            loading: false,
            install_prompt: None,
            pending_updates: BTreeSet::new(),
            game_installs: BTreeMap::new(),
            catalogs: Vec::new(),
            catalogs_read: false,
            catalog_buttons: Vec::new(),
            offers: Vec::new(),
            asked: HashSet::new(),
            catalog_selected: None,
            catalog_grid: None,
            torrents: BTreeMap::new(),
            torrent_watch: false,
            download_error: None,
            vpns: Vec::new(),
            vpn_bound: None,
            vpn_checking: false,
            vpn_error: None,
            grid_columns: HashMap::new(),
            prefetch: None,
            prefetched: HashSet::new(),
        };
        let downloads = Downloads::spawn(kernel.clone());
        let core = Core {
            inner: Arc::new(Inner {
                kernel,
                state: Mutex::new(state),
                downloads,
                next_session: AtomicU64::new(0),
                stops: Mutex::new(HashMap::new()),
                desktop_checked: Default::default(),
            }),
        };
        core.clean_up_game_installs();
        crate::system::adopt_vpn(&core).await;
        core.restore_torrents();
        core.resume_installs().await;
        core.resume_downloads();
        core.schedule_update_checks();
        crate::system::schedule_refresh(&core);
        core.check_hardware();
        core.watch_catalogs();
        core
    }

    /// Reads this PC once, off the UI thread: the system's tools can take a moment.
    fn check_hardware(&self) {
        let core = self.clone();
        tokio::spawn(async move {
            let hardware = core.inner.kernel.hardware().clone();
            let Ok(specs) = tokio::task::spawn_blocking(move || hardware.specs()).await else { return };
            core.notice_blocked(&specs);
            core.state().hardware = Some(specs);
            core.changed();
        });
    }

    /// Once for each PC: a notice when an emulator's games won't start here, before the
    /// player downloads one. The config keeps the PC it was for.
    fn notice_blocked(&self, specs: &crate::Specs) {
        let kernel = &self.inner.kernel;
        let summary = crate::hardware::summary(specs);
        if kernel.config().get(&["app", "pc_noticed"]).as_str() == Some(summary.as_str()) {
            return;
        }
        let blocked: Vec<(String, String)> = kernel
            .of_kind(PluginKind::Emulator)
            .into_iter()
            .filter_map(|id| kernel.manifest(&id).ok())
            .filter(|m| m.capabilities.contains(&Capability::Launcher))
            .filter_map(|m| match crate::hardware::verdict(specs, &m.requires) {
                crate::hardware::Verdict::No(why) => Some((m.name.clone(), why)),
                _ => None,
            })
            .collect();
        let Some(message) = crate::hardware::blocked_notice(&blocked) else { return };
        self.notice_opening(NoticeKind::Error, Route::Settings, message);
        if let Err(e) = kernel.config().set(&["app", "pc_noticed"], Value::String(summary)) {
            tracing::warn!(error = %e, "could not save the PC the notice was for");
        }
    }

    pub fn kernel(&self) -> &Kernel {
        &self.inner.kernel
    }

    pub fn events(&self) -> BoxStream<'static, Event> {
        self.inner.kernel.events().stream()
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, AppState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn changed(&self) {
        self.inner.kernel.events().publish(Event::StateChanged);
    }

    /// A notice of the default kind, `Info`.
    pub(crate) fn notice(&self, message: String) {
        self.state().post_notice(NoticeKind::Info, message, None, None);
    }

    /// A notice of the `Error` kind: something failed.
    pub(crate) fn problem(&self, message: String) {
        self.state().post_notice(NoticeKind::Error, message, None, None);
    }

    /// A notice whose subject `route` shows: clicking it opens that screen.
    pub(crate) fn notice_opening(&self, kind: NoticeKind, route: Route, message: String) {
        self.state().post_notice(kind, message, Some(route), None);
    }

    /// A problem with one game: its card shows the game's art.
    pub(crate) fn game_problem(&self, game: &GameKey, message: String) {
        self.state().post_notice(NoticeKind::Error, message, None, Some(game.clone()));
    }

    /// Shows the player a problem outside the core, such as a theme's logic failing.
    pub fn report(&self, message: String) {
        self.state().post_notice(NoticeKind::Error, message, None, None);
        self.changed();
    }

    /// The view models for the current state.
    pub fn view(&self) -> AppView {
        view::build(self)
    }

    /// The theme the player picked last, if any.
    pub fn selected_theme(&self) -> Option<PluginId> {
        self.state().theme.clone()
    }

    /// Stored settings for a scope, or the plugin's defaults when nothing is stored.
    pub async fn config_value(&self, plugin: &PluginId, scope: &ConfigScope) -> CoreResult<Value> {
        let stored =
            self.inner.kernel.config().get(&["plugins", plugin.as_str(), &crate::ConfigService::scope_key(scope)]);
        if !stored.is_null() {
            return Ok(stored);
        }
        let provided = self.inner.kernel.provided(plugin).await?;
        match provided.configurator {
            Some(c) => self.inner.kernel.guard(plugin, "configurator.defaults", c.defaults(scope)).await,
            None => Ok(Value::Null),
        }
    }

    /// Closes down before the app quits: the torrents stop first, so each writes down the
    /// pieces it has, then every plugin is deactivated. The downloads stay as they were, so
    /// the next launch starts them again. Call it while the async runtime still runs.
    pub async fn shutdown(&self) {
        if let Err(error) = self.inner.kernel.torrents().stop_all().await {
            tracing::warn!(%error, "the torrents didn't stop");
        }
        self.inner.kernel.shutdown().await;
    }

    pub async fn dispatch(&self, command: Command) -> CoreResult<()> {
        tracing::debug!(?command, "dispatch");
        let result = match command {
            Command::Navigate(route) => self.navigate(route).await,
            Command::Back => {
                {
                    let mut state = self.state();
                    if state.nav.len() > 1 {
                        state.nav.pop();
                    }
                }
                self.changed();
                Ok(())
            }
            Command::Select(game) => {
                self.select(&game);
                self.changed();
                self.prefetch_around(&game);
                // Themes show the selected game's details on the home screen, so load them
                // once. Wait first: a player who scrolls past a game doesn't need its details.
                // A plugin that fails here leaves one notice; the selection stands.
                let wanted = {
                    let state = self.state();
                    !state.details.contains_key(&game) && !state.detail_failed.contains(&game)
                };
                if !wanted {
                    return Ok(());
                }
                tokio::time::sleep(SELECT_SETTLE).await;
                let still = {
                    let state = self.state();
                    state.selected.as_ref() == Some(&game) || state.catalog_selected.as_ref() == Some(&game)
                };
                if !still {
                    return Ok(());
                }
                match self.load_detail(&game).await {
                    Ok(()) => self.changed(),
                    Err(e) => {
                        self.state().detail_failed.insert(game);
                        self.problem(e.to_string());
                    }
                }
                Ok(())
            }
            Command::OpenGame(game) => {
                self.state().download_error = None;
                self.open_game(game).await
            }
            Command::RefreshLibrary => self.refresh_library().await,
            Command::UninstallGame(game) => self.uninstall_game(&game).await,
            Command::Search(text) => self.search(text).await,
            Command::Download { plugin, request } => self.download(&plugin, request).await,
            Command::PauseDownload(job) => self.inner.downloads.pause(job).await,
            Command::ResumeDownload(job) => self.inner.downloads.resume(job).await,
            Command::CancelDownload(job) if self.cancel_game_install(job) => Ok(()),
            Command::CancelDownload(job) => self.inner.downloads.cancel(job).await,
            Command::InstallEmulator { plugin, version } => self.install(&plugin, version, None).await,
            Command::UpdateEmulator { plugin } => self.update(&plugin).await,
            Command::UninstallEmulator { plugin, version } => self.uninstall(&plugin, &version).await,
            Command::Launch(game) => {
                let result = crate::launch::launch(self, game.clone()).await;
                // The player pressed Play: say why nothing started.
                if let Err(error) = &result {
                    let why = crate::launch::cant_start(self, &game, error);
                    self.notice_opening(NoticeKind::Error, Route::GameDetail(game.clone()), why);
                }
                result
            }
            Command::Stop(session) => crate::launch::stop(self, session),
            Command::ShowGame(session) => crate::launch::show_game(self, session).await,
            Command::ToggleFocus => {
                crate::launch::toggle_focus(self).await;
                Ok(())
            }
            Command::ClearShaderCache(game) => self.clear_shader_cache(game).await,
            Command::SetConfig { plugin, scope, value } => self.set_config(&plugin, scope, value).await,
            Command::SetConfigField { plugin, scope, key, value } => {
                self.set_config_field(&plugin, scope, key, value).await
            }
            Command::SetThemeOption { key, value } => self.set_theme_option(key, value),
            Command::RunAction { plugin, action, game, args } => self.run_action(&plugin, &action, game, args).await,
            Command::ThemeSwitched(theme) => self.theme_switched(theme),
            Command::SetFolders { name, paths } => self.set_folders(&name, &paths),
            Command::SetPluginEnabled { plugin, enabled } => self.set_plugin_enabled(&plugin, enabled).await,
            Command::SetSounds(on) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "sounds"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::SetFullscreen(on) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "fullscreen"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::ReportResult(level) => {
                let Some((game, build)) = self.state().result_prompt.take() else { return Ok(()) };
                let config = self.inner.kernel.config();
                let set = |key: &str, value: Value| {
                    config.set(&["app", key, &game.to_string()], value).map_err(|e| CoreError::Io(e.to_string()))
                };
                let build = build.map(Value::String).unwrap_or(Value::Null);
                match level {
                    Some(level) => {
                        if !matches!(level.as_str(), "ingame" | "menus" | "boots" | "nothing") {
                            return Err(CoreError::Invalid(format!("no result {level:?}")));
                        }
                        set("results", Value::String(level))?;
                        set("result_builds", build)?;
                    }
                    None => set("result_skipped", build)?,
                }
                self.changed();
                Ok(())
            }
            Command::LibrarySearch(text) => self.set_grid(false, |v| v.query = text),
            Command::LibrarySystem(system) => self.set_grid(false, |v| v.set_system(system)),
            Command::LibraryGenre(genre) => self.set_grid(false, |v| v.genre = genre),
            Command::LibrarySort(sort) => {
                check_sort(&sort)?;
                self.set_grid(false, |v| v.sort = sort)
            }
            Command::LibraryMore => self.more(false),
            Command::GridColumns { grid, columns } => {
                self.state().grid_columns.insert(grid, columns);
                Ok(())
            }
            Command::CatalogSearch(text) => self.set_grid(true, |v| v.query = text),
            Command::CatalogSystem(system) => self.set_grid(true, |v| v.set_system(system)),
            Command::HideGame { game, hidden } => self.hide_game(&game, hidden),
            Command::CatalogGenre(genre) => self.set_grid(true, |v| v.genre = genre),
            Command::CatalogSort(sort) => {
                check_sort(&sort)?;
                self.set_grid(true, |v| v.sort = sort)
            }
            Command::CatalogMore => self.more(true),
            Command::DownloadRelease { offer, release, folder } => {
                self.state().download_error = None;
                let started = self.download_release(&offer, &release, folder.as_deref()).await;
                // The sheet that asked says why nothing downloads, and so does a notice, for
                // when the player closed the sheet.
                if let Err(e) = &started {
                    self.state().download_error = Some((offer, e.to_string()));
                    self.problem(e.to_string());
                }
                started
            }
            Command::Torrent { id, action } => self.torrent_action(&id, action).await,
            Command::InstallDownload { id, folder } => self.install_download(&id, folder.as_deref()).await,
            Command::SetSeedDownloads(on) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "seed_downloads"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                if !on {
                    self.stop_all_seeding().await;
                }
                self.changed();
                Ok(())
            }
            Command::SetVpnOnly(on) => {
                if on && let Some(why) = self.inner.kernel.network().problem() {
                    return Err(CoreError::Invalid(why));
                }
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "vpn_only"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.vpn_changed().await;
                Ok(())
            }
            Command::SetVpn(name) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "vpn"], Value::String(name))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.vpn_changed().await;
                Ok(())
            }
            Command::ConnectVpn(name) => self.vpn_call(move |n| n.connect(&name)).await,
            Command::DisconnectVpn(name) => self.vpn_call(move |n| n.disconnect(&name)).await,
            Command::ImportVpn(file) => {
                let home = self.inner.kernel.host().home.clone();
                let file = crate::folders::expand(&file.to_string_lossy(), home.as_deref())
                    .ok_or_else(|| CoreError::Invalid(format!("{} isn't a full path", file.display())))?;
                self.vpn_call(move |n| n.import(&file)).await.map(|_| ())
            }
            Command::AskResult(game) => {
                let build = self.build_of(&game.plugin);
                self.state().result_prompt = Some((game, build));
                self.changed();
                Ok(())
            }
            Command::SetMonitor(name) => {
                let value = name.map(Value::String).unwrap_or(Value::Null);
                self.inner.kernel.config().set(&["app", "monitor"], value).map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::FinishWelcome => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "welcomed"], Value::Bool(true))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::SetReturnOnExit(on) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "return_on_exit"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::System(command) => crate::system::run(self, command).await,
            Command::DismissNotices => {
                self.state().notices.clear();
                self.changed();
                Ok(())
            }
            Command::DismissNotice(id) => {
                self.state().notices.retain(|n| n.id != id);
                self.changed();
                Ok(())
            }
            Command::CheckLauncherUpdate => {
                use crate::launcher_update::{UpdateState, check};
                self.state().launcher_update = UpdateState::Checking;
                self.changed();
                let found = check(self, env!("CARGO_PKG_VERSION"), self.kernel().host().install).await;
                // Nexus OS brings the new launcher with its next update: look for that now.
                let with_system = matches!(found, UpdateState::WithSystem { .. });
                self.state().launcher_update = found;
                self.changed();
                if with_system {
                    let core = self.clone();
                    tokio::spawn(async move {
                        let _ = crate::system::run(&core, crate::system::SystemCommand::CheckOsUpdate).await;
                    });
                }
                Ok(())
            }
            Command::DownloadLauncherUpdate => {
                use crate::launcher_update::{UpdateState, update};
                let UpdateState::Available { version, asset } = self.state().launcher_update.clone() else {
                    return Err(CoreError::Invalid("no launcher update to download".into()));
                };
                self.state().launcher_update = UpdateState::Downloading { version: version.clone() };
                self.changed();
                let done = update(self, &version, &asset, self.kernel().host().install).await;
                self.state().launcher_update = done.unwrap_or_else(|e| UpdateState::Failed(e.to_string()));
                self.changed();
                Ok(())
            }
            Command::OpenNotice(id) => {
                let route = {
                    let mut state = self.state();
                    let route = state.notices.iter().find(|n| n.id == id).and_then(|n| n.route.clone());
                    state.notices.retain(|n| n.id != id);
                    route
                };
                match route {
                    Some(route) => self.navigate(route).await,
                    None => {
                        self.changed();
                        Ok(())
                    }
                }
            }
            Command::CheckUpdates => {
                self.check_updates().await;
                Ok(())
            }
            Command::SetScale(scale) => {
                if !scale.is_finite() {
                    return Err(CoreError::Invalid(format!("{scale} is not a size")));
                }
                let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
                self.inner
                    .kernel
                    .config()
                    // Two decimals: 1.2, not the f32's 1.2000000476837158.
                    .set(&["app", "scale"], Value::from((f64::from(scale) * 100.0).round() / 100.0))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::SetStartAtLogin(on) => {
                let desktop = self.inner.kernel.desktop().clone();
                if !desktop.can_start_at_login() {
                    return Err(CoreError::Invalid("this build can't start when you log in".into()));
                }
                let done = tokio::task::spawn_blocking(move || desktop.start_at_login(on))
                    .await
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                if let Err(error) = done {
                    self.state().login_error = Some(error.clone());
                    self.changed();
                    return Err(CoreError::Invalid(error));
                }
                self.state().login_error = None;
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "start_at_login"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::SetAutoUpdate(on) => {
                self.inner
                    .kernel
                    .config()
                    .set(&["app", "auto_update"], Value::Bool(on))
                    .map_err(|e| CoreError::Io(e.to_string()))?;
                self.changed();
                Ok(())
            }
            Command::ConfirmInstall => {
                let Some(prompt) = self.state().install_prompt.take() else { return Ok(()) };
                self.changed();
                self.install(&prompt.game.plugin, Some(prompt.version), None).await?;
                crate::launch::launch(self, prompt.game).await
            }
            Command::CancelInstall => {
                self.state().install_prompt = None;
                self.changed();
                Ok(())
            }
        };
        if let Err(error) = &result {
            tracing::warn!(%error, "command failed");
        }
        result
    }

    /// The Catalog shows its "Hidden" filter: unless a plugin that gives catalogs turned it off
    /// (its `show_hidden_filter` setting).
    pub(crate) fn hidden_filter_on(kernel: &Kernel) -> bool {
        kernel.providers(Capability::Releases).iter().all(|plugin| {
            kernel.config().get(&["plugins", plugin.as_str(), "emulator", "show_hidden_filter"]).as_bool()
                != Some(false)
        })
    }

    /// The games the player hid from the Catalog.
    pub(crate) fn hidden_games(kernel: &Kernel) -> HashSet<String> {
        let list = kernel.config().get(&["app", "hidden_games"]);
        list.as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
    }

    fn hide_game(&self, game: &GameKey, hidden: bool) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let mut list: Vec<String> = Self::hidden_games(kernel).into_iter().filter(|k| *k != game.to_string()).collect();
        if hidden {
            list.push(game.to_string());
        }
        list.sort();
        let saved = if list.is_empty() {
            kernel.config().remove(&["app", "hidden_games"])
        } else {
            kernel.config().set(&["app", "hidden_games"], serde_json::json!(list))
        };
        saved.map_err(|e| CoreError::Io(e.to_string()))?;
        self.changed();
        Ok(())
    }

    /// The offers' covers kept for installed games (see [`AppState::offer_art`]).
    fn kept_offer_art(kernel: &Kernel) -> HashMap<GameKey, (PluginId, nexus_api::ImageSource)> {
        let kept = kernel.config().get(&["app", "game_art"]);
        let Some(kept) = kept.as_object() else { return HashMap::new() };
        kept.iter()
            .filter_map(|(game, art)| {
                let plugin = PluginId::new(art.get("plugin")?.as_str()?).ok()?;
                let image = serde_json::from_value(art.get("image")?.clone()).ok()?;
                Some((game.parse().ok()?, (plugin, image)))
            })
            .collect()
    }

    /// Finds the offer each installed game is ([`AppState::installed_offers`]), gives the game
    /// its cover (see [`AppState::offer_art`]), and keeps the covers that changed.
    pub(crate) fn remember_offer_art(&self) {
        let changed = {
            let mut state = self.state();
            let found = view::installed_offers(&state);
            state.installed_offers = found.iter().map(|(game, offer, _)| (game.clone(), offer.clone())).collect();
            let art: Vec<_> = found.into_iter().filter_map(|(game, _, art)| Some((game, art?))).collect();
            let changed = art.iter().any(|(game, art)| state.offer_art.get(game) != Some(art));
            state.offer_art.extend(art);
            changed.then(|| state.offer_art.clone())
        };
        let Some(art) = changed else { return };
        let kept: serde_json::Map<String, Value> = art
            .into_iter()
            .map(|(game, (plugin, image))| (game.to_string(), serde_json::json!({ "plugin": plugin, "image": image })))
            .collect();
        if let Err(e) = self.inner.kernel.config().set(&["app", "game_art"], Value::Object(kept)) {
            tracing::warn!(error = %e, "could not keep the installed games' art");
        }
    }

    /// A turned-on plugin gives release catalogs, so there is a Catalog.
    pub(crate) fn catalog_available(kernel: &Kernel) -> bool {
        !kernel.providers(Capability::Releases).is_empty()
    }

    async fn navigate(&self, route: Route) -> CoreResult<()> {
        if route == Route::Catalog && !Self::catalog_available(&self.inner.kernel) {
            return Err(CoreError::Invalid("no game catalog is turned on".into()));
        }
        match &route {
            Route::GameDetail(game) => return self.open_game(game.clone()).await,
            Route::Emulators => self.refresh_emulators().await,
            Route::Plugin(plugin) => {
                let manifest = self.inner.kernel.manifest(plugin)?;
                // A service's blocks show on its page too.
                if manifest.kind != PluginKind::Theme {
                    self.refresh_emulator(plugin).await;
                }
                self.refresh_settings().await;
            }
            Route::Settings => {
                self.refresh_vpns().await;
                self.refresh_settings().await;
            }
            // Read once; a new catalog address reads them again. The Catalog opens at once and
            // says it is loading until the read ends.
            Route::Catalog if self.state().catalogs.is_empty() => {
                self.push(route);
                self.refresh_releases().await;
                return Ok(());
            }
            Route::GameSettings(game) if !self.state().details.contains_key(game) => self.load_detail(game).await?,
            _ => {}
        }
        self.push(route);
        Ok(())
    }

    pub(crate) fn push(&self, route: Route) {
        {
            let mut state = self.state();
            // One plugin's page at a time: opening another replaces it.
            if matches!(route, Route::Plugin(_)) && matches!(state.nav.last(), Some(Route::Plugin(_))) {
                state.nav.pop();
            }
            if state.nav.last() != Some(&route) {
                state.nav.push(route);
            }
        }
        self.changed();
    }

    /// Activates every plugin with `capability`, skipping (with a notice) those that fail.
    pub(crate) async fn providers(&self, capability: Capability) -> Vec<(PluginId, Provided)> {
        let mut out = Vec::new();
        for plugin in self.inner.kernel.providers(capability) {
            match self.inner.kernel.provided(&plugin).await {
                Ok(provided) => out.push((plugin, provided)),
                Err(e) => self.problem(e.to_string()),
            }
        }
        out
    }

    async fn refresh_library(&self) -> CoreResult<()> {
        {
            let mut state = self.state();
            state.loading = true;
            state.detail_failed.clear();
        }
        self.changed();
        let kernel = &self.inner.kernel;
        let mut library = Vec::new();
        let mut badges = HashMap::new();
        for (plugin, provided) in self.providers(Capability::Catalog).await {
            let catalog = provided.catalog.expect("declared and checked at init");
            let page = match kernel.guard(&plugin, "catalog.list", catalog.list(PageRequest::ALL)).await {
                Ok(page) => page,
                Err(e) => {
                    self.problem(e.to_string());
                    continue;
                }
            };
            for summary in page.items {
                let key = GameKey::new(plugin.clone(), summary.id.clone());
                if let Some(slots) = &provided.slots {
                    let request = SlotRequest { slot: SlotId::GameBadge, game: Some(summary.id.clone()) };
                    match kernel.guard(&plugin, "slots.items", slots.items(&request)).await {
                        Ok(items) => {
                            badges.insert(key.clone(), slot_vms(&plugin, SlotId::GameBadge, items));
                        }
                        Err(e) => self.problem(e.to_string()),
                    }
                }
                library.push(Entry { key, summary });
            }
        }
        library.sort_by(|a, b| {
            a.summary.title.to_lowercase().cmp(&b.summary.title.to_lowercase()).then(a.key.cmp(&b.key))
        });
        // The home screen shows the selected game with its details. With no selection, or one
        // that is gone, the first game is selected, so the first screen is a whole one.
        let first = {
            let mut state = self.state();
            state.library = library;
            state.badges = badges;
            state.loading = false;
            let kept = state.selected.as_ref().is_some_and(|s| state.library.iter().any(|e| &e.key == s));
            if kept {
                None
            } else {
                state.selected = state.library.first().map(|e| e.key.clone());
                state.selected.clone()
            }
        };
        self.remember_offer_art();
        kernel.events().publish(Event::LibraryChanged);
        self.changed();
        if let Some(game) = first
            && !self.state().details.contains_key(&game)
        {
            if let Err(e) = self.load_detail(&game).await {
                self.state().detail_failed.insert(game);
                self.problem(e.to_string());
            }
            self.changed();
        }
        if self.state().booted {
            // Later loads: the store's art and the details, all in the background.
            let core = self.clone();
            tokio::spawn(async move {
                core.describe_library().await;
                core.read_all_details().await;
            });
            return Ok(());
        }
        // The first load of a run is the boot: the emulators' builds for the Welcome
        // checklist, and the store's art and details, as the old launcher's boot did, so Home
        // opens whole. A slow network doesn't hold it for more than BOOT_DESCRIBE_LIMIT.
        self.refresh_emulators().await;
        if tokio::time::timeout(BOOT_DESCRIBE_LIMIT, self.describe_library()).await.is_err() {
            tracing::info!("the store's descriptions are slow: Home opens without the rest");
            let core = self.clone();
            tokio::spawn(async move { core.describe_library().await });
        }
        self.state().booted = true;
        self.changed();
        let core = self.clone();
        tokio::spawn(async move {
            // The catalogs, read once a run, after Home is up.
            if core.state().catalogs.is_empty() {
                core.refresh_releases().await;
            }
            core.read_all_details().await;
        });
        Ok(())
    }

    /// Asks the metadata plugins about every game they haven't described yet.
    async fn describe_library(&self) {
        self.state().describing = Some((0, 0));
        let todo: Vec<(GameKey, nexus_api::GameRef)> = {
            let state = self.state();
            state
                .library
                .iter()
                .filter(|e| !state.meta.contains_key(&e.key))
                .filter_map(|e| {
                    let title_id = e.summary.title_id.clone()?;
                    Some((
                        e.key.clone(),
                        nexus_api::GameRef {
                            platform: e.summary.platform.clone(),
                            title_id,
                            content_id: e.summary.content_id.clone().unwrap_or_default(),
                            title: e.summary.title.clone(),
                            region: String::new(),
                        },
                    ))
                })
                .collect()
        };
        let described = self.describe_with(todo, true).await;
        let mut state = self.state();
        if !described {
            state.describing = None;
        }
        state.described = true;
        drop(state);
        self.changed();
    }

    /// Asks the metadata plugins about these games, and keeps what they say.
    pub(crate) async fn describe(&self, todo: Vec<(GameKey, nexus_api::GameRef)>) {
        self.describe_with(todo, false).await;
    }

    /// `false` when no plugin describes games. `boot` counts the answers for the boot screen.
    async fn describe_with(&self, todo: Vec<(GameKey, nexus_api::GameRef)>, boot: bool) -> bool {
        let providers: Vec<_> = self
            .providers(Capability::Metadata)
            .await
            .into_iter()
            .filter_map(|(plugin, provided)| provided.metadata.map(|m| (plugin, m)))
            .collect();
        if providers.is_empty() {
            return false;
        }
        // Six at a time, as the old launcher's fetch threads.
        let total = todo.len();
        let done = std::sync::atomic::AtomicUsize::new(0);
        let providers = &providers;
        let answers = futures::stream::iter(todo.into_iter().map(|(key, game)| async move {
            let mut failed = false;
            for (plugin, metadata) in providers {
                match self.inner.kernel.guard(plugin, "metadata.describe", metadata.describe(&game)).await {
                    Ok(Some(found)) => return (key, Some((plugin.clone(), found)), false),
                    Ok(None) => {}
                    Err(e) => {
                        failed = true;
                        tracing::info!(plugin_id = %plugin, error = %e, "could not describe a game");
                    }
                }
            }
            (key, None, failed)
        }));
        let mut answers = futures::StreamExt::buffer_unordered(answers, DESCRIBE_AT_ONCE);
        while let Some((key, found, failed)) = futures::StreamExt::next(&mut answers).await {
            let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let mut state = self.state();
            match found {
                Some(found) => {
                    state.meta.insert(key, found);
                }
                // The store didn't answer (offline): it is asked again when the game shows.
                None if failed => {
                    state.asked.remove(&key);
                }
                None => {}
            }
            if boot {
                state.describing = Some((n, total));
            }
            state.task = Some(format!("Downloading game art {n}/{total}"));
            drop(state);
            self.changed();
        }
        self.state().task = None;
        self.changed();
        true
    }

    /// Reads every game's details, for the Library's sizes.
    async fn read_all_details(&self) {
        let keys: Vec<GameKey> = {
            let state = self.state();
            state.library.iter().map(|e| e.key.clone()).filter(|k| !state.details.contains_key(k)).collect()
        };
        for (n, key) in keys.iter().enumerate() {
            self.state().task = Some(format!("Reading game details {}/{}", n + 1, keys.len()));
            self.changed();
            if self.load_detail(key).await.is_err() {
                self.state().detail_failed.insert(key.clone());
            }
        }
        self.state().task = None;
        self.changed();
    }

    /// Searches the Catalog, and shows it.
    async fn search(&self, text: String) -> CoreResult<()> {
        if self.state().catalogs.is_empty() {
            self.refresh_releases().await;
        }
        self.set_grid(true, |v| v.query = text)?;
        self.push(Route::Catalog);
        Ok(())
    }

    /// Changes the Library's or the Catalog's view, and goes back to its first page.
    fn set_grid(&self, catalog: bool, change: impl FnOnce(&mut LibraryView)) -> CoreResult<()> {
        {
            let mut state = self.state();
            let view = if catalog { &mut state.catalog_view } else { &mut state.library_view };
            change(view);
            view.shown = GRID_PAGE;
        }
        self.changed();
        if catalog {
            self.describe_catalog_page();
        }
        Ok(())
    }

    /// The next page of the Library or the Catalog.
    fn more(&self, catalog: bool) -> CoreResult<()> {
        {
            let mut state = self.state();
            let view = if catalog { &mut state.catalog_view } else { &mut state.library_view };
            view.shown += GRID_PAGE;
        }
        self.changed();
        if catalog {
            self.describe_catalog_page();
        }
        Ok(())
    }

    /// Selects a game: a game the catalogs offer in the Catalog, any other on Home.
    fn select(&self, game: &GameKey) {
        let mut state = self.state();
        if state.offers.iter().any(|o| &o.entry.key == game) {
            state.catalog_selected = Some(game.clone());
        } else {
            state.selected = Some(game.clone());
        }
    }

    async fn open_game(&self, game: GameKey) -> CoreResult<()> {
        // A game a catalog offers: its download sheet names the VPN in use.
        if self.vpn_only() && self.offer(&game).is_some() {
            self.refresh_vpns().await;
        }
        self.load_detail(&game).await?;
        self.select(&game);
        self.push(Route::GameDetail(game));
        Ok(())
    }

    /// Asks the game's plugin for its details, compatibility, shader cache and slots.
    async fn load_detail(&self, game: &GameKey) -> CoreResult<()> {
        if let Some(offer) = self.offer(game) {
            let detail = self.offer_detail(&offer);
            self.state().details.insert(game.clone(), detail);
            return Ok(());
        }
        let kernel = &self.inner.kernel;
        let plugin = game.plugin.clone();
        let provided = kernel.provided(&plugin).await?;
        let catalog = require(&plugin, Capability::Catalog, provided.catalog.clone())?;
        let details = kernel.guard(&plugin, "catalog.details", catalog.details(&game.local)).await?;
        let mut detail = Detail { details: Some(details), ..Detail::default() };
        if let Some(compat) = &provided.compatibility {
            match kernel.guard(&plugin, "compatibility.status", compat.status(&game.local)).await {
                Ok(info) => {
                    detail.compat_page = info.page.clone().unwrap_or_default();
                    detail.compat = Some(view::compat_vm(&info));
                }
                Err(e) => self.problem(e.to_string()),
            }
        }
        if let Some(shaders) = &provided.shaders {
            match kernel.guard(&plugin, "shaders.cache", shaders.cache(&game.local)).await {
                Ok(info) => detail.shaders = Some(info),
                Err(e) => self.problem(e.to_string()),
            }
        }
        if let Some(slots) = &provided.slots {
            for slot in [SlotId::GameAction, SlotId::GameDetailSection, SlotId::GameBadge] {
                let request = SlotRequest { slot, game: Some(game.local.clone()) };
                match kernel.guard(&plugin, "slots.items", slots.items(&request)).await {
                    Ok(items) => {
                        let vms = slot_vms(&plugin, slot, items);
                        match slot {
                            SlotId::GameAction => detail.actions = vms,
                            SlotId::GameDetailSection => detail.sections = vms,
                            _ => {
                                self.state().badges.insert(game.clone(), vms);
                            }
                        }
                    }
                    Err(e) => self.problem(e.to_string()),
                }
            }
        }
        detail.settings = self.game_settings(&plugin, &provided, game).await;
        self.refresh_emulator(&plugin).await;
        let mut state = self.state();
        state.details.insert(game.clone(), detail);
        state.detail_failed.remove(game);
        Ok(())
    }

    /// Installed and available versions, and panels, for one emulator.
    pub(crate) async fn refresh_emulator(&self, plugin: &PluginId) {
        let kernel = &self.inner.kernel;
        let Ok(provided) = kernel.provided(plugin).await else { return };
        let mut emu = self.state().emulators.get(plugin).cloned().unwrap_or_default();
        if let Some(installer) = &provided.installer {
            match kernel.guard(plugin, "installer.installed", installer.installed()).await {
                Ok(list) => {
                    emu.own = list.iter().any(|v| v.own);
                    emu.installed = list.into_iter().map(|v| v.version).collect();
                }
                Err(e) => self.problem(e.to_string()),
            }
            match kernel.guard(plugin, "installer.available", installer.available()).await {
                Ok(list) => emu.available = list.into_iter().map(|v| v.version).collect(),
                Err(e) => self.problem(e.to_string()),
            }
        }
        if let Some(controllers) = &provided.controllers {
            match kernel.guard(plugin, "controllers.profiles", controllers.profiles()).await {
                Ok(profiles) => emu.controllers = profiles,
                Err(e) => self.problem(e.to_string()),
            }
        }
        if let Some(slots) = &provided.slots {
            let request = SlotRequest { slot: SlotId::EmulatorPanel, game: None };
            if let Ok(items) = kernel.guard(plugin, "slots.items", slots.items(&request)).await {
                emu.panels = slot_vms(plugin, SlotId::EmulatorPanel, items);
            }
        }
        self.state().emulators.insert(plugin.clone(), emu);
    }

    async fn refresh_emulators(&self) {
        for plugin in self.inner.kernel.of_kind(PluginKind::Emulator) {
            self.refresh_emulator(&plugin).await;
        }
    }

    /// A game's own settings, from its emulator's schema for it. `None` when it has none.
    async fn game_settings(&self, plugin: &PluginId, provided: &Provided, game: &GameKey) -> Option<SettingsPanelVm> {
        let kernel = &self.inner.kernel;
        let configurator = provided.configurator.as_ref()?;
        let scope = ConfigScope::Game(game.local.clone());
        let schema = kernel.guard(plugin, "configurator.schema", configurator.schema(&scope)).await.ok()?;
        if schema["properties"].as_object().is_none_or(|p| p.is_empty()) {
            return None;
        }
        let values = self.config_value(plugin, &scope).await.unwrap_or(Value::Null);
        let name = kernel.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_default();
        let slot = SlotVm {
            slot: SlotId::SettingsPanel,
            plugin: plugin.clone(),
            id: "game".into(),
            label: format!("{plugin}:game"),
            kind: nexus_ui_api::SlotKind::SettingsPanel,
            text: format!("{name} settings for this game"),
            short: String::new(),
            tone: nexus_api::Tone::Neutral,
            style: nexus_api::ActionStyle::Secondary,
            icon: None,
            rows: Vec::new(),
            action: None,
            args: Value::Null,
            scope: Some(scope),
        };
        Some(SettingsPanelVm { fields: view::fields(&schema, &values), slot })
    }

    async fn refresh_settings(&self) {
        let kernel = &self.inner.kernel;
        let mut panels = Vec::new();
        for (plugin, provided) in self.providers(Capability::Slots).await {
            let slots = provided.slots.clone().expect("declared and checked at init");
            let request = SlotRequest { slot: SlotId::SettingsPanel, game: None };
            let Ok(items) = kernel.guard(&plugin, "slots.items", slots.items(&request)).await else { continue };
            for slot in slot_vms(&plugin, SlotId::SettingsPanel, items) {
                let scope = slot.scope.clone().unwrap_or(ConfigScope::Emulator);
                let schema = match &provided.configurator {
                    Some(c) => {
                        kernel.guard(&plugin, "configurator.schema", c.schema(&scope)).await.unwrap_or(Value::Null)
                    }
                    None => Value::Null,
                };
                let values = self.config_value(&plugin, &scope).await.unwrap_or(Value::Null);
                panels.push(SettingsPanelVm { fields: view::fields(&schema, &values), slot });
            }
        }
        self.state().settings = panels;
    }

    /// Plans a download and queues it with the download manager. It stays saved until it
    /// ends, so a crash resumes it.
    async fn download(&self, plugin: &PluginId, request: DownloadRequest) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        let downloader = require(plugin, Capability::Downloader, provided.downloader)?;
        let plan = kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await?;
        let record = DownloadRecord { plugin: plugin.clone(), request };
        let key = record.key();
        let saved = serde_json::to_value(&record).expect("records serialize");
        if let Err(e) = kernel.config().set(&["app", "downloads", &key], saved) {
            tracing::warn!(error = %e, "could not save download state");
        }
        // The manager removes the record and reports the end as an event.
        let _ = self.inner.downloads.submit(plugin, plan, Some(key));
        self.changed();
        Ok(())
    }

    /// Re-runs downloads that were in progress when the app last stopped. Their part files
    /// are still there, so each continues where it stopped.
    fn resume_downloads(&self) {
        let pending = self.inner.kernel.config().get(&["app", "downloads"]);
        let Some(pending) = pending.as_object() else { return };
        for (key, saved) in pending.clone() {
            let Ok(record) = serde_json::from_value::<DownloadRecord>(saved) else {
                let _ = self.inner.kernel.config().remove(&["app", "downloads", &key]);
                continue;
            };
            tracing::info!(plugin_id = %record.plugin, request = ?record.request, "resuming an interrupted download");
            let core = self.clone();
            tokio::spawn(async move {
                // Saved again under its own key, then removed when it ends.
                let _ = core.inner.kernel.config().remove(&["app", "downloads", &key]);
                if let Err(e) = core.download(&record.plugin, record.request).await {
                    core.notice_opening(
                        NoticeKind::Error,
                        Route::Downloads,
                        format!("Could not resume a download for {}: {e}", record.plugin),
                    );
                }
            });
        }
    }

    fn set_install(&self, plugin: &PluginId, record: Option<InstallRecord>, phase: InstallPhase, version: &str) {
        let key = ["app", "installs", plugin.as_str()];
        let saved = match record {
            Some(record) => {
                self.inner.kernel.config().set(&key, serde_json::to_value(record).expect("records serialize"))
            }
            None => self.inner.kernel.config().remove(&key),
        };
        if let Err(e) = saved {
            tracing::warn!(error = %e, "could not save install state");
        }
        if let Some(emu) = self.state().emulators.get_mut(plugin) {
            emu.busy = matches!(phase, InstallPhase::Downloading | InstallPhase::Installing);
        }
        self.inner.kernel.events().publish(Event::InstallChanged {
            plugin: plugin.clone(),
            version: version.into(),
            state: phase,
        });
        self.changed();
    }

    /// The install workflow: plan, download, check, install. Each phase is saved, so a crash
    /// resumes it at the next start.
    async fn install(&self, plugin: &PluginId, version: Option<String>, from: Option<String>) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        let installer = require(plugin, Capability::Installer, provided.installer.clone())?;
        let downloader = require(plugin, Capability::Downloader, provided.downloader.clone())?;
        let version = match version {
            Some(v) => v,
            None => kernel
                .guard(plugin, "installer.available", installer.available())
                .await?
                .into_iter()
                .next()
                .map(|v| v.version)
                .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?,
        };
        self.state().emulators.entry(plugin.clone()).or_default();
        let record = |phase| InstallRecord { version: version.clone(), from: from.clone(), phase };
        self.set_install(plugin, Some(record(InstallPhase::Downloading)), InstallPhase::Downloading, &version);

        let result: CoreResult<()> = async {
            let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
            let plan = kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await?;
            let files = self.inner.downloads.run(plugin, plan).await?;
            self.set_install(plugin, Some(record(InstallPhase::Installing)), InstallPhase::Installing, &version);
            match &from {
                Some(from) => {
                    kernel.guard(plugin, "installer.update", installer.update(from, &version, &files)).await?
                }
                None => kernel.guard(plugin, "installer.install", installer.install(&version, &files)).await?,
            };
            Ok(())
        }
        .await;

        match &result {
            Ok(()) => self.set_install(plugin, None, InstallPhase::Installed, &version),
            Err(e) => {
                self.set_install(plugin, None, InstallPhase::Failed, &version);
                self.notice_opening(
                    NoticeKind::Error,
                    Route::Plugin(plugin.clone()),
                    format!("Could not install {plugin} {version}: {e}"),
                );
            }
        }
        self.refresh_emulator(plugin).await;
        self.changed();
        result
    }

    async fn update(&self, plugin: &PluginId) -> CoreResult<()> {
        self.refresh_emulator(plugin).await;
        let emu = self.state().emulators.get(plugin).cloned().unwrap_or_default();
        let from =
            emu.installed.last().cloned().ok_or_else(|| CoreError::Invalid(format!("{plugin} is not installed")))?;
        let to = view::update_for(&emu).ok_or_else(|| CoreError::Invalid(format!("{plugin} is up to date")))?;
        self.install(plugin, Some(to), Some(from)).await
    }

    async fn uninstall_game(&self, game: &GameKey) -> CoreResult<()> {
        let also;
        let (folder, title) = {
            let state = self.state();
            if state.sessions.values().any(|s| &s.game == game) {
                return Err(CoreError::Invalid("Stop the game before you uninstall it".into()));
            }
            if state
                .game_installs
                .values()
                .any(|g| g.plugin == game.plugin && g.phase == crate::game_installs::GamePhase::Installing)
            {
                return Err(CoreError::Invalid("Wait until the install ends".into()));
            }
            let details = state.details.get(game).and_then(|d| d.details.as_ref());
            let folder = details.and_then(|d| d.folder.clone());
            also = details.map(|d| d.also.clone()).unwrap_or_default();
            let title = state.library.iter().find(|e| &e.key == game).map(|e| e.summary.title.clone());
            (folder, title.unwrap_or_else(|| game.local.clone()))
        };
        let folder = folder.ok_or_else(|| CoreError::Invalid(format!("{title} has no folder to delete")))?;
        let folders = game_folders(&self.inner.kernel, game)
            .ok_or_else(|| CoreError::Invalid(format!("{title} has no folder to delete")))?;
        folders.remove(&folder).await.map_err(|e| CoreError::Plugin { plugin: game.plugin.clone(), error: e })?;
        // Its update and add-ons, in their own folders.
        for other in &also {
            if let Err(e) = folders.remove(other).await {
                tracing::warn!(error = %e, folder = %other.path, "a folder of an uninstalled game stays");
            }
        }
        {
            let mut state = self.state();
            state.details.remove(game);
            state.nav.retain(|r| r != &Route::GameDetail(game.clone()));
            if state.nav.is_empty() {
                state.nav.push(Route::Home);
            }
        }
        self.refresh_library().await?;
        self.notice(format!("Uninstalled {title}"));
        self.changed();
        Ok(())
    }

    async fn uninstall(&self, plugin: &PluginId, version: &str) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        let installer = require(plugin, Capability::Installer, provided.installer)?;
        kernel.guard(plugin, "installer.uninstall", installer.uninstall(version)).await?;
        self.set_install(plugin, None, InstallPhase::Uninstalled, version);
        self.refresh_emulator(plugin).await;
        Ok(())
    }

    /// Re-runs installs that were in progress when the app last stopped.
    async fn resume_installs(&self) {
        let pending = self.inner.kernel.config().get(&["app", "installs"]);
        let Some(pending) = pending.as_object() else { return };
        for (plugin, saved) in pending.clone() {
            let (Ok(plugin), Ok(record)) = (PluginId::new(plugin), serde_json::from_value::<InstallRecord>(saved))
            else {
                continue;
            };
            tracing::info!(plugin_id = %plugin, version = %record.version, "resuming an interrupted install");
            let core = self.clone();
            tokio::spawn(async move {
                let _ = core.install(&plugin, Some(record.version), record.from).await;
            });
        }
    }

    async fn clear_shader_cache(&self, game: GameKey) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(&game.plugin).await?;
        let shaders = require(&game.plugin, Capability::Shaders, provided.shaders)?;
        kernel.guard(&game.plugin, "shaders.clear_cache", shaders.clear_cache(&game.local)).await?;
        let info = kernel.guard(&game.plugin, "shaders.cache", shaders.cache(&game.local)).await?;
        if let Some(detail) = self.state().details.get_mut(&game) {
            detail.shaders = Some(info);
        }
        self.changed();
        Ok(())
    }

    async fn set_config(&self, plugin: &PluginId, scope: ConfigScope, value: Value) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        if let Some(configurator) = &provided.configurator {
            kernel.guard(plugin, "configurator.validate", configurator.validate(&scope, &value)).await?;
        }
        kernel
            .config()
            .set(&["plugins", plugin.as_str(), &crate::ConfigService::scope_key(&scope)], value)
            .map_err(|e| CoreError::Io(e.to_string()))?;
        if !self.state().settings.is_empty() {
            self.refresh_settings().await;
        }
        // A new catalog address: the catalogs are read again.
        if provided.releases.is_some() {
            self.refresh_releases().await;
        }
        // A game's own settings: its panel shows the new value.
        if let ConfigScope::Game(local) = &scope {
            let game = GameKey::new(plugin.clone(), local.clone());
            let provided = kernel.provided(plugin).await?;
            let panel = self.game_settings(plugin, &provided, &game).await;
            if let Some(detail) = self.state().details.get_mut(&game) {
                detail.settings = panel;
            }
        }
        self.changed();
        Ok(())
    }

    async fn run_action(&self, plugin: &PluginId, action: &str, game: Option<GameKey>, args: Value) -> CoreResult<()> {
        // `host:` actions are the host's: a plugin's slot asks the host to do something.
        if action == nexus_api::HOST_OPEN_SETTINGS {
            return self.navigate(Route::Settings).await;
        }
        if action == HOST_INSTALL_GAME {
            let source = serde_json::from_value(args)
                .map_err(|e| CoreError::Invalid(format!("{HOST_INSTALL_GAME} needs a folder path: {e}")))?;
            return self.install_game(plugin, source, None).await;
        }
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        let slots = require(plugin, Capability::Slots, provided.slots)?;
        let local = game.as_ref().filter(|g| &g.plugin == plugin).map(|g| g.local.clone());
        kernel.guard(plugin, "slots.run_action", slots.run_action(action, local.as_deref(), &args)).await?;
        // The action may have changed what the plugin offers and shows.
        if provided.releases.is_some() {
            self.refresh_releases().await;
        }
        if self.state().nav.last() == Some(&Route::Plugin(plugin.clone())) {
            self.refresh_emulator(plugin).await;
        }
        if let Some(game) = game.filter(|g| &g.plugin == plugin)
            && self.state().nav.last() == Some(&Route::GameDetail(game.clone()))
        {
            self.open_game(game).await?;
        }
        self.changed();
        Ok(())
    }

    async fn set_config_field(
        &self,
        plugin: &PluginId,
        scope: ConfigScope,
        key: String,
        text: String,
    ) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let provided = kernel.provided(plugin).await?;
        let schema = match &provided.configurator {
            Some(c) => kernel.guard(plugin, "configurator.schema", c.schema(&scope)).await?,
            None => Value::Null,
        };
        let prop = schema.pointer(&format!("/properties/{key}")).cloned().unwrap_or(Value::Null);
        // An optional setting left empty is unset: the emulator's own value applies.
        let value = match view::type_of(&prop) {
            _ if text.trim().is_empty() && view::nullable(&prop) => Value::Null,
            Some("boolean") => Value::Bool(text == "true"),
            Some("integer") => text
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| CoreError::Invalid(format!("{key} must be a whole number")))?,
            Some("number") => text
                .trim()
                .parse::<f64>()
                .map(Value::from)
                .map_err(|_| CoreError::Invalid(format!("{key} must be a number")))?,
            _ => Value::String(text),
        };
        let mut current = self.config_value(plugin, &scope).await?;
        if !current.is_object() {
            current = Value::Object(serde_json::Map::new());
        }
        current.as_object_mut().expect("an object").insert(key, value);
        self.set_config(plugin, scope, current).await
    }

    async fn set_plugin_enabled(&self, plugin: &PluginId, enabled: bool) -> CoreResult<()> {
        let kernel = &self.inner.kernel;
        let manifest = kernel.manifest(plugin)?;
        if manifest.kind == PluginKind::Theme {
            return Err(CoreError::Invalid(format!("{} is a theme: switch themes instead", manifest.name)));
        }
        if !enabled && self.state().emulators.get(plugin).is_some_and(|e| e.busy) {
            return Err(CoreError::Invalid(format!("{} is busy: turn it off when it is done", manifest.name)));
        }
        // The list of exceptions: plugins turned off, or for one made for other systems, on.
        let key = if kernel.elsewhere(plugin) { "enabled_plugins" } else { "disabled_plugins" };
        let mut listed: Vec<String> = kernel
            .config()
            .get(&["app", key])
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        listed.retain(|id| id != plugin.as_str());
        if enabled == kernel.elsewhere(plugin) {
            listed.push(plugin.to_string());
        }
        kernel.config().set(&["app", key], serde_json::json!(listed)).map_err(|e| CoreError::Io(e.to_string()))?;
        // The last catalog turned off: the Catalog goes, and Back passes where it was. Its
        // offers go too, so turning one on reads them again.
        if !Self::catalog_available(kernel) {
            let mut state = self.state();
            state.nav.retain(|r| *r != Route::Catalog);
            state.nav.dedup();
            if state.nav.is_empty() {
                state.nav.push(Route::Home);
            }
            state.catalogs.clear();
            state.offers.clear();
            state.catalog_grid = None;
        }
        if enabled && kernel.of_kind(PluginKind::Emulator).contains(plugin) {
            self.refresh_emulator(plugin).await;
        }
        // An emulator on or off: its console's catalog comes or goes, and with it the catalog's
        // settings for that console.
        if manifest.kind == PluginKind::Emulator && Self::catalog_available(kernel) {
            self.refresh_releases().await;
        }
        self.refresh_settings().await;
        self.refresh_library().await?;
        self.changed();
        Ok(())
    }

    fn set_folders(&self, name: &str, paths: &[std::path::PathBuf]) -> CoreResult<()> {
        if !nexus_api::PathGrant::valid_name(name) {
            return Err(CoreError::Invalid(format!("{name:?} is not a folder name")));
        }
        // `~` and `~/…` are the player's home folder, as a shell reads them.
        let home = self.inner.kernel.host().home.clone();
        let paths: Vec<std::path::PathBuf> = paths
            .iter()
            .map(|p| match (p.strip_prefix("~"), &home) {
                (Ok(rest), Some(home)) if rest.as_os_str().is_empty() => home.clone(),
                (Ok(rest), Some(home)) => home.join(rest),
                _ => p.clone(),
            })
            .collect();
        if let Some(relative) = paths.iter().find(|p| !p.is_absolute()) {
            return Err(CoreError::Invalid(format!("{} is not a full path", relative.display())));
        }
        let config = self.inner.kernel.config();
        let saved = if paths.is_empty() {
            config.remove(&["app", "folders", name])
        } else {
            let list: Vec<Value> = paths.iter().map(|p| Value::from(p.to_string_lossy().into_owned())).collect();
            config.set(&["app", "folders", name], Value::Array(list))
        };
        saved.map_err(|e| CoreError::Io(e.to_string()))?;
        self.changed();
        Ok(())
    }

    /// The size of everything the theme draws: 1 unless the player changed it.
    pub fn scale(&self) -> f32 {
        let saved = self.inner.kernel.config().get(&["app", "scale"]).as_f64().unwrap_or(1.0) as f32;
        if saved.is_finite() { saved.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 }
    }

    /// The launcher starts when the player logs in. Off until the player turns it on.
    pub fn start_at_login_on(&self) -> bool {
        self.inner.kernel.config().get(&["app", "start_at_login"]).as_bool().unwrap_or(false)
    }

    /// Emulator updates install by themselves. On unless the player turned it off.
    pub fn auto_update_on(&self) -> bool {
        self.inner.kernel.config().get(&["app", "auto_update"]).as_bool().unwrap_or(true)
    }

    /// Runs [`Command::CheckUpdates`] every few hours while the app runs.
    fn schedule_update_checks(&self) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_UPDATE_CHECK).await;
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let core = Core { inner };
                if core.auto_update_on() {
                    core.check_updates().await;
                }
                drop(core);
                tokio::time::sleep(UPDATE_CHECK_EVERY).await;
            }
        });
    }

    /// Looks for updates of every installed emulator, and installs them when automatic
    /// updates are on. One whose game is running waits for it to close.
    async fn check_updates(&self) {
        for plugin in self.inner.kernel.of_kind(PluginKind::Emulator) {
            let installs =
                self.inner.kernel.manifest(&plugin).is_ok_and(|m| m.capabilities.contains(&Capability::Installer));
            if !installs {
                continue;
            }
            self.refresh_emulator(&plugin).await;
            let emu = self.state().emulators.get(&plugin).cloned().unwrap_or_default();
            if emu.installed.is_empty() || emu.own || view::update_for(&emu).is_none() || !self.auto_update_on() {
                continue;
            }
            if self.plays(&plugin) {
                tracing::info!(plugin_id = %plugin, "an update waits for the game to close");
                self.state().pending_updates.insert(plugin);
                continue;
            }
            if let Err(e) = self.update(&plugin).await {
                self.problem(e.to_string());
            }
        }
        self.changed();
    }

    /// One of the emulator's games is running.
    fn plays(&self, plugin: &PluginId) -> bool {
        self.state().sessions.values().any(|s| &s.game.plugin == plugin)
    }

    /// A game closed: an update that waited for it installs now.
    pub(crate) fn game_closed(&self, plugin: &PluginId) {
        if self.plays(plugin) || !self.state().pending_updates.remove(plugin) {
            return;
        }
        let (core, plugin) = (self.clone(), plugin.clone());
        tokio::spawn(async move {
            if let Err(e) = core.update(&plugin).await {
                core.problem(e.to_string());
            }
            core.changed();
        });
    }

    /// The game's emulator isn't installed: asks the player first. Returns whether it asked.
    pub(crate) async fn ask_to_install(&self, game: &GameKey, provided: &Provided) -> CoreResult<bool> {
        let kernel = &self.inner.kernel;
        let (Some(installer), Some(downloader)) = (&provided.installer, &provided.downloader) else { return Ok(false) };
        let plugin = &game.plugin;
        if !kernel.guard(plugin, "installer.installed", installer.installed()).await?.is_empty() {
            return Ok(false);
        }
        let version = kernel
            .guard(plugin, "installer.available", installer.available())
            .await?
            .into_iter()
            .next()
            .map(|v| v.version)
            .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?;
        let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
        let size_bytes = match kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await {
            Ok(plan) => plan.artifacts.iter().map(|a| a.size).sum::<Option<u64>>(),
            Err(_) => None,
        };
        let emulator =
            kernel.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_else(|_| plugin.to_string());
        let title = {
            let state = self.state();
            state
                .library
                .iter()
                .find(|e| &e.key == game)
                .map(|e| e.summary.title.clone())
                .unwrap_or_else(|| game.local.clone())
        };
        self.state().install_prompt = Some(InstallPrompt { game: game.clone(), emulator, version, title, size_bytes });
        self.changed();
        Ok(true)
    }

    /// Whether the launcher fills the screen. On until the player turns it off, as in the old
    /// launcher.
    /// The monitor the player chose for the launcher, by name.
    pub fn monitor(&self) -> Option<String> {
        self.inner.kernel.config().get(&["app", "monitor"]).as_str().map(str::to_owned)
    }

    pub fn fullscreen_on(&self) -> bool {
        self.inner.kernel.config().get(&["app", "fullscreen"]).as_bool().unwrap_or(true)
    }

    /// The game's folder on disk, through its plugin's folder grants. `None` before its
    /// details are in, or when it has no folder.
    pub fn game_folder(&self, game: &GameKey) -> Option<std::path::PathBuf> {
        let folder = self.state().details.get(game)?.details.as_ref()?.folder.clone()?;
        resolve_game_folder(&self.inner.kernel, game, &folder)
    }

    /// The folder of the download `id`, for its Open folder: once it installed, the installed
    /// game's folder. `None` before the folder is on disk.
    pub fn download_folder(&self, id: &str) -> Option<std::path::PathBuf> {
        let (job, game) = {
            let state = self.state();
            let job = state.torrents.get(id)?.clone();
            let game = state
                .game_installs
                .values()
                .rev()
                .find(|g| g.plugin == job.emulator && g.source == job.source)
                .and_then(|g| g.installed.clone());
            (job, game)
        };
        let kernel = &self.inner.kernel;
        let installed = game.and_then(|g| {
            let manifest = kernel.manifest(&job.emulator).ok()?;
            let host = kernel.host();
            crate::folders::resolve(
                kernel.config(),
                host.home.as_deref(),
                host.platform,
                &manifest.permissions,
                &g.dest,
            )
            .ok()
        });
        installed.filter(|p| p.is_dir()).or(Some(job.folder).filter(|p| p.is_dir()))
    }

    /// The emulator's log of the game's last run. `None` before it ran.
    pub fn game_log(&self, game: &GameKey) -> Option<std::path::PathBuf> {
        Some(crate::launch::log_path(&self.inner.kernel, game)).filter(|p| p.is_file())
    }

    /// The player's own result for a game: `ingame`, `menus`, `boots` or `nothing`.
    pub(crate) fn your_result(&self, game: &GameKey) -> Option<String> {
        self.inner.kernel.config().get(&["app", "results", &game.to_string()]).as_str().map(str::to_owned)
    }

    /// The emulator build `plugin` runs games on: its newest installed one.
    pub(crate) fn build_of(&self, plugin: &PluginId) -> Option<String> {
        self.state().emulators.get(plugin).and_then(|e| e.installed.last().cloned())
    }

    /// Whether the player answered "How far did it get?" for `game` on `build`, or said
    /// "Not now" to it. A game with no build is always asked.
    pub(crate) fn result_asked(&self, game: &GameKey, build: Option<&str>) -> bool {
        let Some(build) = build else { return false };
        let config = self.inner.kernel.config();
        ["result_builds", "result_skipped"]
            .iter()
            .any(|key| config.get(&["app", key, &game.to_string()]).as_str() == Some(build))
    }

    /// The first library load of the run, with the store's art and details, is done.
    pub fn booted(&self) -> bool {
        self.state().booted
    }

    /// Whether the player has been welcomed: the Welcome screen shows on the first run only.
    pub fn welcomed(&self) -> bool {
        self.inner.kernel.config().get(&["app", "welcomed"]).as_bool().unwrap_or(false)
    }

    /// Whether the launcher comes back to the front when a game closes. On until the player
    /// turns it off, as in the old launcher.
    pub fn return_on_exit_on(&self) -> bool {
        self.inner.kernel.config().get(&["app", "return_on_exit"]).as_bool().unwrap_or(true)
    }

    /// A game is running.
    pub fn game_running(&self) -> bool {
        !self.state().sessions.is_empty()
    }

    /// The running games' sessions, to stop them.
    pub fn running_sessions(&self) -> Vec<SessionId> {
        self.state().sessions.keys().copied().collect()
    }

    /// A game has the screen: the gamepad is the game's, not the launcher's. The desktop says
    /// which window is in front; when it can't, the last window the launcher showed counts.
    /// Cheap: it reads app state, so the UI can ask on every press.
    pub fn game_in_front(&self) -> bool {
        crate::launch::game_in_front(self)
    }

    /// Whether theme sounds play. On until the player turns them off.
    pub fn sounds_on(&self) -> bool {
        self.inner.kernel.config().get(&["app", "sounds"]).as_bool().unwrap_or(true)
    }

    /// The player's option values for a theme.
    pub fn theme_options(&self, theme: &PluginId) -> Value {
        self.inner.kernel.config().get(&["app", "theme-options", theme.as_str()])
    }

    fn set_theme_option(&self, key: String, value: String) -> CoreResult<()> {
        let theme = self.state().theme.clone().ok_or_else(|| CoreError::Invalid("no theme is active".into()))?;
        let mut options = self.theme_options(&theme);
        if !options.is_object() {
            options = Value::Object(serde_json::Map::new());
        }
        options.as_object_mut().expect("an object").insert(key, Value::String(value));
        self.inner
            .kernel
            .config()
            .set(&["app", "theme-options", theme.as_str()], options)
            .map_err(|e| CoreError::Io(e.to_string()))?;
        self.changed();
        Ok(())
    }

    /// Records the theme the UI shows, without saving it as the player's choice. The UI host
    /// calls this at start, including when it falls back to the default theme.
    pub fn set_active_theme(&self, theme: PluginId) {
        self.state().theme = Some(theme);
    }

    /// The bytes of an image a view model refers to. URLs are fetched within the plugin's
    /// network allowlist.
    pub async fn image_bytes(&self, image: &nexus_ui_api::ImageRef) -> CoreResult<Vec<u8>> {
        let kernel = &self.inner.kernel;
        match image {
            nexus_ui_api::ImageRef::PluginAsset { plugin, path } => {
                let provided = kernel.provided(plugin).await?;
                let from_plugin = provided.assets.as_ref().and_then(|files| files.read(path));
                let from_dir = || {
                    let folder = kernel.manifest(plugin).ok()?.assets.clone()?;
                    let dir = kernel.plugin_dir(plugin).ok()??;
                    nexus_api::Files::read(&crate::DirFiles::new(folder.under(dir)), path)
                };
                from_plugin.or_else(from_dir).ok_or_else(|| CoreError::Io(format!("{plugin} has no asset {path}")))
            }
            nexus_ui_api::ImageRef::Folder { plugin, path } => {
                let folders = crate::folders::HostFolders {
                    plugin: plugin.clone(),
                    permissions: kernel.manifest(plugin)?.permissions.clone(),
                    config: kernel.config().clone(),
                    home: kernel.host().home.clone(),
                    platform: kernel.host().platform,
                };
                nexus_api::Folders::read_range(&folders, path, 0, nexus_api::MAX_FOLDER_READ)
                    .await
                    .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })
            }
            nexus_ui_api::ImageRef::Url { plugin, url } => {
                // Kept as it came, as the old launcher's image cache: a warm start doesn't
                // touch the network.
                use sha2::Digest;
                let cached = kernel.host().data_dir.join("cache/net").join(hex::encode(sha2::Sha256::digest(url)));
                if let Ok(bytes) = tokio::fs::read(&cached).await {
                    return Ok(bytes);
                }
                let permissions = &kernel.manifest(plugin)?.permissions;
                let request = nexus_api::HttpRequest::get(url.clone());
                let response = crate::facades::send_checked(kernel.http().as_ref(), plugin, permissions, request)
                    .await
                    .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })?;
                if response.status == 200 {
                    if let Some(dir) = cached.parent() {
                        let _ = tokio::fs::create_dir_all(dir).await;
                    }
                    if let Err(e) = tokio::fs::write(&cached, &response.body).await {
                        tracing::debug!(error = %e, "could not keep an image");
                    }
                    Ok(response.body)
                } else {
                    Err(CoreError::Io(format!("HTTP {} for {url}", response.status)))
                }
            }
        }
    }

    /// Saves `theme` as the player's choice and announces [`Event::ThemeChanged`]. The UI host
    /// calls it once the new theme is on screen.
    pub fn theme_switched(&self, theme: PluginId) -> CoreResult<()> {
        let from = self.state().theme.replace(theme.clone());
        self.inner
            .kernel
            .config()
            .set(&["app", "theme"], Value::from(theme.as_str()))
            .map_err(|e| CoreError::Io(e.to_string()))?;
        self.inner.kernel.events().publish(Event::ThemeChanged { from, to: theme });
        self.changed();
        Ok(())
    }

    pub(crate) fn next_session(&self) -> SessionId {
        SessionId(self.inner.next_session.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

/// The sorts the Library and the Catalog know.
fn check_sort(sort: &str) -> CoreResult<()> {
    match sort {
        "title" | "played" | "size" | "smallest" | "rating" | "release" | "compat" | "added" => Ok(()),
        _ => Err(CoreError::Invalid(format!("no sort {sort:?}"))),
    }
}

impl LibraryView {
    /// A system, or `INGAME_FILTER`, `INSTALLED_FILTER` or `HIDDEN_FILTER` to turn "In-game",
    /// "Installed" or "Hidden" on or off; `None` clears them all.
    fn set_system(&mut self, system: Option<String>) {
        match system.as_deref() {
            Some(INGAME_FILTER) => self.ingame = !self.ingame,
            Some(INSTALLED_FILTER) => self.installed = !self.installed,
            Some(HIDDEN_FILTER) => self.hidden = !self.hidden,
            None => {
                self.system = None;
                self.ingame = false;
                self.installed = false;
                self.hidden = false;
            }
            Some(_) => self.system = system,
        }
    }
}
