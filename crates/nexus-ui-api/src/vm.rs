//! Headless view models. The core builds them from app state; themes bind to them.
//!
//! They hold display-ready data only (text, numbers, flags, image references), so a theme
//! needs no logic beyond layout to show them.

use nexus_api::{ActionStyle, ConfigScope, PluginId, RelPath, SlotId, Tone};
use serde::{Deserialize, Serialize};

/// The screens every theme can show. The default theme has all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenId {
    /// Home and Library.
    Home,
    GameDetail,
    /// Catalog and Search.
    Catalog,
    Downloads,
    Settings,
    /// The emulator plugins, to open one.
    Emulators,
    /// The theme plugins, to open one.
    Themes,
    /// One plugin: on or off, its emulator builds, its settings and folders.
    Plugin,
    /// One game's own settings, from its emulator.
    GameSettings,
    /// The Launch / Running overlay.
    Running,
}

impl ScreenId {
    pub const ALL: [ScreenId; 10] = [
        Self::Home,
        Self::GameDetail,
        Self::Catalog,
        Self::Downloads,
        Self::Settings,
        Self::Emulators,
        Self::Themes,
        Self::Plugin,
        Self::GameSettings,
        Self::Running,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::GameDetail => "game-detail",
            Self::Catalog => "catalog",
            Self::Downloads => "downloads",
            Self::Settings => "settings",
            Self::Emulators => "emulators",
            Self::Themes => "themes",
            Self::Plugin => "plugin",
            Self::GameSettings => "game-settings",
            Self::Running => "running",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == s)
    }
}

/// An image a theme shows. The UI host turns it into pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageRef {
    /// Fetched within the plugin's network allowlist.
    Url { plugin: PluginId, url: String },
    /// A file in a plugin's `assets` folder.
    PluginAsset { plugin: PluginId, path: RelPath },
    /// A file in a host folder, read with the plugin's folder grants.
    Folder { plugin: PluginId, path: nexus_api::HostPath },
}

/// One slot item, ready to show. `label` is `<plugin-id>:<item-id>`: themes set it as the
/// rendered item's accessible label.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotVm {
    pub slot: SlotId,
    pub plugin: PluginId,
    pub id: String,
    pub label: String,
    pub kind: SlotKind,
    pub text: String,
    /// Badges: the text for small places, such as a card. The same as `text` when the plugin
    /// gives none.
    pub short: String,
    pub tone: Tone,
    pub style: ActionStyle,
    pub icon: Option<ImageRef>,
    pub rows: Vec<RowVm>,
    /// For actions: what the UI sends back when the player picks it.
    pub action: Option<String>,
    pub args: serde_json::Value,
    /// For settings panels.
    pub scope: Option<ConfigScope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Badge,
    Action,
    Section,
    SettingsPanel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowVm {
    pub label: String,
    pub value: String,
}

/// A game's system, as the plugin of that system describes it (`[system]` in its manifest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSystemVm {
    /// Its full name, "PlayStation 5"; the system's code when no plugin names it.
    pub name: String,
    /// Its color, red, green and blue; `None` draws it in the theme's neutral look.
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameCardVm {
    /// The game's [`nexus_api::GameKey`] as text.
    pub key: String,
    pub title: String,
    pub platform: String,
    pub system: GameSystemVm,
    pub cover: Option<ImageRef>,
    pub selected: bool,
    pub badges: Vec<SlotVm>,
    pub release_year: Option<u16>,
    /// Out of 5, from a store's catalog.
    pub rating: Option<f32>,
    /// Once the game's details are in.
    pub size_bytes: Option<u64>,
    /// Days since the game last ran; `None` when it never ran.
    pub last_played_days: Option<u64>,
    /// How many releases a catalog offers of the game; 0 for an installed game.
    pub releases: usize,
    /// For a game a catalog offers, how easy its best release is to download: `good`, `low`,
    /// `none` or `unknown` (see [`ReleaseVm::availability`]). Empty for an installed game.
    pub availability: String,
    /// The player hid this Catalog game; it shows only under the Catalog's "Hidden" filter.
    pub hidden: bool,
    /// What can be done with the game: the same for every game and on every screen.
    pub can: GameActionsVm,
}

