//! The public app API: commands in, view models and events out.
//!
//! All state lives here, never in a theme, so the UI can be rebuilt (or a theme swapped) at
//! any time without losing navigation, selection, downloads or running games.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::stream::BoxStream;
use nexus_api::{
    Capability, ConfigScope, DownloadRequest, Event, GameKey, InstallPhase, JobId, PluginId, PluginKind, Provided,
    SessionId, SlotId, SlotRequest,
};
use nexus_ui_api::{AppView, NoticeKind};
use serde_json::Value;

use crate::changes::{Area, Change, Changes, Notifier};
use crate::downloads::Downloads;
use crate::error::require;
use crate::library::DetailState;
use crate::preferences::{Pref, Preferences, PreferencesSnapshot};
use crate::view;
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
    /// The Library: list its games again, or hide a game from the Catalog.
    Library(crate::LibraryCmd),
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
    /// The running games: launch one, stop it, and switch between it and the launcher.
    Session(crate::SessionCmd),
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
    /// Sets one of the player's preferences: sounds, full screen, size, the monitor and so
    /// on. [`Pref`] says what values each takes; a size outside its range goes to its nearest
    /// end, and a value of the wrong type is refused.
    Pref(Pref, Value),
    /// The UI host finished swapping to this theme.
    ThemeSwitched(PluginId),
    /// The notices: close one or all, or open what one is about.
    Notice(crate::NoticeCmd),
    /// Looks for emulator updates and, if automatic updates are on, installs them. An update
    /// for an emulator whose game is running waits until the game closes. The core runs this
    /// every few hours by itself.
    CheckUpdates,
    /// The launcher's own update: look for one, or download it.
    LauncherUpdate(crate::LauncherUpdateCmd),
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
    /// The catalogs' downloads: download a release, act on a download, install it, and
    /// "Keep sharing finished downloads".
    Torrent(crate::TorrentCmd),
    /// Game installs: install a finished download or a package, delete a download's unpacked
    /// files, stop a download's install.
    GameInstall(crate::GameInstallCmd),
    /// "Download only through a VPN", and the VPNs: connect, disconnect and add them.
    Vpn(crate::VpnCmd),
    /// Something the player asked of the machine: Wi-Fi, Bluetooth, sound, power, Nexus OS.
    System(crate::system::SystemCommand),
    /// The player agreed to install the emulator a game needs: install it, then play.
    ConfirmInstall,
    /// The player said "not now" to that.
    CancelInstall,
}

pub use nexus_api::HOST_INSTALL_GAME;

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
pub(crate) struct EmulatorState {
    pub installed: Vec<String>,
    /// The installed build is the player's own: it never updates by itself.
    pub own: bool,
    pub available: Vec<String>,
    pub busy: bool,
    /// Its `emulator.panel` slot items.
    pub panels: Vec<nexus_api::SlotItem>,
}

