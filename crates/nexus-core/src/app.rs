//! The public app API: commands in, view models and events out.
//!
//! All state lives in the core, never in a theme, so the UI can be rebuilt (or a theme
//! swapped) at any time without losing navigation, selection, downloads or running games.
//!
//! `Core` is a router. It starts the subsystems over the ports bundle, sends each
//! [`Command`] to the subsystem or the flow that owns it, merges the subsystems' changes, and
//! reads them all into one [`Snapshot`] for the view. A flow is a free function over the core
//! in its own module (`navigation`, `emulator_builds`, `library_reads`, ...): it runs work that
//! crosses areas, such as a plugin read whose answer goes to a subsystem and posts notices.

use std::sync::{Arc, Mutex, MutexGuard};

use futures::stream::BoxStream;
use nexus_api::{Capability, ConfigScope, DownloadRequest, Event, GameKey, JobId, PluginId, Provided, SessionId};
use nexus_ui_api::AppView;
use serde_json::Value;

use crate::changes::{Area, Change, Changes, Notifier};
use crate::downloads::Downloads;
use crate::preferences::{Pref, Preferences, PreferencesSnapshot};
use crate::snapshot::Snapshot;
use crate::view;
// The view reads them from here.
pub(crate) use crate::app_state::{AppState, SettingsPanel};
use crate::{CoreError, CoreResult, Kernel};
use crate::{emulator_builds, library_reads, navigation, plugin_downloads, plugin_settings, release_downloads};

/// Where the player is. The navigation stack is a list of routes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Grid {
    /// The Library's grid, of the installed games.
    Library,
    /// The Catalog's grid, of the games the catalogs offer.
    Catalog,
}

/// Everything the UI can ask the core to do. It reads and writes as JSON with `serde`, each
/// variant by its name: `{"Navigate": "Settings"}`, `"Back"`, `{"Torrent": {"SeedDownloads": true}}`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
        #[serde(deserialize_with = "crate::typed_text::folder_list")]
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
    /// The Library's system (a platform label, such as `PS5`), or a filter to turn on or off;
    /// `None` clears the system and every filter.
    LibrarySystem(Option<crate::SystemChoice>),
    /// The Library's genre, or every genre.
    LibraryGenre(Option<String>),
    /// How the Library is sorted: `title`, `played`, `size` (largest first), `smallest`,
    /// `rating`, `release` (newest first), `compat` or `added`.
    LibrarySort(crate::Sort),
    /// The Library shows its next page of games ([`crate::GRID_PAGE`] more).
    LibraryMore,
    /// The Catalog's search text, system, genre and sort, as the Library's.
    CatalogSearch(String),
    CatalogSystem(Option<crate::SystemChoice>),
    CatalogGenre(Option<String>),
    CatalogSort(crate::Sort),
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

