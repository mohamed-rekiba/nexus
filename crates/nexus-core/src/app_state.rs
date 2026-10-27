//! The state no subsystem owns: where the player is and what they look at (the navigation
//! stack, the selections, the Library's and the Catalog's grids), the questions the player
//! answers (the install prompt, "How far did it get?"), each emulator's builds and the Settings
//! screen's panels as the plugins last gave them, the machine's state, the stores' installs,
//! and the theme on screen.
//!
//! The core's flows write it under one lock, and the view reads it. The core takes it only
//! while it holds no subsystem's lock.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nexus_api::{GameKey, PluginId};
use serde_json::Value;

use crate::app::{Grid, HIDDEN_FILTER, INGAME_FILTER, INSTALLED_FILTER, Route};

/// A first play waiting for the player to agree to install its emulator.
#[derive(Clone, Debug)]
pub(crate) struct InstallPrompt {
    pub game: GameKey,
    pub emulator: String,
    pub version: String,
    pub title: String,
    pub size_bytes: Option<u64>,
}

/// An emulator's builds and blocks, as its plugin last gave them.
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
    pub fn catalog() -> Self {
        Self { sort: "added".into(), ..Self::default() }
    }
}

/// The state no subsystem owns (see the module's text). One lock holds it all.
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

impl AppState {
    /// Home, with nothing selected, and `theme` on screen.
    pub fn new(theme: Option<PluginId>) -> Self {
        Self {
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
        }
    }
}

impl LibraryView {
    /// A system, or `INGAME_FILTER`, `INSTALLED_FILTER` or `HIDDEN_FILTER` to turn "In-game",
    /// "Installed" or "Hidden" on or off; `None` clears them all.
    pub fn set_system(&mut self, system: Option<String>) {
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
