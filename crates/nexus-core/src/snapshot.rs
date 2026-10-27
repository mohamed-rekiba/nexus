//! The app's state, read at once for a render: one read of each subsystem, a copy of the
//! core's own state, and what the view needs from the plugins, the config and the disk.
//! [`take`] does every read; [`crate::view::build`] then works on the [`Snapshot`] alone, with
//! no lock and no I/O.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nexus_api::{Capability, GameKey, PluginId, PluginKind, PluginManifest};
use nexus_ui_api::GameSystemVm;
use serde_json::Value;

use crate::app::{AppState, Route};
use crate::game_installs::InstallsSnapshot;
use crate::grid_query::GridData;
use crate::launcher_update::LauncherUpdateSnapshot;
use crate::library::LibrarySnapshot;
use crate::notices::Notice;
use crate::ownership::Ownership;
use crate::sessions::SessionsSnapshot;
use crate::torrents::TorrentsSnapshot;
use crate::vpn::VpnSnapshot;
use crate::{Core, HostConfig, Job, Kernel, PluginStatus, PreferencesSnapshot};

/// Everything a render shows.
pub(crate) struct Snapshot<'a> {
    pub plugins: Plugins<'a>,
    pub library: LibrarySnapshot,
    pub torrents: TorrentsSnapshot,
    pub installs: InstallsSnapshot,
    pub sessions: SessionsSnapshot,
    pub prefs: PreferencesSnapshot,
    pub notices: Arc<Vec<Notice>>,
    pub launcher_update: LauncherUpdateSnapshot,
    pub vpn: VpnSnapshot,
    /// The emulator downloads.
    pub downloads: Vec<Job>,
    /// The core's own state: the screens, the selection, the grids, the emulators' builds, the
    /// settings panels and the rest no subsystem holds yet.
    pub state: AppState,
    /// The game whose Game Hub the view shows (see [`detail_game`]).
    pub detail_game: Option<GameKey>,
    /// What the view reads from the disk.
    pub files: Files,
    /// The player's option values for the current theme.
    pub theme_options: Value,
    /// The Catalog shows its "Hidden" filter: no catalog plugin turned it off.
    pub hidden_filter: bool,
    /// The system can start Nexus at login.
    pub can_start_at_login: bool,
    /// Now, by the core's clock.
    pub now: Instant,
    /// Now, in seconds since 1970: for "new" releases and the days since a game was played.
    pub now_secs: u64,
}

impl Snapshot<'_> {
    /// The links between the offers and the installed games.
    pub fn ownership(&self) -> Ownership<'_> {
        Ownership::new(&self.library, &self.torrents.jobs, &self.installs)
    }

    /// What the grids read.
    pub fn grid_data<'a>(&'a self, ownership: &'a Ownership<'a>) -> GridData<'a> {
        GridData { library: &self.library, ownership, plays: &self.sessions.plays, hidden_filter: self.hidden_filter }
    }

    /// The games beside `game` in its row or grid, at most `most` (see
    /// [`crate::grid_query::neighbours`]).
    pub fn neighbours(&self, game: &GameKey, most: usize) -> Vec<GameKey> {
        let ownership = self.ownership();
        let data = self.grid_data(&ownership);
        let state = &self.state;
        crate::grid_query::neighbours(&data, &state.library_view, &state.catalog_view, &state.grid_columns, game, most)
    }

    /// The games on the Catalog's shown pages, in order.
    pub fn catalog_page(&self) -> Vec<GameKey> {
        let ownership = self.ownership();
        self.state.catalog_view.page(&self.grid_data(&ownership))
    }

    /// Days since 1970, today.
    pub fn today(&self) -> i64 {
        (self.now_secs / 86_400) as i64
    }
}

/// What the view reads from the disk, read with the snapshot.
#[derive(Default)]
pub(crate) struct Files {
    /// The games whose folder is there: the Library's games with their details in, and the
    /// game of the Game Hub.
    pub game_folders: HashSet<GameKey>,
    /// The Game Hub's game has a log of its last run.
    pub has_log: bool,
    /// Where each downloading emulator installs games, for the downloads' cards.
    pub install_folders: HashMap<PluginId, PathBuf>,
    /// The host folders' paths, by the folder's name, as the first plugin that asks for each
    /// sees them.
    pub roots: HashMap<String, Vec<PathBuf>>,
}

/// The loaded plugins: their manifests, which are on, and how each is.
pub(crate) struct Plugins<'a> {
    host: &'a HostConfig,
    order: &'a [PluginId],
    manifests: BTreeMap<&'a PluginId, &'a PluginManifest>,
    enabled: BTreeSet<PluginId>,
    statuses: BTreeMap<PluginId, PluginStatus>,
}

impl<'a> Plugins<'a> {
    pub fn new(kernel: &'a Kernel) -> Self {
        let order = kernel.order();
        Self {
            host: kernel.host(),
            order,
            manifests: order.iter().filter_map(|id| Some((id, kernel.manifest(id).ok()?))).collect(),
            enabled: order.iter().filter(|id| kernel.enabled(id)).cloned().collect(),
            statuses: kernel.statuses(),
        }
    }