/// What can be done with a game, worked out once for any game, a Library one or a Catalog
/// offer, of any plugin: every card and the Game Hub draw it the same way.
///
/// - Installed: the main action is Play.
/// - Not installed: the main action is getting it, a download or the store's install.
/// - Uninstall is never the main action: it's in the game's menu.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameActionsVm {
    /// The main action: `play`, `get`, or empty for none.
    pub primary: String,
    /// It is installed, and its plugin can start it (for a Catalog offer, the installed game it
    /// is: Play starts that one).
    pub play: bool,
    /// How to get it when it isn't installed: "Download" (a torrent: pick a release on its
    /// Game Hub) or "Install in Steam". Empty when there is nothing to get.
    pub get: String,
    /// For a store's install, the release whose download asks the store, and the store's
    /// name; empty for a torrent.
    pub get_release: String,
    pub get_store: String,
    /// The key of the installed game Uninstall removes (for a Catalog offer, the installed game
    /// it is); empty when it can't be uninstalled.
    pub uninstall: String,
    /// The store that uninstalls it ("Steam"), asking first in its own way; empty when Nexus
    /// deletes its folder.
    pub uninstall_store: String,
}

/// One Library filter: a system or a genre, with how many games it has.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterVm {
    /// The value to filter by, for example `PS5` or `Shooter`; empty for "All".
    pub id: String,
    pub label: String,
    pub count: usize,
    pub on: bool,
}

/// The Library: every game, searched, filtered by system and genre, and sorted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryVm {
    /// The shown pages of games: more come with [`crate::LibraryVm::total`] and a "more".
    pub games: Vec<GameCardVm>,
    /// How many games match, on every page.
    pub total: usize,
    /// How many games there are before the search and the filters.
    pub all: usize,
    /// Index into `games`.
    pub selected: Option<usize>,
    /// "All", then each system.
    pub systems: Vec<FilterVm>,
    /// "All genres", then each genre, most games first.
    pub genres: Vec<FilterVm>,
    /// `title`, `played`, `size`, `rating`, `release`, `compat` or `added`.
    pub sort: String,
    pub query: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeVm {
    pub games: Vec<GameCardVm>,
    /// Index into `games`.
    pub selected: Option<usize>,
    pub loading: bool,
    /// `games` split by platform, in the order each platform first appears.
    pub groups: Vec<GroupVm>,
    pub library: LibraryVm,
}

/// The library's games on one platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupVm {
    pub platform: String,
    pub games: Vec<GameCardVm>,
    /// Index into `games`, when the selected game is on this platform.
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompatVm {
    /// For example `playable`.
    pub level: String,
    /// For example `Playable`.
    pub label: String,
    pub notes: String,
    /// When the result was last tested, as `YYYY-MM-DD`; empty when no one says.
    pub tested: String,
}