/// A plugin's settings panel on the Settings screen: its slot item, and the schema and the
/// values of the settings scope it names.
#[derive(Clone, Debug)]
pub(crate) struct SettingsPanel {
    pub plugin: PluginId,
    pub item: nexus_api::SlotItem,
    pub schema: Value,
    pub values: Value,
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

/// How long a selection must stay before its details load.
const SELECT_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

pub(crate) struct AppState {
    pub nav: Vec<Route>,
    pub selected: Option<GameKey>,
    /// Each emulator's builds and blocks, and a service plugin's blocks once its page opens.
    pub emulators: BTreeMap<PluginId, EmulatorState>,
    pub settings: Vec<SettingsPanel>,
    pub theme: Option<PluginId>,
    /// The Library's search, filters and sort.
    pub library_view: LibraryView,
    /// The Catalog's.
    pub catalog_view: LibraryView,
    /// A game that just closed, to ask the player how far it got, and the emulator build it
    /// ran on.
    pub result_prompt: Option<(GameKey, Option<String>)>,
    /// This PC, once the check ran.
    pub hardware: Option<crate::Specs>,
    /// The machine's settings, from the system plugins.
    pub system: crate::system::SystemState,
    pub install_prompt: Option<InstallPrompt>,
    /// Emulators with an update that waits for their game to close.
    pub pending_updates: BTreeSet<PluginId>,
    /// The installs stores are doing (Steam's), by the catalog plugin, as last read.
    pub store_installs: Vec<(PluginId, nexus_api::StoreInstall)>,
    /// Until when the stores' installs are read often: a minute after Nexus asked a store to
    /// install, as the store writes its install down a moment later.
    pub store_watch_until: Option<std::time::Instant>,
    /// The game selected in the Catalog. Home's selection stays a library game.
    pub catalog_selected: Option<GameKey>,
    /// The Catalog's grid as last built: on another screen, each view shows this one instead
    /// of searching and sorting every offer again.
    pub catalog_grid: Option<nexus_ui_api::LibraryVm>,
    /// Each grid's columns.
    pub grid_columns: HashMap<Grid, usize>,
}

pub(crate) struct Inner {
    pub kernel: Kernel,
    pub state: Mutex<AppState>,
    pub downloads: Downloads,
    pub changes: Changes,
    /// The catalogs' downloads.
    pub torrents: crate::torrents::TorrentsSubsystem,
    /// The app's own settings.
    pub preferences: Preferences,
    pub installs: crate::game_installs::GameInstallsSubsystem,
    /// The games and the catalogs' offers.
    pub library: crate::library::LibrarySubsystem,
    /// The running games.
    pub sessions: crate::sessions::SessionsSubsystem,
    pub notices: crate::notices::NoticesSubsystem,
    pub launcher_update: crate::launcher_update::LauncherUpdateSubsystem,
    pub vpn: crate::vpn::VpnSubsystem,
    /// Held while the catalogs are read: see [`Core::refresh_releases`].
    pub reading_catalogs: tokio::sync::Mutex<()>,
}

/// The app. Cheap to clone; every clone is the same app.
#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
}