    pub fn host(&self) -> &'a HostConfig {
        self.host
    }

    /// Loaded plugins, dependencies first.
    pub fn order(&self) -> &'a [PluginId] {
        self.order
    }

    pub fn manifest(&self, plugin: &PluginId) -> Option<&'a PluginManifest> {
        self.manifests.get(plugin).copied()
    }

    /// The plugin's name; its id when it isn't loaded.
    pub fn name(&self, plugin: &PluginId) -> String {
        self.manifest(plugin).map_or_else(|| plugin.to_string(), |m| m.name.clone())
    }

    /// Whether the player has the plugin on.
    pub fn enabled(&self, plugin: &PluginId) -> bool {
        self.enabled.contains(plugin)
    }

    /// The plugin is made for other systems than this one.
    pub fn elsewhere(&self, plugin: &PluginId) -> bool {
        let platform = self.host.platform;
        self.manifest(plugin).is_some_and(|m| platform.is_some_and(|p| !m.supports(p)))
    }

    pub fn status(&self, plugin: &PluginId) -> Option<&PluginStatus> {
        self.statuses.get(plugin)
    }

    /// Turned-on plugins of one kind, in load order.
    pub fn of_kind(&self, kind: PluginKind) -> Vec<PluginId> {
        self.on(|m| m.kind == kind)
    }

    /// Turned-on plugins whose manifest declares `capability`, in load order.
    pub fn providers(&self, capability: Capability) -> Vec<PluginId> {
        self.on(|m| m.capabilities.contains(&capability))
    }

    fn on(&self, pick: impl Fn(&PluginManifest) -> bool) -> Vec<PluginId> {
        let picked = |id: &&PluginId| self.enabled(id) && self.manifest(id).is_some_and(&pick);
        self.order.iter().filter(picked).cloned().collect()
    }

    /// A turned-on plugin gives release catalogs, so there is a Catalog.
    pub fn catalog_available(&self) -> bool {
        !self.providers(Capability::Releases).is_empty()
    }

    /// The system `platform` (`PS5`), as the plugin whose `[system]` it is describes it: its
    /// full name and color. Without one, the code is its name and it has no color.
    pub fn system(&self, platform: &str) -> GameSystemVm {
        let look =
            self.order.iter().filter_map(|id| self.manifest(id)?.system.as_ref()).find(|look| look.id == platform);
        match look {
            Some(look) => GameSystemVm { name: look.name.clone(), color: look.rgb() },
            None => GameSystemVm { name: platform.to_owned(), color: None },
        }
    }
}

/// The game whose Game Hub the view shows: the one open, else the one the player last opened
/// under the overlays, else the selected game (in the Catalog, the Catalog's).
pub(crate) fn detail_game(state: &AppState) -> Option<GameKey> {
    let route = state.nav.last().cloned().unwrap_or(Route::Home);
    let open = match &route {
        Route::GameDetail(g) | Route::GameSettings(g) => Some(g.clone()),
        _ => None,
    };
    open.or_else(|| {
        state.nav.iter().rev().find_map(|r| if let Route::GameDetail(g) = r { Some(g.clone()) } else { None })
    })
    .or_else(|| if route == Route::Catalog { state.catalog_selected.clone() } else { None })
    .or_else(|| state.selected.clone())
}

/// Reads the app for a render. The subsystems are read before the core's own state, as the
/// core never holds that lock while it takes a subsystem's.
pub(crate) fn take(core: &Core) -> Snapshot<'_> {
    let kernel = core.kernel();
    let library = core.library().snapshot();
    let torrents = core.torrents().snapshot();
    let installs = core.installs().snapshot();
    let sessions = core.sessions().snapshot();
    let prefs = core.prefs();
    let notices = core.notices().snapshot();
    let launcher_update = core.launcher_update().snapshot();
    let vpn = core.vpn().snapshot();
    let downloads = core.inner.downloads.jobs();
    let state = core.state().clone();
    let plugins = Plugins::new(kernel);
    let detail_game = detail_game(&state);
    let files = files(kernel, &library, &torrents, detail_game.as_ref());
    let theme_options = state.theme.as_ref().map(|theme| core.theme_options(theme)).unwrap_or(Value::Null);
    Snapshot {
        hidden_filter: Core::hidden_filter_on(kernel),
        can_start_at_login: kernel.login_session().can_start_at_login(),
        now: kernel.clock().now(),
        now_secs: crate::sessions::now_secs(),
        plugins,
        library,
        torrents,
        installs,
        sessions,
        prefs,
        notices,
        launcher_update,
        vpn,
        downloads,
        state,
        detail_game,
        files,
        theme_options,
    }
}

fn files(kernel: &Kernel, library: &LibrarySnapshot, torrents: &TorrentsSnapshot, detail: Option<&GameKey>) -> Files {
    let folder_there = |key: &GameKey| {
        let folder = library.detail(key).and_then(|d| d.details.folder.as_ref());
        folder.is_some_and(|f| crate::app::resolve_game_folder(kernel, key, f).is_some())
    };
    let games = library.library().map(|e| &e.key).chain(detail);
    let host = kernel.host();
    let mut roots = HashMap::new();
    for manifest in kernel.order().iter().filter_map(|id| kernel.manifest(id).ok()) {
        for grant in &manifest.permissions.filesystem {
            roots
                .entry(grant.folder.clone())
                .or_insert_with(|| crate::folders::roots(kernel.config(), host.home.as_deref(), host.platform, grant));
        }
    }
    let install_folder = |emulator: &PluginId| {
        let (_, path) = crate::release_downloads::install_folder(kernel, emulator, None).ok()?;
        Some((emulator.clone(), path))
    };
    Files {
        game_folders: games.filter(|key| folder_there(key)).cloned().collect(),
        has_log: detail.is_some_and(|key| crate::sessions::log_path(&host.data_dir, key).is_file()),
        install_folders: torrents.jobs.values().filter_map(|j| install_folder(&j.emulator)).collect(),
        roots,
    }
}