/// How long [`GameDetailVm::summary`] is at most: one line on Home.
pub const SUMMARY_CHARS: usize = 75;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameDetailVm {
    pub key: String,
    pub title: String,
    pub platform: String,
    pub system: GameSystemVm,
    pub description: String,
    /// `description` on one line, at most [`SUMMARY_CHARS`] characters, for Home.
    pub summary: String,
    pub developer: String,
    pub genres: Vec<String>,
    pub cover: Option<ImageRef>,
    pub background: Option<ImageRef>,
    pub compat: Option<CompatVm>,
    pub badges: Vec<SlotVm>,
    pub actions: Vec<SlotVm>,
    pub sections: Vec<SlotVm>,
    /// What can be done with the game: as on its card.
    pub can: GameActionsVm,
    /// The game's own settings from its emulator: one panel, or none.
    pub settings: Vec<SettingsPanelVm>,
    /// What Play does first, for example "Play installs KytyPS5 first". Empty for nothing.
    pub launch_hint: String,
    pub running: bool,
    /// The game's size on disk.
    pub size_bytes: Option<u64>,
    /// The game's name as a picture, from a store's catalog.
    pub logo: Option<ImageRef>,
    pub rating: Option<nexus_api::Rating>,
    /// `YYYY-MM-DD`, or empty.
    pub release: String,
    /// For example `PEGI 12`, from a store's catalog.
    pub age_rating: String,
    pub screenshots: Vec<ImageRef>,
    /// The game's page in a store, or empty.
    pub store_url: String,
    /// For a Windows game packed with Wine that the catalog found on Steam: ProtonDB's tier,
    /// such as `gold` (see [`nexus_api::SteamInfo::protondb`]). Empty for none.
    pub protondb: String,
    /// The game's ProtonDB page, for the same games; empty for none.
    pub protondb_url: String,
    /// How the game runs on this PC, from its emulator's needs: `None` before the check, or
    /// when the system can't say. For a game a catalog offers that doesn't fit in the
    /// download folder, `no` with "Not enough space" and the free space ("10.0 GB free").
    pub this_pc: Option<VerdictVm>,
    /// Why the game won't start on this PC, such as "This PC can't run KytyPS5 games: No
    /// Vulkan 1.3"; empty when it can.
    pub pc_warning: String,
    /// The player's own result: `ingame`, `menus`, `boots` or `nothing`; empty for none.
    pub your_result: String,
    /// The game has a folder to open, and a log of its last run.
    pub has_folder: bool,
    pub has_log: bool,
    /// The player hid this game from the Catalog: it shows only under the "Hidden" filter.
    pub hidden: bool,
    /// The community compatibility list's page; empty for none.
    pub compat_page: String,
    /// The game's trailer, an `https://` video address; empty for none.
    pub trailer_url: String,
    /// Short facts from the plugin, such as the region; see [`nexus_api::GameFact`].
    pub facts: Vec<nexus_api::GameFact>,
    /// Seconds played, in all.
    pub played_secs: u64,
    /// Whole days, of 24 hours, since the game last closed. `None` when it never ran.
    pub last_played_days: Option<u64>,
    /// The emulator's shader cache for this game, when it has the `shaders` capability.
    pub shader_cache: Option<ShaderCacheVm>,
    /// For a game a catalog offers: its releases by version, highest first. Empty for an
    /// installed game.
    pub releases: Vec<ReleaseVm>,
    /// For a game a catalog offers: the name of the program that runs it, `KytyPS5`.
    pub emulator: String,
    /// For a game a catalog offers: where its downloads go; empty when the emulator can't
    /// install downloads.
    pub download_folder: String,
    /// For a game a catalog offers, how easy its best release is to download (see
    /// [`ReleaseVm::availability`]); empty for an installed game.
    pub availability: String,
    /// Why the download the player last asked for here didn't start; empty when it did.
    pub download_error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShaderCacheVm {
    pub entries: u64,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogVm {
    /// A plugin that gives release catalogs is turned on. Without one there is no Catalog:
    /// a theme hides the ways to it.
    pub available: bool,
    /// The games the catalogs offer, as the Library shows its games, with a `New` or
    /// `Installed` badge. Current while the Catalog is the screen; on another screen, the
    /// grid as it last was.
    pub games: LibraryVm,
    /// Which copy of each catalog is in use, one line each, for example
    /// "KytyPS5: fetched from xyz.dev · updated 2026-10-18 · 120 releases".
    pub sources: Vec<String>,
    /// How many releases the catalogs list, of every game.
    pub releases: usize,
    /// When the list was updated, `YYYY-MM-DD`: the newest count of a release's seeders, else
    /// the newest date a catalog gives. Empty when neither says.
    pub updated: String,
    /// The catalog plugins have not answered yet.
    pub loading: bool,
    /// The catalog plugins' buttons for the Catalog screen (`catalog.action`), such as a
    /// refresh, and their badges, which say what the buttons did.
    pub actions: Vec<SlotVm>,
    pub status: Vec<SlotVm>,
}

/// One release of a game a catalog offers: a version, for a region, to download.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReleaseVm {
    /// The torrent's info hash.
    pub id: String,
    /// The name of the store that installs it ("Steam"), for a release its store installs;
    /// empty for a torrent. Its download asks the store, with no download sheet.
    pub store: String,
    pub version: String,
    pub region: String,
    pub size_bytes: Option<u64>,
    /// `YYYY-MM-DD`, or empty.
    pub added: String,
    /// Added in the last 14 days.
    pub new: bool,
    pub description: String,
    /// A PC release's system: `Linux`, `Windows` or `macOS`; empty when the catalog doesn't say.
    pub system: String,
    /// Why this computer can't start it, such as "This is a Linux release: it can't run on
    /// Windows"; empty when it can.
    pub not_here: String,
    /// Why the release may not install, such as "Retail PS5 packages can't be installed; pick
    /// a folder, an archive or an exFAT image"; empty for none.
    pub warning: String,
    /// Why it doesn't fit in the download folder, such as "Not enough free space in the
    /// download folder: 50.0 GB needed, 10.0 GB free"; empty when it fits, when the disk
    /// doesn't say, or when it has a download.
    pub no_room: String,
    /// Its download's state (see [`TorrentVm::state`]); empty when it has none.
    pub download: String,
    pub download_progress: f32,
    /// Its place among the game's releases, numbered by version from the lowest: "Version 3
    /// of 3 (newest)" for the highest version.
    pub version_label: String,
    /// The release the game's card shows, and the one to offer first: the version the player
    /// has, then one with art, then the highest version, then the one most people share.
    pub best: bool,
    /// How many people share it, and how many download it, as the catalog counted them;
    /// `None` when the catalog has no count. They are not live counts.
    pub seeders: Option<u64>,
    pub leechers: Option<u64>,
    /// When the catalog counted them, an RFC 3339 time; empty when it doesn't say.
    pub counted_at: String,
    /// How easy it is to download, from `seeders`: `good` (3 or more), `low` (1 or 2), `none`
    /// (0) or `unknown` (no count).
    pub availability: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadVm {
    /// The job number.
    pub id: u64,
    pub plugin: PluginId,
    pub label: String,
    pub received: u64,
    pub total: Option<u64>,
    /// 0.0 to 1.0. 0 when the size is unknown.
    pub progress: f32,
    /// `queued`, `downloading`, `verifying`, `paused`, `done`, `failed` or `cancelled`.
    pub state: String,
    pub error: String,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadsVm {
    pub jobs: Vec<DownloadVm>,
    /// The catalogs' downloads, newest first.
    pub torrents: Vec<TorrentVm>,
    /// "Keep sharing finished downloads".
    pub seeding: bool,
    /// How many of the catalogs' downloads read their files, check or download now.
    pub downloading: usize,
    /// How many of the catalogs' downloads share their finished files now.
    pub sharing: usize,
}

/// One download of a release a catalog offers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TorrentVm {
    /// The torrent's info hash: what [`nexus_api`]'s host commands name it by.
    pub id: String,
    pub title: String,
    pub version: String,
    pub region: String,
    /// `resolving`, `ready` (the files are listed: the player starts it), `checking`,
    /// `downloading`, `paused`, `complete`, `failed` or `cancelled`.
    pub state: String,
    /// 0.0 to 1.0.
    pub progress: f32,
    pub done: u64,
    pub total: u64,
    /// Bytes a second.
    pub download_rate: u64,
    pub upload_rate: u64,
    /// How long the rest takes at `download_rate`, in seconds, while it downloads; 0 when no
    /// bytes arrive.
    pub seconds_left: u64,
    pub peers: u32,
    pub seeding: bool,
    /// Checking a finished download's files before it seeds again.
    pub verifying: bool,
    /// Paused by "Download only through a VPN": it starts again by itself once the VPN is
    /// connected.
    pub waiting_vpn: bool,
    pub error: String,
    /// The first files, as "path · size", and how many there are.
    pub files: Vec<String>,
    pub file_count: usize,
    /// Where the files are.
    pub folder: String,
    /// What the player can do now, in order: `start`, `pause`, `cancel`, `remove`,
    /// `stop-seeding`, `install`, `play`, `remove-unpacked`. While its archives unpack,
    /// `cancel` stops the unpacking.
    pub actions: Vec<String>,
    /// Its install: `installing`, `done`, `failed` or `cancelled`; empty for none yet.
    pub install: String,
    pub install_progress: f32,
    /// Its archives: `waiting`, `unpacking`, `unpacked`, `failed` or `cancelled`; empty when
    /// it holds none.
    pub unpack: String,
    /// 0.0 to 1.0.
    pub unpack_progress: f32,
    /// The installed game, for `play`; empty until it is in the library.
    pub game: String,
    /// What the emulator installs from, for the install sheet.
    pub install_note: String,
    /// The game folder the install sheet offers, with `~` for the home folder; empty when the
    /// emulator has no game folder.
    pub install_folder: String,
    /// Why the last Install didn't start; empty when it did.
    pub install_error: String,
    /// The store doing it, for an install a store does ("Steam"): its id is `store:<plugin>:
    /// <release>`, its state `queued`, `downloading` or `paused` (or `install` `installing`), and
    /// its one action `open-store`, the store's own Downloads page. Empty for a torrent.
    pub store: String,
}

/// One settings field, flattened from the plugin's JSON Schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldVm {
    pub key: String,
    pub label: String,
    pub help: String,
    pub kind: FieldKind,
    /// The current value as text.
    pub value: String,
    /// For `Choice`.
    pub choices: Vec<String>,
    /// What to call each choice, in the order of `choices`: "" when the plugin names none.
    pub choice_labels: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Toggle,
    Number,
    Text,
    Choice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsPanelVm {
    /// The slot item this panel came from.
    pub slot: SlotVm,
    pub fields: Vec<FieldVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeChoiceVm {
    pub id: PluginId,
    pub name: String,
    pub current: bool,
}

/// The launcher's own update, for Settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LauncherUpdateVm {
    /// off | idle | checking | current | available | downloading | ready | installed | system |
    /// failed. `installed`: the package manager put it in, a restart starts it. `system`: it
    /// comes with Nexus OS's next update.
    pub state: String,
    /// The running version: "0.1.0".
    pub current: String,
    /// The newer one, when there is one.
    pub version: String,
    /// What to tell the player: "Nexus 0.2.0 is ready · restart to update".
    pub text: String,
    /// How an update goes in: swap | installer (both at the next start) | package (the
    /// system's package manager, now) | system (Nexus OS's update).
    pub method: String,
}

/// This PC: its processor, memory and graphics, and a verdict per emulator.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareVm {
    /// The check has run.
    pub known: bool,
    /// "Apple M4 Max · 14 cores".
    pub cpu: String,
    /// "36 GB".
    pub memory: String,
    /// One line per card: "NVIDIA GeForce RTX 3090 · 24 GB · Vulkan 1.3".
    pub graphics: Vec<String>,
    pub verdicts: Vec<VerdictVm>,
}

/// How one emulator's games run here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VerdictVm {
    pub plugin: String,
    /// The plugin's name, which names its console: "PS5".
    pub emulator: String,
    /// good | slow | no | unknown
    pub level: String,
    /// "Should run well", "May run slowly", "Won't start" or "Can't tell yet".
    pub headline: String,
    /// Why, for all but good: "Integrated GPU".
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsVm {
    /// The launcher's own update.
    pub launcher_update: LauncherUpdateVm,
    /// This PC, and how each emulator's games run on it.
    pub hardware: HardwareVm,
    pub panels: Vec<SettingsPanelVm>,
    pub themes: Vec<ThemeChoiceVm>,
    /// The current theme's options.
    pub theme_options: Vec<FieldVm>,
    /// Named host folders that plugins ask for, and where the player put them.
    pub folders: Vec<FolderVm>,
    /// Theme sounds play.
    pub sounds: bool,
    /// The launcher's window fills the screen.
    pub fullscreen: bool,
    /// The launcher comes back to the front when a game closes.
    pub return_on_exit: bool,
    /// Emulator updates install by themselves, after the emulator's game closes.
    pub auto_update: bool,
    /// The size of everything the theme draws, from 0.8 to 1.4.
    pub scale: f32,
    /// This build can start when the player logs in: it is installed.
    pub can_start_at_login: bool,
    /// The launcher starts when the player logs in.
    pub start_at_login: bool,
    /// Why the system refused the last change to it. Empty when it didn't.
    pub start_at_login_error: String,
    /// "Download only through a VPN", and the VPNs.
    pub vpn: VpnSettingsVm,
}

/// "Download only through a VPN", and the VPNs this computer has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VpnSettingsVm {
    /// Why downloads can't go only through a VPN here, such as "Not available on Windows".
    /// Empty when they can.
    pub problem: String,
    /// Downloads go only through the VPN.
    pub only: bool,
    /// The system tells VPNs from other networks. When it can't, `vpns` lists every network
    /// with an address, and the player picks the one the VPN uses.
    pub finds: bool,
    /// The system can add a VPN from a provider's file.
    pub can_import: bool,
    /// The VPN the player picked. Empty: the first connected one, where the system finds VPNs.
    pub chosen: String,
    /// The network interface that downloads go through now. Empty while they wait, and while
    /// the setting is off.
    pub route: String,
    pub vpns: Vec<VpnVm>,
    /// Why the last connect, disconnect or import failed. Empty when it didn't.
    pub error: String,
}