/// The player's folders as `game`'s plugin may use them.
pub(crate) fn game_folders(kernel: &Kernel, game: &GameKey) -> Option<crate::folders::HostFolders> {
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

/// Where a game's folder is on disk, through its plugin's folder grants; `None` when the
/// grants refuse it or it isn't a folder.
pub(crate) fn resolve_game_folder(
    kernel: &Kernel,
    game: &GameKey,
    folder: &nexus_api::HostPath,
) -> Option<std::path::PathBuf> {
    game_folders(kernel, game)?.resolve(folder).ok().filter(|p| p.is_dir())
}

pub(crate) struct Inner {
    pub kernel: Kernel,
    /// Every port the app plugged in, split per subsystem when the core starts.
    pub ports: crate::ports::Ports,
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
    /// Held while the catalogs are read: see [`crate::catalog_reads::refresh_releases`].
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
        let state = AppState::new(theme);
        let ports = kernel.ports().clone();
        let downloads = Downloads::spawn(crate::downloads::DownloadContext {
            ports: ports.downloads(),
            host: kernel.host().clone(),
            permissions: kernel
                .order()
                .iter()
                .filter_map(|id| Some((id.clone(), kernel.manifest(id).ok()?.permissions.clone())))
                .collect(),
            config: kernel.config().clone(),
            bus: kernel.events().clone(),
        });
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
                        crate::release_downloads::torrent_event(&Core { inner }, event);
                    }
                })
            };
            let torrents = crate::torrents::TorrentsSubsystem::new(
                ports.torrents(),
                crate::torrents::Polls { torrents: kernel.host().torrent_poll, vpn: kernel.host().vpn_poll },
                crate::torrents::Records::new(kernel.config().clone()),
                ports.vpn.clone(),
                Notifier::new(changes.clone(), kernel.events().clone()),
                events,
            );
            let preferences = Preferences::new(
                kernel.config().clone(),
                ports.preferences(),
                Notifier::new(changes.clone(), kernel.events().clone()),
            );
            let install_events: crate::game_installs::EventSink = Arc::new(move |event| {
                let core = weak.upgrade().map(|inner| Core { inner });
                Box::pin(async move {
                    if let Some(core) = core {
                        crate::release_downloads::install_event(&core, event).await;
                    }
                })
            });
            let installs = crate::game_installs::GameInstallsSubsystem::new(
                ports.installs(Arc::new(torrents.clone()), kernel.host().call_timeout),
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
                ports.sessions(),
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
                ports.launcher_update(),
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
                ports.vpn.clone(),
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
                ports,
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
        crate::emulator_builds::resume_installs(&core).await;
        crate::plugin_downloads::resume_downloads(&core);
        crate::emulator_builds::schedule_update_checks(&core);
        crate::system::schedule_refresh(&core);
        crate::hardware::check_hardware(&core);
        crate::catalog_reads::watch_catalogs(&core);
        crate::catalog_reads::watch_store_installs(&core);
        core
    }

    pub fn kernel(&self) -> &Kernel {
        &self.inner.kernel
    }

    /// Every port the app plugged in.
    pub(crate) fn ports(&self) -> &crate::ports::Ports {
        &self.inner.ports
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
            crate::sessions::SessionEvent::Problem(problem) => self.notices().info(problem),
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

    /// Shows the player a problem outside the core, such as a theme's logic failing.
    pub fn report(&self, message: String) {
        self.notices().report(message);
    }

    /// The whole app, read for a render: every subsystem once, a copy of the state no
    /// subsystem owns, and what the view needs from the plugins, the config and the disk. The
    /// subsystems are read before that state, as the core never holds its lock while it takes a
    /// subsystem's.
    pub(crate) fn snapshot(&self) -> Snapshot<'_> {
        let kernel = self.kernel();
        let library = self.library().snapshot();
        let torrents = self.torrents().snapshot();
        let state = self.state().clone();
        let detail_game = crate::snapshot::detail_game(&state);
        let files = crate::snapshot::files(kernel, &library, &torrents, detail_game.as_ref());
        Snapshot {
            plugins: crate::snapshot::Plugins::new(kernel),
            installs: self.installs().snapshot(),
            sessions: self.sessions().snapshot(),
            prefs: self.prefs(),
            notices: self.notices().snapshot(),
            launcher_update: self.launcher_update().snapshot(),
            vpn: self.vpn().snapshot(),
            downloads: self.inner.downloads.jobs(),
            theme_options: state.theme.as_ref().map(|theme| self.theme_options(theme)).unwrap_or(Value::Null),
            hidden_filter: crate::navigation::hidden_filter_on(kernel),
            can_start_at_login: self.ports().login_session.can_start_at_login(),
            now: self.ports().clock.now(),
            now_secs: crate::sessions::now_secs(),
            library,
            torrents,
            state,
            detail_game,
            files,
        }
    }

    /// The view models for the current state. The Catalog's grid, once built on the Catalog,
    /// stands on the other screens.
    pub fn view(&self) -> AppView {
        let snapshot = self.snapshot();
        let view = view::build(&snapshot);
        let on_catalog = snapshot.state.nav.last() == Some(&Route::Catalog);
        self.state().catalog_grid.keep(on_catalog, &view.catalog.games);
        view
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

    /// Runs a command: each goes to the subsystem or the flow that owns it.
    pub async fn dispatch(&self, command: Command) -> CoreResult<()> {
        tracing::debug!(?command, "dispatch");
        let result = match command {
            Command::Navigate(route) => navigation::navigate(self, route).await,
            Command::Back => navigation::back(self),
            Command::Select(game) => navigation::select_game(self, game).await,
            Command::OpenGame(game) => navigation::open_game_anew(self, game).await,
            Command::Search(text) => navigation::search(self, text).await,
            Command::LibrarySearch(text) => navigation::set_grid(self, Grid::Library, |v| v.query = text),
            Command::LibrarySystem(system) => navigation::set_grid(self, Grid::Library, |v| v.set_system(system)),
            Command::LibraryGenre(genre) => navigation::set_grid(self, Grid::Library, |v| v.genre = genre),
            Command::LibrarySort(sort) => navigation::set_grid(self, Grid::Library, |v| v.sort = sort),
            Command::LibraryMore => navigation::more(self, Grid::Library),
            Command::CatalogSearch(text) => navigation::set_grid(self, Grid::Catalog, |v| v.query = text),
            Command::CatalogSystem(system) => navigation::set_grid(self, Grid::Catalog, |v| v.set_system(system)),
            Command::CatalogGenre(genre) => navigation::set_grid(self, Grid::Catalog, |v| v.genre = genre),
            Command::CatalogSort(sort) => navigation::set_grid(self, Grid::Catalog, |v| v.sort = sort),
            Command::CatalogMore => navigation::more(self, Grid::Catalog),
            Command::GridColumns { grid, columns } => navigation::set_columns(self, grid, columns),
            Command::Library(crate::LibraryCmd::Refresh) => library_reads::refresh_library(self).await,
            Command::Library(command) => self.library().handle(command),
            Command::UninstallGame(game) => crate::uninstall::uninstall_game(self, &game).await,
            Command::Download { plugin, request } => plugin_downloads::download(self, &plugin, request).await,
            Command::PauseDownload(job) => self.inner.downloads.pause(job).await,
            Command::ResumeDownload(job) => self.inner.downloads.resume(job).await,
            Command::CancelDownload(job) if self.installs().cancel(job) => Ok(()),
            Command::CancelDownload(job) => self.inner.downloads.cancel(job).await,
            Command::InstallEmulator { plugin, version } => {
                emulator_builds::install(self, &plugin, version, None).await
            }
            Command::UpdateEmulator { plugin } => emulator_builds::update(self, &plugin).await,
            Command::UninstallEmulator { plugin, version } => emulator_builds::uninstall(self, &plugin, &version).await,
            Command::CheckUpdates => {
                emulator_builds::check_updates(self).await;
                Ok(())
            }
            Command::ConfirmInstall => emulator_builds::confirm_install(self).await,
            Command::CancelInstall => emulator_builds::cancel_install(self),
            Command::Session(command) => crate::launch::session(self, command).await,
            Command::AskResult(game) => crate::launch::ask_result(self, game),
            Command::ReportResult(level) => crate::launch::report_result(self, level),
            Command::SetConfig { plugin, scope, value } => {
                plugin_settings::set_config(self, &plugin, scope, value).await
            }
            Command::SetConfigField { plugin, scope, key, value } => {
                plugin_settings::set_config_field(self, &plugin, scope, key, value).await
            }
            Command::SetThemeOption { key, value } => plugin_settings::set_theme_option(self, key, value),
            Command::RunAction { plugin, action, game, args } => {
                plugin_settings::run_action(self, &plugin, &action, game, args).await
            }
            Command::SetFolders { name, paths } => plugin_settings::set_folders(self, &name, &paths),
            Command::SetPluginEnabled { plugin, enabled } => {
                plugin_settings::set_plugin_enabled(self, &plugin, enabled).await
            }
            Command::ThemeSwitched(theme) => self.theme_switched(theme),
            Command::Pref(pref, value) => self.preferences().set(pref, value).await,
            Command::Torrent(command) => release_downloads::torrent(self, command).await,
            Command::GameInstall(command) => release_downloads::game_install(self, command).await,
            Command::Vpn(command) => self.vpn().handle(command).await,
            Command::System(command) => crate::system::run(self, command).await,
            Command::Notice(crate::NoticeCmd::Open(id)) => navigation::open_notice(self, id).await,
            Command::Notice(command) => self.notices().handle(command),
            Command::LauncherUpdate(crate::LauncherUpdateCmd::Check) => {
                crate::system::check_launcher_update(self).await
            }
            Command::LauncherUpdate(command) => self.launcher_update().handle(command).await,
        };
        if let Err(error) = &result {
            tracing::warn!(%error, "command failed");
        }
        result
    }

    /// Activates every plugin with `capability`, skipping (with a notice) those that fail.
    pub(crate) async fn providers(&self, capability: Capability) -> Vec<(PluginId, Provided)> {
        let mut out = Vec::new();
        for plugin in self.inner.kernel.providers(capability) {
            match self.inner.kernel.provided(&plugin).await {
                Ok(provided) => out.push((plugin, provided)),
                Err(e) => self.notices().problem(e.to_string()),
            }
        }
        out
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
                let response = crate::facades::send_checked(self.ports().http.as_ref(), plugin, permissions, request)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(command: Command) {
        let json = serde_json::to_string(&command).expect("a command serializes");
        let back: Command = serde_json::from_str(&json).expect("a command deserializes");
        assert_eq!(back, command, "{json}");
    }

    #[test]
    fn commands_read_as_json_by_their_variant_names() {
        let read = |json: &str| serde_json::from_str::<Command>(json).expect(json);
        assert_eq!(read(r#"{"Navigate": "Settings"}"#), Command::Navigate(Route::Settings));
        assert_eq!(read(r#""Back""#), Command::Back);
        assert_eq!(
            read(r#"{"Torrent": {"SeedDownloads": true}}"#),
            Command::Torrent(crate::TorrentCmd::SeedDownloads(true))
        );
        assert_eq!(read(r#"{"Pref": ["sounds", false]}"#), Command::Pref(Pref::Sounds, Value::Bool(false)));
    }

    #[test]
    fn every_kind_of_command_goes_to_json_and_back() {
        let game = GameKey::new(PluginId::new("emu").unwrap(), "astro");
        round_trip(Command::Navigate(Route::GameDetail(game.clone())));
        round_trip(Command::Select(game.clone()));
        round_trip(Command::GridColumns { grid: Grid::Catalog, columns: 6 });
        round_trip(Command::SetConfig {
            plugin: game.plugin.clone(),
            scope: ConfigScope::Game("astro".into()),
            value: serde_json::json!({ "fps": 60 }),
        });
        round_trip(Command::Session(crate::SessionCmd::Launch(game.clone())));
        round_trip(Command::Torrent(crate::TorrentCmd::Act { id: "abc".into(), action: crate::TorrentAction::Pause }));
        round_trip(Command::System(crate::system::SystemCommand::JoinWifi {
            ssid: "home".into(),
            password: Some("secret".into()),
        }));
        round_trip(Command::SetFolders { name: "games".into(), paths: vec!["/games".into()] });
        round_trip(Command::ReportResult(None));
    }
}