impl Core {
    /// Starts the app on a booted kernel and resumes installs a crash interrupted.
    pub async fn start(kernel: Kernel) -> Core {
        let theme = Pref::Theme.load(kernel.config()).as_str().and_then(|s| PluginId::new(s).ok());
        let state = AppState {
            nav: vec![Route::Home],
            selected: None,
            emulators: BTreeMap::new(),
            settings: Vec::new(),
            theme,
            library_view: LibraryView::default(),
            catalog_view: LibraryView::catalog(),
            result_prompt: None,
            hardware: None,
            system: Default::default(),
            install_prompt: None,
            pending_updates: BTreeSet::new(),
            store_installs: Vec::new(),
            store_watch_until: None,
            catalog_selected: None,
            catalog_grid: None,
            grid_columns: HashMap::new(),
        };
        let downloads = Downloads::spawn(kernel.clone());
        let changes = Changes::new();
        let library = crate::library::LibrarySubsystem::new(
            crate::library::Records::new(kernel.config().clone()),
            Notifier::new(changes.clone(), kernel.events().clone()),
        );
        let inner = Arc::new_cyclic(|weak: &std::sync::Weak<Inner>| {
            let weak_sessions = weak.clone();
            let weak = weak.clone();
            // The subsystem's events reach the core while it lives.
            let events: crate::torrents::EventSink = {
                let weak = weak.clone();
                Arc::new(move |event| {
                    if let Some(inner) = weak.upgrade() {
                        Core { inner }.torrent_event(event);
                    }
                })
            };
            let torrents = crate::torrents::TorrentsSubsystem::new(
                kernel.torrent_ports(),
                crate::torrents::Polls { torrents: kernel.host().torrent_poll, vpn: kernel.host().vpn_poll },
                crate::torrents::Records::new(kernel.config().clone()),
                kernel.vpn_rule().clone(),
                Notifier::new(changes.clone(), kernel.events().clone()),
                events,
            );
            let preferences = Preferences::new(
                kernel.config().clone(),
                kernel.pref_ports(),
                Notifier::new(changes.clone(), kernel.events().clone()),
            );
            let install_events: crate::game_installs::EventSink = Arc::new(move |event| {
                let core = weak.upgrade().map(|inner| Core { inner });
                Box::pin(async move {
                    if let Some(core) = core {
                        core.install_event(event).await;
                    }
                })
            });
            let installs = crate::game_installs::GameInstallsSubsystem::new(
                crate::game_installs::InstallPorts {
                    archives: kernel.archives().clone(),
                    hardware: kernel.hardware().clone(),
                    clock: kernel.clock().clone(),
                    downloads: Arc::new(torrents.clone()),
                    call_timeout: kernel.host().call_timeout,
                },
                crate::game_installs::Records::new(kernel.config().clone()),
                Notifier::new(changes.clone(), kernel.events().clone()),
                install_events,
            );
            let events: crate::sessions::EventSink = Arc::new(move |event| {
                if let Some(inner) = weak_sessions.upgrade() {
                    Core { inner }.session_event(event);
                }
            });
            let sessions = crate::sessions::SessionsSubsystem::new(
                kernel.session_ports(),
                crate::sessions::Timings {
                    watch_every: kernel.host().watch_every,
                    show_timeout: kernel.host().show_timeout,
                    hand_over_timeout: kernel.host().hand_over_timeout,
                    stop_grace: kernel.host().stop_grace,
                },
                kernel.config().clone(),
                kernel.events().clone(),
                Notifier::new(changes.clone(), kernel.events().clone()),
                events,
            );
            let notices =
                crate::notices::NoticesSubsystem::new(Notifier::new(changes.clone(), kernel.events().clone()));
            let launcher_update = crate::launcher_update::LauncherUpdateSubsystem::new(
                crate::launcher_update::LauncherUpdatePorts {
                    http: kernel.http().clone(),
                    login_session: kernel.login_session().clone(),
                },
                crate::launcher_update::ThisLauncher {
                    data_dir: kernel.host().data_dir.clone(),
                    install: kernel.host().install,
                },
                kernel.config().clone(),
                Notifier::new(changes.clone(), kernel.events().clone()),
            );
            // The setting or the VPNs changed: the downloads' kill switch looks at once.
            let kill_switch = torrents.clone();
            let vpn = crate::vpn::VpnSubsystem::new(
                kernel.vpn_rule().clone(),
                kernel.host().home.clone(),
                Notifier::new(changes.clone(), kernel.events().clone()),
                Arc::new(move || {
                    let torrents = kill_switch.clone();
                    Box::pin(async move { torrents.vpn_changed().await })
                }),
            );
            Inner {
                preferences,
                kernel,
                state: Mutex::new(state),
                downloads,
                changes,
                torrents,
                installs,
                library,
                sessions,
                notices,
                launcher_update,
                vpn,
                reading_catalogs: tokio::sync::Mutex::new(()),
            }
        });
        let core = Core { inner };
        core.installs().clean_up(|plugin| plugin_folders(core.kernel(), plugin));
        crate::system::adopt_vpn(&core).await;
        core.torrents().restore();
        core.resume_installs().await;
        core.resume_downloads();
        core.schedule_update_checks();
        crate::system::schedule_refresh(&core);
        core.check_hardware();
        core.watch_catalogs();
        core.watch_store_installs();
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
            core.changed(Area::Hardware);
        });
    }

    /// Once for each PC: a notice when an emulator's games won't start here, before the
    /// player downloads one. The config keeps the PC it was for.
    fn notice_blocked(&self, specs: &crate::Specs) {
        let kernel = &self.inner.kernel;
        let summary = crate::hardware::summary(specs);
        if self.preferences().get(Pref::PcNoticed).as_str() == Some(summary.as_str()) {
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
        if let Err(e) = self.preferences().save(Pref::PcNoticed, Value::String(summary)) {
            tracing::warn!(error = %e, "could not save the PC the notice was for");
        }
    }

    pub fn kernel(&self) -> &Kernel {
        &self.inner.kernel
    }

    pub fn events(&self) -> BoxStream<'static, Event> {
        self.inner.kernel.events().stream()
    }

    /// The catalogs' downloads.
    pub(crate) fn torrents(&self) -> &crate::torrents::TorrentsSubsystem {
        &self.inner.torrents
    }

    /// The app's own settings.
    pub(crate) fn preferences(&self) -> &Preferences {
        &self.inner.preferences
    }

    /// The app's own settings, read whole: sounds, full screen, size, the monitor and so on.
    pub fn prefs(&self) -> PreferencesSnapshot {
        self.preferences().snapshot()
    }

    pub(crate) fn installs(&self) -> &crate::game_installs::GameInstallsSubsystem {
        &self.inner.installs
    }

    /// The games and the catalogs' offers.
    pub(crate) fn library(&self) -> &crate::library::LibrarySubsystem {
        &self.inner.library
    }

    /// The running games.
    pub(crate) fn sessions(&self) -> &crate::sessions::SessionsSubsystem {
        &self.inner.sessions
    }

    /// What the sessions subsystem says the core must do.
    fn session_event(&self, event: crate::sessions::SessionEvent) {
        match event {
            crate::sessions::SessionEvent::Problem(problem) => self.notice(problem),
        }
    }

    /// The notices.
    pub(crate) fn notices(&self) -> &crate::notices::NoticesSubsystem {
        &self.inner.notices
    }

    /// The launcher's own update.
    pub(crate) fn launcher_update(&self) -> &crate::launcher_update::LauncherUpdateSubsystem {
        &self.inner.launcher_update
    }

    /// "Download only through a VPN", and the VPNs.
    pub(crate) fn vpn(&self) -> &crate::vpn::VpnSubsystem {
        &self.inner.vpn
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, AppState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Each change of the state, by its area, from now on. The app renders on
    /// [`Event::StateChanged`] instead, which every change also publishes.
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<Change> {
        self.inner.changes.subscribe()
    }

    /// The state in `area` changed: the core's readers learn which part, and the app that
    /// something did.
    pub(crate) fn changed(&self, area: Area) {
        self.inner.changes.publish(area);
        self.inner.kernel.events().publish(Event::StateChanged);
    }

    /// Posts a notice. The change that goes with it publishes [`Event::StateChanged`].
    fn post(&self, kind: NoticeKind, message: String, route: Option<Route>, game: Option<GameKey>) {
        self.notices().post(kind, message, route, game);
    }

    /// A notice of the default kind, `Info`.
    pub(crate) fn notice(&self, message: String) {
        self.post(NoticeKind::Info, message, None, None);
    }

    /// A notice of the `Error` kind: something failed.
    pub(crate) fn problem(&self, message: String) {
        self.post(NoticeKind::Error, message, None, None);
    }

    /// A notice whose subject `route` shows: clicking it opens that screen.
    pub(crate) fn notice_opening(&self, kind: NoticeKind, route: Route, message: String) {
        self.post(kind, message, Some(route), None);
    }

    /// A problem with one game: its card shows the game's art.
    pub(crate) fn game_problem(&self, game: &GameKey, message: String) {
        self.post(NoticeKind::Error, message, None, Some(game.clone()));
    }

    /// Shows the player a problem outside the core, such as a theme's logic failing.
    pub fn report(&self, message: String) {
        self.notices().report(message);
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
        self.torrents().shutdown().await;
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
                self.changed(Area::Navigation);
                Ok(())
            }
            Command::Select(game) => {
                self.select(&game);
                self.changed(Area::Navigation);
                self.prefetch_around(&game);
                // Themes show the selected game's details on the home screen, so load them
                // once. Wait first: a player who scrolls past a game doesn't need its details.
                // A plugin that fails here leaves one notice; the selection stands.
                let wanted = matches!(self.library().snapshot().detail_state(&game), DetailState::Unread);
                if !wanted {
                    return Ok(());
                }
                self.kernel().clock().sleep(SELECT_SETTLE).await;
                let still = {
                    let state = self.state();
                    state.selected.as_ref() == Some(&game) || state.catalog_selected.as_ref() == Some(&game)
                };
                if !still {
                    return Ok(());
                }
                match self.load_detail(&game).await {
                    Ok(()) => self.changed(Area::Library),
                    Err(e) => {
                        self.library().detail_failed(&game);
                        self.problem(e.to_string());
                    }
                }
                Ok(())
            }
            Command::OpenGame(game) => {
                self.torrents().set_download_error(None);
                self.open_game(game).await
            }
            Command::Library(crate::LibraryCmd::Refresh) => self.refresh_library().await,
            Command::Library(command) => self.library().handle(command),
            Command::UninstallGame(game) => self.uninstall_game(&game).await,
            Command::Search(text) => self.search(text).await,
            Command::Download { plugin, request } => self.download(&plugin, request).await,
            Command::PauseDownload(job) => self.inner.downloads.pause(job).await,
            Command::ResumeDownload(job) => self.inner.downloads.resume(job).await,
            Command::CancelDownload(job) if self.installs().cancel(job) => Ok(()),
            Command::CancelDownload(job) => self.inner.downloads.cancel(job).await,
            Command::InstallEmulator { plugin, version } => self.install(&plugin, version, None).await,
            Command::UpdateEmulator { plugin } => self.update(&plugin).await,
            Command::UninstallEmulator { plugin, version } => self.uninstall(&plugin, &version).await,
            Command::Session(command) => crate::launch::session(self, command).await,
            Command::SetConfig { plugin, scope, value } => self.set_config(&plugin, scope, value).await,
            Command::SetConfigField { plugin, scope, key, value } => {
                self.set_config_field(&plugin, scope, key, value).await
            }
            Command::SetThemeOption { key, value } => self.set_theme_option(key, value),
            Command::RunAction { plugin, action, game, args } => self.run_action(&plugin, &action, game, args).await,
            Command::ThemeSwitched(theme) => self.theme_switched(theme),
            Command::SetFolders { name, paths } => self.set_folders(&name, &paths),
            Command::SetPluginEnabled { plugin, enabled } => self.set_plugin_enabled(&plugin, enabled).await,
            Command::Pref(pref, value) => self.preferences().set(pref, value).await,
            Command::ReportResult(level) => {
                let Some((game, build)) = self.state().result_prompt.take() else { return Ok(()) };
                self.library().answer(&game, level.as_deref(), build.as_deref())?;
                self.changed(Area::Sessions);
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
            Command::CatalogGenre(genre) => self.set_grid(true, |v| v.genre = genre),
            Command::CatalogSort(sort) => {
                check_sort(&sort)?;
                self.set_grid(true, |v| v.sort = sort)
            }
            Command::CatalogMore => self.more(true),
            Command::Torrent(command) => self.torrent(command).await,
            Command::GameInstall(command) => self.game_install(command).await,
            Command::Vpn(command) => self.vpn().handle(command).await,
            Command::AskResult(game) => {
                let build = self.build_of(&game.plugin);
                self.state().result_prompt = Some((game, build));
                self.changed(Area::Sessions);
                Ok(())
            }
            Command::System(command) => crate::system::run(self, command).await,
            Command::Notice(crate::NoticeCmd::Open(id)) => match self.notices().take(id).and_then(|n| n.route) {
                Some(route) => self.navigate(route).await,
                None => Ok(()),
            },
            Command::Notice(command) => self.notices().handle(command),
            Command::LauncherUpdate(crate::LauncherUpdateCmd::Check) => {
                let found = self.launcher_update().check().await;
                // Nexus OS brings the new launcher with its next update: look for that now.
                if matches!(found, crate::launcher_update::UpdateState::WithSystem { .. }) {
                    let core = self.clone();
                    tokio::spawn(async move {
                        let _ = crate::system::run(&core, crate::system::SystemCommand::CheckOsUpdate).await;
                    });
                }
                Ok(())
            }
            Command::LauncherUpdate(command) => self.launcher_update().handle(command).await,
            Command::CheckUpdates => {
                self.check_updates().await;
                Ok(())
            }
            Command::ConfirmInstall => {
                let Some(prompt) = self.state().install_prompt.take() else { return Ok(()) };
                self.changed(Area::Sessions);
                self.install(&prompt.game.plugin, Some(prompt.version), None).await?;
                crate::launch::launch(self, prompt.game).await
            }
            Command::CancelInstall => {
                self.state().install_prompt = None;
                self.changed(Area::Sessions);
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
                self.vpn().refresh().await;
                self.refresh_settings().await;
            }
            // Read once; a new catalog address reads them again. The Catalog opens at once and
            // says it is loading until the read ends.
            Route::Catalog if !self.library().has_catalogs() => {
                self.push(route);
                self.refresh_releases().await;
                return Ok(());
            }
            Route::GameSettings(game) if self.library().snapshot().detail(game).is_none() => {
                self.load_detail(game).await?
            }
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
        self.changed(Area::Navigation);
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

    /// Searches the Catalog, and shows it.
    async fn search(&self, text: String) -> CoreResult<()> {
        if !self.library().has_catalogs() {
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
        self.changed(Area::Navigation);
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
        self.changed(Area::Navigation);
        if catalog {
            self.describe_catalog_page();
        }
        Ok(())
    }

    /// Selects a game: a game the catalogs offer in the Catalog, any other on Home.
    fn select(&self, game: &GameKey) {
        let offered = self.library().snapshot().offer(game).is_some();
        let mut state = self.state();
        if offered {
            state.catalog_selected = Some(game.clone());
        } else {
            state.selected = Some(game.clone());
        }
    }

    async fn open_game(&self, game: GameKey) -> CoreResult<()> {
        // A game a catalog offers: its download sheet names the VPN in use.
        if self.vpn().only() && self.library().snapshot().offer(&game).is_some() {
            self.vpn().refresh().await;
        }
        self.load_detail(&game).await?;
        self.select(&game);
        self.push(Route::GameDetail(game));
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
        if let Some(slots) = &provided.slots {
            let request = SlotRequest { slot: SlotId::EmulatorPanel, game: None };
            if let Ok(items) = kernel.guard(plugin, "slots.items", slots.items(&request)).await {
                emu.panels = items;
            }
        }
        self.state().emulators.insert(plugin.clone(), emu);
    }

    pub(crate) async fn refresh_emulators(&self) {
        for plugin in self.inner.kernel.of_kind(PluginKind::Emulator) {
            self.refresh_emulator(&plugin).await;
        }
    }

    async fn refresh_settings(&self) {
        let kernel = &self.inner.kernel;
        let mut panels = Vec::new();
        for (plugin, provided) in self.providers(Capability::Slots).await {
            let slots = provided.slots.clone().expect("declared and checked at init");
            let request = SlotRequest { slot: SlotId::SettingsPanel, game: None };
            let Ok(items) = kernel.guard(&plugin, "slots.items", slots.items(&request)).await else { continue };
            let items =
                crate::order::sort_by_order(items.into_iter().map(|i| (i.id.clone(), i.order.clone(), i)).collect());
            for item in items {
                let scope = match &item.content {
                    nexus_api::SlotContent::SettingsPanel { scope, .. } => scope.clone(),
                    _ => ConfigScope::Emulator,
                };
                let schema = match &provided.configurator {
                    Some(c) => {
                        kernel.guard(&plugin, "configurator.schema", c.schema(&scope)).await.unwrap_or(Value::Null)
                    }
                    None => Value::Null,
                };
                let values = self.config_value(&plugin, &scope).await.unwrap_or(Value::Null);
                panels.push(SettingsPanel { plugin: plugin.clone(), item, schema, values });
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
        self.changed(Area::Downloads);
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
        self.changed(Area::Downloads);
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
        self.changed(Area::Downloads);
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
        let link;
        if self.sessions().runs(game) {
            return Err(CoreError::Invalid("Stop the game before you uninstall it".into()));
        }
        let installs = self.installs().snapshot();
        let (folder, title) = {
            let library = self.library().snapshot();
            if installs.installing(&game.plugin) {
                return Err(CoreError::Invalid("Wait until the install ends".into()));
            }
            let details = library.detail(game).map(|d| &d.details);
            let folder = details.and_then(|d| d.folder.clone());
            also = details.map(|d| d.also.clone()).unwrap_or_default();
            // A store's game: the store removes it (Steam asks in its own dialog).
            link = details.and_then(|d| Some((d.uninstall_link.clone()?, d.summary.platform.clone())));
            (folder, library.title(game))
        };
        if let Some((link, platform)) = link {
            let allowed = self.inner.kernel.manifest(&game.plugin)?.permissions.allows_link(&link);
            if !allowed {
                return Err(CoreError::PermissionDenied {
                    plugin: game.plugin.clone(),
                    action: format!("open {link}"),
                });
            }
            let opening = link.clone();
            crate::running::open_link(self, opening).await.map_err(CoreError::Invalid)?;
            let store = crate::view::system_of(&self.inner.kernel, &platform).name;
            tracing::info!(%game, %link, "asked the store to uninstall");
            self.notice(format!("{store} is uninstalling {title}: it leaves your Library when it's done"));
            self.changed(Area::Library);
            return Ok(());
        }
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
        self.library().forget_detail(game);
        {
            let mut state = self.state();
            state.nav.retain(|r| r != &Route::GameDetail(game.clone()));
            if state.nav.is_empty() {
                state.nav.push(Route::Home);
            }
        }
        self.refresh_library().await?;
        self.notice(format!("Uninstalled {title}"));
        self.changed(Area::Library);
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
            self.library().set_game_settings(&game, panel);
        }
        self.changed(Area::Plugins);
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
        self.changed(Area::Plugins);
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
        let value = crate::settings_schema::typed(&schema, &key, text)?;
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
            self.library().clear_catalogs();
            let mut state = self.state();
            state.nav.retain(|r| *r != Route::Catalog);
            state.nav.dedup();
            if state.nav.is_empty() {
                state.nav.push(Route::Home);
            }
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
        self.changed(Area::Plugins);
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
        self.changed(Area::Settings);
        Ok(())
    }

    /// Runs [`Command::CheckUpdates`] every few hours while the app runs.
    fn schedule_update_checks(&self) {
        let weak = Arc::downgrade(&self.inner);
        let clock = self.kernel().clock().clone();
        tokio::spawn(async move {
            clock.sleep(FIRST_UPDATE_CHECK).await;
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let core = Core { inner };
                if core.prefs().auto_update {
                    core.check_updates().await;
                }
                drop(core);
                clock.sleep(UPDATE_CHECK_EVERY).await;
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
            if emu.installed.is_empty() || emu.own || view::update_for(&emu).is_none() || !self.prefs().auto_update {
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
        self.changed(Area::Downloads);
    }

    /// One of the emulator's games is running.
    fn plays(&self, plugin: &PluginId) -> bool {
        self.sessions().plays(plugin)
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
            core.changed(Area::Downloads);
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
        let title = self.library().snapshot().title(game);
        self.state().install_prompt = Some(InstallPrompt { game: game.clone(), emulator, version, title, size_bytes });
        self.changed(Area::Sessions);
        Ok(true)
    }

    /// The game's folder on disk, through its plugin's folder grants. `None` before its
    /// details are in, or when it has no folder.
    pub fn game_folder(&self, game: &GameKey) -> Option<std::path::PathBuf> {
        let folder = self.library().snapshot().detail(game)?.details.folder.clone()?;
        resolve_game_folder(&self.inner.kernel, game, &folder)
    }

    /// The folder of the download `id`, for its Open folder: once it installed, the installed
    /// game's folder. `None` before the folder is on disk.
    pub fn download_folder(&self, id: &str) -> Option<std::path::PathBuf> {
        let job = self.torrents().job(id)?;
        let game = self.installs().snapshot().latest(&job.emulator, &job.source).and_then(|g| g.installed.clone());
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
        Some(crate::sessions::log_path(&self.inner.kernel.host().data_dir, game)).filter(|p| p.is_file())
    }

    /// The emulator build `plugin` runs games on: its newest installed one.
    pub(crate) fn build_of(&self, plugin: &PluginId) -> Option<String> {
        self.state().emulators.get(plugin).and_then(|e| e.installed.last().cloned())
    }

    /// The first library load of the run, with the store's art and details, is done.
    pub fn booted(&self) -> bool {
        self.library().booted()
    }

    /// A game is running.
    pub fn game_running(&self) -> bool {
        self.sessions().running()
    }

    /// The running games' sessions, to stop them.
    pub fn running_sessions(&self) -> Vec<SessionId> {
        self.sessions().session_ids()
    }

    /// A game has the screen: the gamepad is the game's, not the launcher's. The desktop says
    /// which window is in front; when it can't, the last window the launcher showed counts.
    /// Cheap: it reads the sessions' state, so the UI can ask on every press.
    pub fn game_in_front(&self) -> bool {
        self.sessions().game_in_front()
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
        self.changed(Area::Settings);
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
        self.preferences().save(Pref::Theme, Value::from(theme.as_str()))?;
        self.inner.kernel.events().publish(Event::ThemeChanged { from, to: theme });
        Ok(())
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