/// One VPN, or one network where the system can't tell VPNs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VpnVm {
    pub name: String,
    /// "WireGuard", "OpenVPN", "VPN" or "Network".
    pub kind: String,
    pub connected: bool,
    /// The system can connect and disconnect it.
    pub switchable: bool,
}

/// One named host folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderVm {
    pub name: String,
    /// What Settings calls it: the first plugin's title for it. Empty when none names it.
    pub title: String,
    /// The first plugin's line about it. Empty when none.
    pub help: String,
    /// The folders it means now: the player's, or else the default.
    pub paths: Vec<String>,
    /// `paths` joined with "; ", for a text field.
    pub text: String,
    /// What to show: `text`, or the default when it is empty, with the home folder as `~`.
    pub shown: String,
    /// The plugins' default, if any. Empty when there is none.
    pub default: String,
    /// The names of the plugins that ask for it.
    pub requested_by: Vec<String>,
    /// Some plugin may write to it.
    pub write: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmulatorVm {
    pub plugin: PluginId,
    /// The program's name, the manifest's `emulator_name()`: `KytyPS5`.
    pub name: String,
    /// `loaded`, `active` or `failed`.
    pub status: String,
    /// It installs builds of its own. An emulator that only starts games (PC) has none.
    pub has_builds: bool,
    pub installed: Vec<String>,
    pub available: Vec<String>,
    /// A newer version than every installed one, if there is one.
    pub update: Option<String>,
    /// An install, update or uninstall is running.
    pub busy: bool,
    pub panels: Vec<SlotVm>,
    /// Controller profiles the emulator has, as `name (device)`.
    pub controllers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRowVm {
    pub id: PluginId,
    pub name: String,
    /// `emulator`, `service` (code that launches no games, such as a store's details) or
    /// `theme`.
    pub kind: String,
    pub version: String,
    pub status: String,
    /// The player has it on.
    pub enabled: bool,
    /// A warning about it, or empty: for one made for other systems, which systems.
    pub note: String,
    /// An emulator's build in use, or empty when none is installed. `version` is the plugin's.
    pub build: String,
    /// `build` with the program's name, once: `shadPS4 v.0.20.0`, `KytyPS5-2026-10-21-e2dd48a`
    /// (the build names it already). Empty when none is installed.
    pub build_label: String,
    /// It installs builds of its own. An emulator that only starts games (PC) has none.
    pub has_builds: bool,
}

/// One plugin's page. A plugin that is turned off has only its row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginPageVm {
    pub plugin: PluginRowVm,
    /// Its builds, when it is an emulator and turned on.
    pub emulator: Option<EmulatorVm>,
    /// Its settings panels.
    pub panels: Vec<SettingsPanelVm>,
    /// Its blocks in the `emulator.panel` slot: sections and actions. An emulator's are also
    /// in `emulator`.
    pub sections: Vec<SlotVm>,
    /// The folders it asks for, but not the game folders: those are in Settings.
    pub folders: Vec<FolderVm>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmulatorsVm {
    pub emulators: Vec<EmulatorVm>,
    pub plugins: Vec<PluginRowVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunningVm {
    pub session: u64,
    pub key: String,
    pub title: String,
    /// The name of the emulator that runs it, for example `KytyPS5`.
    pub emulator: String,
    /// The name of the page with the emulator's keys: its plugin's name, `PS5`.
    pub page: String,
    /// `starting` until the game's window shows, then `running`, then `stopping`.
    pub state: String,
    /// Seconds since the game started, when the view was made.
    pub elapsed_secs: u64,
    /// The game's window is in front, so the gamepad is the game's.
    pub in_front: bool,
    /// The emulator's keyboard keys for the console's buttons, from its manifest's `[[keymap]]`.
    #[serde(default)]
    pub keymap: Vec<nexus_api::KeyBinding>,
}

/// The start of a run: loading, and on the first run the Welcome screen with its checklist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BootVm {
    /// The library and the first game's details are in.
    pub ready: bool,
    /// The player hasn't been welcomed yet: the theme shows Welcome until `finish-welcome`.
    pub welcome: bool,
    /// 0 to 1.
    pub progress: f32,
    /// "Loading..." or "All set · 5 installed games found".
    pub status: String,
    pub steps: Vec<BootStepVm>,
    /// Covers of the library, for a wall behind the screen.
    pub covers: Vec<ImageRef>,
}

/// One line of the Welcome checklist: "KytyPS5 emulator · Latest build".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootStepVm {
    pub label: String,
    pub detail: String,
    /// `waiting`, `working`, `done` or `failed`.
    pub state: String,
}

/// Everything a theme can show, rebuilt from app state after every change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppView {
    pub screen: ScreenId,
    /// The game a Game Detail screen shows.
    pub game: Option<String>,
    pub can_go_back: bool,
    pub home: HomeVm,
    pub detail: Option<GameDetailVm>,
    pub catalog: CatalogVm,
    pub downloads: DownloadsVm,
    pub settings: SettingsVm,
    pub emulators: EmulatorsVm,
    /// The plugin the Plugin screen shows.
    pub plugin_page: Option<PluginPageVm>,
    pub running: Option<RunningVm>,
    /// Short messages for the player, newest last: the text of `notice_cards`.
    pub notices: Vec<String>,
    /// The notices the theme shows, newest last. The core keeps at most `MAX_NOTICES`.
    pub notice_cards: Vec<NoticeVm>,
    /// A game was started whose emulator isn't installed: the player is asked first.
    pub install_prompt: Option<InstallPromptVm>,
    pub boot: BootVm,
    /// Work going on in the background, for example "Downloading game art 3/40".
    pub task: Option<String>,
    /// The art of the game the newest notice is about.
    pub notice_art: Option<ImageRef>,
    /// What clicking the newest notice opens, for example "Open Downloads". Empty for nothing.
    pub notice_opens: String,
    /// "How far did it get?", for a game that just closed: its title.
    pub result_prompt: Option<String>,
    /// The machine's settings: network, sound, Bluetooth, battery, power, Nexus OS.
    pub system: SystemVm,
}

/// The machine's settings, from the system plugins. A part whose `*_available` is false shows
/// its `*_reason` instead of its controls.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemVm {
    /// Nexus is the whole screen (the Nexus session, Nexus OS): the theme shows the network,
    /// sound and Bluetooth controls and Settings → System then. A desktop has its own.
    pub whole_screen: bool,
    pub nexus_os: bool,
    /// Nexus OS started from its USB stick, which can install it.
    pub stick: bool,

    pub network_available: bool,
    pub network_reason: String,
    /// none | wired | wifi
    pub connection: String,
    /// The Wi-Fi network's or the wired connection's name.
    pub network_name: String,
    /// The Wi-Fi signal in bars, 0 to 4 (0 too when not on Wi-Fi).
    pub signal_bars: i32,
    pub wifi: Vec<WifiVm>,

    pub audio_available: bool,
    pub audio_reason: String,
    /// 0 to 100.
    pub volume: i32,
    pub muted: bool,
    pub outputs: Vec<AudioOutputVm>,

    pub bluetooth_available: bool,
    pub bluetooth_reason: String,
    pub bluetooth_on: bool,
    /// How many devices are connected.
    pub connected_devices: i32,
    pub devices: Vec<BluetoothDeviceVm>,

    /// Restart and shut down.
    pub power_available: bool,
    pub power_reason: String,
    pub battery_shown: bool,
    pub battery_percent: i32,
    pub battery_charging: bool,

    /// Nexus OS's update: "" (not Nexus OS) | current | checking | staged | failed.
    pub os_update: String,
    /// What to tell the player: "Restart to update to 2026.10.24".
    pub os_update_text: String,

    /// The disks Nexus OS can be installed on (the USB stick only).
    pub disks: Vec<InstallDiskVm>,
    /// Long work going on, and work that just ended.
    pub jobs: Vec<SystemJobVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WifiVm {
    pub ssid: String,
    /// 0 to 4.
    pub bars: i32,
    pub secured: bool,
    pub saved: bool,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioOutputVm {
    pub name: String,
    pub description: String,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BluetoothDeviceVm {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstallDiskVm {
    pub path: String,
    /// "512 GB".
    pub size: String,
    pub windows: bool,
    pub usb: bool,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemJobVm {
    /// Names it for cancelling.
    pub id: i32,
    /// join | scan | pair | install
    pub kind: String,
    /// The network's name, the device's address, the disk.
    pub target: String,
    /// running | done | failed
    pub state: String,
    /// 0 to 100, or -1 when not known.
    pub percent: i32,
    pub text: String,
    /// A finished install: Secure Boot's key screen comes at the restart.
    pub mok: bool,
    /// A finished install: the PC gets NVIDIA's driver on the installed system.
    pub nvidia_later: bool,
}

/// The most notices the core keeps: a new one drops the oldest.
pub const MAX_NOTICES: usize = 3;

/// What a notice tells the player: news (`Info`, the default), something that finished
/// (`Success`) or a problem (`Error`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeKind {
    #[default]
    Info,
    Success,
    Error,
}

impl NoticeKind {
    /// "info", "success" or "error", as the theme contract names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

/// One notice: "Download complete: Astro Bot. Install it from Downloads."
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoticeVm {
    /// Names the notice for `Command::DismissNotice` and `Command::OpenNotice`.
    pub id: u64,
    pub text: String,
    pub kind: NoticeKind,
    /// The art of the game the notice is about.
    pub art: Option<ImageRef>,
    /// What clicking the notice opens, for example "Open Downloads". Empty for nothing.
    pub opens: String,
}

/// "Install KytyPS5 to play Astro Bot?": what the player is asked before a first play
/// downloads an emulator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstallPromptVm {
    pub plugin: PluginId,
    /// The name of the program that downloads: `KytyPS5`.
    pub emulator: String,
    pub version: String,
    /// The game's title.
    pub game: String,
    /// The download's size, when the plan knows it.
    pub size_bytes: Option<u64>,
}
