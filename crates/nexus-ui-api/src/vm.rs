//! Headless view models. The core builds them from app state; themes bind to them.
//!
//! They hold display-ready data only (text, numbers, flags, image references), so a theme
//! needs no logic beyond layout to show them.
//!
//! Their `serde` form is the theme contract: the UI host turns [`AppView`] into the `Vm`
//! properties of `types.slint`, each field kebab-cased (`can_go_back` is `can-go-back`). A
//! field the contract doesn't have is `#[serde(skip)]`: the core's tests read it. The
//! [`mark`]ed fields take a form of their own there, such as an image.

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
    #[serde(serialize_with = "mark::thumb")]
    pub icon: Option<ImageRef>,
    pub rows: Vec<RowVm>,
    /// For actions: what the UI sends back when the player picks it.
    pub action: Option<String>,
    /// As JSON text, "" for none: the theme passes it back unchanged.
    #[serde(serialize_with = "mark::json_text")]
    pub args: serde_json::Value,
    /// For settings panels; as JSON text, "" for none.
    #[serde(serialize_with = "mark::json_text")]
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
    #[serde(rename = "system_name")]
    pub name: String,
    /// Its color, red, green and blue; `None` draws it in the theme's neutral look.
    #[serde(rename = "system_color", serialize_with = "mark::color")]
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameCardVm {
    /// The game's [`nexus_api::GameKey`] as text.
    pub key: String,
    pub title: String,
    pub platform: String,
    #[serde(flatten)]
    pub system: GameSystemVm,
    #[serde(serialize_with = "mark::cover")]
    pub cover: Option<ImageRef>,
    pub selected: bool,
    pub badges: Vec<SlotVm>,
    pub release_year: Option<u16>,
    /// Out of 5, from a store's catalog.
    pub rating: Option<f32>,
    /// Once the game's details are in.
    #[serde(skip)]
    pub size_bytes: Option<u64>,
    /// `size_bytes` as text, "7.6 GB"; empty when it isn't known.
    pub size: String,
    /// Days since the game last ran; `None` when it never ran.
    #[serde(skip)]
    pub last_played_days: Option<u64>,
    /// `last_played_days` as text: "Today", "Yesterday" or "3 days ago"; empty when it never ran.
    pub last_played: String,
    /// How many releases a catalog offers of the game; 0 for an installed game.
    pub releases: usize,
    /// For a game a catalog offers, how easy its best release is to download: `good`, `low`,
    /// `none` or `unknown` (see [`ReleaseVm::availability`]). Empty for an installed game.
    pub availability: String,
    /// `availability` in words: "Good", "Low", "None", "Unknown"; empty for none.
    pub availability_text: String,
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
    #[serde(serialize_with = "mark::cards")]
    pub games: Vec<GameCardVm>,
    /// How many games match, on every page.
    pub total: usize,
    /// How many games there are before the search and the filters.
    pub all: usize,
    /// Index into `games`.
    #[serde(serialize_with = "mark::index")]
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
    #[serde(serialize_with = "mark::cards")]
    pub games: Vec<GameCardVm>,
    /// Index into `games`.
    #[serde(serialize_with = "mark::index")]
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
    #[serde(serialize_with = "mark::index")]
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
    #[serde(skip)]
    pub tested: String,
    /// `tested` as text, "Sep 1, 2026".
    #[serde(rename = "tested")]
    pub tested_text: String,
}

/// How long [`GameDetailVm::summary`] is at most: one line on Home.
pub const SUMMARY_CHARS: usize = 75;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameDetailVm {
    pub key: String,
    pub title: String,
    pub platform: String,
    #[serde(flatten)]
    pub system: GameSystemVm,
    pub description: String,
    /// `description` on one line, at most [`SUMMARY_CHARS`] characters, for Home.
    pub summary: String,
    pub developer: String,
    pub genres: Vec<String>,
    #[serde(serialize_with = "mark::cover_picture")]
    pub cover: Option<ImageRef>,
    #[serde(serialize_with = "mark::picture")]
    pub background: Option<ImageRef>,
    /// With `has-compat`.
    #[serde(serialize_with = "mark::has")]
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
    #[serde(skip)]
    pub size_bytes: Option<u64>,
    /// `size_bytes` as text, "7.6 GB"; empty when it isn't known.
    pub size: String,
    /// The game's name as a picture, from a store's catalog. With `has-logo`.
    #[serde(serialize_with = "mark::has_picture")]
    pub logo: Option<ImageRef>,
    #[serde(skip)]
    pub rating: Option<nexus_api::Rating>,
    /// `rating`: its score out of 5, how many players gave it, and both as text, "4.4 · 3.2K
    /// ratings". 0, 0 and empty for none.
    #[serde(rename = "rating")]
    pub rating_score: f32,
    pub ratings: u64,
    pub rating_text: String,
    /// `YYYY-MM-DD`, or empty.
    #[serde(skip)]
    pub release: String,
    /// `release` as text, "Mar 12, 2024", and its year, "2024".
    #[serde(rename = "release")]
    pub release_text: String,
    pub year: String,
    /// For example `PEGI 12`, from a store's catalog.
    pub age_rating: String,
    #[serde(serialize_with = "mark::pictures")]
    pub screenshots: Vec<ImageRef>,
    /// The game's page in a store, or empty.
    pub store_url: String,
    /// For a Windows game packed with Wine that the catalog found on Steam: ProtonDB's tier,
    /// such as `gold` (a release's [`nexus_api::FactKind::Compatibility`] fact). Empty for none.
    pub protondb: String,
    /// The game's ProtonDB page, for the same games; empty for none.
    pub protondb_url: String,
    /// How the game runs on this PC, from its emulator's needs: `None` before the check, or
    /// when the system can't say. For a game a catalog offers that doesn't fit in the
    /// download folder, `no` with "Not enough space" and the free space ("10.0 GB free").
    /// With `has-this-pc`.
    #[serde(serialize_with = "mark::has")]
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
    /// The values of the facts `region`, `age` and `version`; empty when the plugin gives none.
    pub region: String,
    pub age: String,
    pub version: String,
    /// Short facts from the plugin, such as the region; see [`nexus_api::GameFact`].
    pub facts: Vec<nexus_api::GameFact>,
    /// Seconds played, in all.
    #[serde(skip)]
    pub played_secs: u64,
    /// `played_secs` as text, "4 hr 6 min"; empty when the game never ran.
    pub played: String,
    /// Whole days, of 24 hours, since the game last closed. `None` when it never ran.
    #[serde(skip)]
    pub last_played_days: Option<u64>,
    /// `last_played_days` as text: "Today", "Yesterday" or "3 days ago"; empty when it never ran.
    pub last_played: String,
    /// For a game a catalog offers: its releases by version, highest first. Empty for an
    /// installed game.
    pub releases: Vec<ReleaseVm>,
    /// For a game a catalog offers: the name of the program that runs it, `KytyPS5`.
    pub emulator: String,
    /// For a game a catalog offers: where its downloads go; empty when the emulator can't
    /// install downloads.
    pub download_folder: String,
    /// For a game a catalog offers, how easy its best release is to download (see
    /// [`ReleaseVm::availability`]), and that in words; empty for an installed game.
    pub availability: String,
    pub availability_text: String,
    /// Why the download the player last asked for here didn't start; empty when it did.
    pub download_error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
///
/// In the contract its fields are `Vm` properties of their own: `catalog` for `games`, and
/// `catalog-` and the field's name for the others.
pub struct CatalogVm {
    /// A plugin that gives release catalogs is turned on. Without one there is no Catalog:
    /// a theme hides the ways to it.
    #[serde(rename = "catalog_available")]
    pub available: bool,
    /// The games the catalogs offer, as the Library shows its games, with a `New` or
    /// `Installed` badge. Current while the Catalog is the screen; on another screen, the
    /// grid as it last was.
    #[serde(rename = "catalog")]
    pub games: LibraryVm,
    /// Which copy of each catalog is in use, one line each, for example
    /// "KytyPS5: fetched from xyz.dev · updated 2026-10-18 · 120 releases".
    #[serde(rename = "catalog_sources")]
    pub sources: Vec<String>,
    /// How many releases the catalogs list, of every game.
    #[serde(rename = "catalog_releases")]
    pub releases: usize,
    /// When the list was updated, `YYYY-MM-DD`: the newest count of a release's seeders, else
    /// the newest date a catalog gives. Empty when neither says.
    #[serde(skip)]
    pub updated: String,
    /// `updated` as text, "Oct 19, 2026".
    #[serde(rename = "catalog_updated")]
    pub updated_text: String,
    /// The catalog plugins have not answered yet.
    #[serde(rename = "catalog_loading")]
    pub loading: bool,
    /// The catalog plugins' buttons for the Catalog screen (`catalog.action`), such as a
    /// refresh, and their badges, which say what the buttons did.
    #[serde(rename = "catalog_actions")]
    pub actions: Vec<SlotVm>,
    #[serde(rename = "catalog_status")]
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
    #[serde(skip)]
    pub size_bytes: Option<u64>,
    /// `size_bytes` as text, "7.6 GB"; empty when the catalog doesn't say.
    pub size: String,
    /// `YYYY-MM-DD`, or empty.
    #[serde(skip)]
    pub added: String,
    /// `added` as text, "Oct 15, 2026".
    #[serde(rename = "added")]
    pub added_text: String,
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
    /// `None` when the catalog has no count (empty in the contract). They are not live counts.
    #[serde(serialize_with = "mark::count_text")]
    pub seeders: Option<u64>,
    #[serde(serialize_with = "mark::count_text")]
    pub leechers: Option<u64>,
    /// When the catalog counted them, an RFC 3339 time; empty when it doesn't say.
    #[serde(skip)]
    pub counted_at: String,
    /// The day of `counted_at` as text, "Oct 7, 2026".
    #[serde(rename = "counted")]
    pub counted_text: String,
    /// How easy it is to download, from `seeders`: `good` (3 or more), `low` (1 or 2), `none`
    /// (0) or `unknown` (no count), and that in words.
    pub availability: String,
    pub availability_text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadVm {
    /// The job number.
    pub id: u64,
    pub plugin: PluginId,
    pub label: String,
    #[serde(skip)]
    pub received: u64,
    #[serde(skip)]
    pub total: Option<u64>,
    /// 0.0 to 1.0. 0 when the size is unknown.
    pub progress: f32,
    /// `received` and `total` as text, "12.5 MB of 40.0 MB".
    pub progress_text: String,
    /// `queued`, `downloading`, `verifying`, `paused`, `done`, `failed` or `cancelled`.
    pub state: String,
    pub error: String,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
///
/// In the contract its fields are `Vm` properties of their own.
pub struct DownloadsVm {
    #[serde(rename = "downloads")]
    pub jobs: Vec<DownloadVm>,
    /// The catalogs' downloads, newest first.
    pub torrents: Vec<TorrentVm>,
    /// "Keep sharing finished downloads".
    #[serde(rename = "seed_downloads")]
    pub seeding: bool,
    /// How many of the catalogs' downloads read their files, check or download now.
    #[serde(rename = "torrents_downloading")]
    pub downloading: usize,
    /// How many of the catalogs' downloads share their finished files now.
    #[serde(rename = "torrents_sharing")]
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
    #[serde(skip)]
    pub done: u64,
    #[serde(skip)]
    pub total: u64,
    /// Bytes a second.
    #[serde(skip)]
    pub download_rate: u64,
    #[serde(skip)]
    pub upload_rate: u64,
    /// How long the rest takes at `download_rate`, in seconds, while it downloads; 0 when no
    /// bytes arrive.
    #[serde(skip)]
    pub seconds_left: u64,
    #[serde(skip)]
    pub peers: u32,
    /// One line of its sizes, speed and peers, by its state: "1.0 GB of 3.0 GB · 2.0 MB/s ·
    /// 3 peers · 2 hr 5 min left".
    pub detail: String,
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
    /// What the file list shows: "120 files · 1.5 GB · first 100 shown".
    pub file_summary: String,
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
    #[serde(rename = "labels")]
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

///
/// In the contract its fields are `Vm` properties of their own: `settings` for `panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsVm {
    /// The launcher's own update.
    pub launcher_update: LauncherUpdateVm,
    /// This PC, and how each emulator's games run on it.
    pub hardware: HardwareVm,
    #[serde(rename = "settings")]
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
    #[serde(skip)]
    pub paths: Vec<String>,
    /// `paths` joined with "; ", for a text field.
    pub text: String,
    /// What to show: `text`, or the default when it is empty, with the home folder as `~`.
    pub shown: String,
    /// The plugins' default, if any. Empty when there is none.
    pub default: String,
    /// The names of the plugins that ask for it, and those joined with ", ".
    #[serde(skip)]
    pub requested_by: Vec<String>,
    #[serde(rename = "requested_by")]
    pub requested_by_text: String,
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
    /// Controller profiles the emulator has, as `name (device)`, and those joined with ", ".
    #[serde(skip)]
    pub controllers: Vec<String>,
    #[serde(rename = "controllers")]
    pub controllers_text: String,
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
    /// Its builds, when it is an emulator and turned on. With `has-emulator`.
    #[serde(serialize_with = "mark::has")]
    pub emulator: Option<EmulatorVm>,
    /// Its settings panels.
    #[serde(rename = "settings")]
    pub panels: Vec<SettingsPanelVm>,
    /// Its blocks in the `emulator.panel` slot: sections and actions. An emulator's are also
    /// in `emulator`.
    pub sections: Vec<SlotVm>,
    /// The actions of `sections`, for one row each.
    pub actions: Vec<SlotVm>,
    /// The folders it asks for, but not the game folders: those are in Settings.
    pub folders: Vec<FolderVm>,
    /// A theme that is the one shown.
    pub current: bool,
}

///
/// In the contract its fields are `Vm` properties of their own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmulatorsVm {
    pub emulators: Vec<EmulatorVm>,
    pub plugins: Vec<PluginRowVm>,
    /// `plugins` for the Emulators list: the emulators, then the services that describe their
    /// games.
    pub emulator_plugins: Vec<PluginRowVm>,
    /// `plugins` for the Themes list: the themes, the one shown with the status "in use".
    pub theme_plugins: Vec<PluginRowVm>,
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
    #[serde(skip)]
    pub elapsed_secs: u64,
    /// `elapsed_secs` as text, "12 min".
    pub elapsed: String,
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
    #[serde(serialize_with = "mark::thumbs")]
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
///
/// Each field is a `Vm` property, but for `catalog`, `downloads`, `settings` and `emulators`,
/// whose fields are. An optional part comes with `has-` and its name, such as `has-detail`.
pub struct AppView {
    pub screen: ScreenId,
    /// The game a Game Detail screen shows.
    #[serde(serialize_with = "mark::text")]
    pub game: Option<String>,
    pub can_go_back: bool,
    pub home: HomeVm,
    #[serde(serialize_with = "mark::has")]
    pub detail: Option<GameDetailVm>,
    #[serde(flatten)]
    pub catalog: CatalogVm,
    #[serde(flatten)]
    pub downloads: DownloadsVm,
    #[serde(flatten)]
    pub settings: SettingsVm,
    #[serde(flatten)]
    pub emulators: EmulatorsVm,
    /// The plugin the Plugin screen shows.
    #[serde(serialize_with = "mark::has")]
    pub plugin_page: Option<PluginPageVm>,
    #[serde(serialize_with = "mark::has")]
    pub running: Option<RunningVm>,
    /// Short messages for the player, newest last: the text of `notice_cards`.
    pub notices: Vec<String>,
    /// The notices the theme shows, newest last. The core keeps at most `MAX_NOTICES`.
    pub notice_cards: Vec<NoticeVm>,
    /// A game was started whose emulator isn't installed: the player is asked first.
    #[serde(serialize_with = "mark::has")]
    pub install_prompt: Option<InstallPromptVm>,
    pub boot: BootVm,
    /// Work going on in the background, for example "Downloading game art 3/40".
    #[serde(serialize_with = "mark::text")]
    pub task: Option<String>,
    /// The art of the game the newest notice is about.
    #[serde(serialize_with = "mark::thumb")]
    pub notice_art: Option<ImageRef>,
    /// What clicking the newest notice opens, for example "Open Downloads". Empty for nothing.
    pub notice_opens: String,
    /// "How far did it get?", for a game that just closed: its title.
    #[serde(serialize_with = "mark::has")]
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
    /// Names the notice for `NoticeCmd::Dismiss` and `NoticeCmd::Open`.
    pub id: u64,
    pub text: String,
    pub kind: NoticeKind,
    /// The art of the game the notice is about.
    #[serde(serialize_with = "mark::thumb")]
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
    #[serde(rename = "size")]
    pub size_bytes: Option<u64>,
}

/// The fields whose contract form isn't their plain `serde` form, and the names that mark
/// them. A field marked with a name goes to the serializer as a newtype struct of that name,
/// which a JSON serializer passes through as the value inside. The UI host's serializer reads
/// the name.
pub mod mark {
    use serde::{Serialize, Serializer};

    use super::{GameCardVm, ImageRef};

    /// An `Option<ImageRef>` shown small: a cover, tile or icon. An empty image for none.
    pub const THUMB: &str = "nexus.thumb";
    /// An `Option<ImageRef>` shown large: a background, logo or screenshot.
    pub const PICTURE: &str = "nexus.picture";
    /// A card's cover: as [`THUMB`], cut as the struct's `platform` needs. It comes with
    /// `<field>-state`: `loading`, `ready` or `none`.
    pub const COVER: &str = "nexus.cover";
    /// The Game Hub's cover: as [`PICTURE`], cut as the struct's `platform` needs.
    pub const COVER_PICTURE: &str = "nexus.cover-picture";
    /// A color, as `0xAARRGGBB`.
    pub const COLOR: &str = "nexus.color";
    /// An `Option` that comes with `has-<field>`. When it is `None` the field is left out.
    pub const HAS: &str = "nexus.has";
    /// A long list of game cards, which the host changes in place.
    pub const CARDS: &str = "nexus.cards";

    /// A value under a mark's name.
    struct Marked<'a, T: ?Sized>(&'static str, &'a T);

    impl<T: Serialize + ?Sized> Serialize for Marked<'_, T> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_newtype_struct(self.0, self.1)
        }
    }

    pub(crate) fn thumb<S: Serializer>(image: &Option<ImageRef>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(THUMB, image)
    }

    pub(crate) fn thumbs<S: Serializer>(images: &[ImageRef], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(images.iter().map(|image| Marked(THUMB, image)))
    }

    pub(crate) fn picture<S: Serializer>(image: &Option<ImageRef>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(PICTURE, image)
    }

    pub(crate) fn pictures<S: Serializer>(images: &[ImageRef], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(images.iter().map(|image| Marked(PICTURE, image)))
    }

    pub(crate) fn has_picture<S: Serializer>(image: &Option<ImageRef>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(HAS, &image.as_ref().map(|image| Marked(PICTURE, image)))
    }

    pub(crate) fn cover<S: Serializer>(image: &Option<ImageRef>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(COVER, image)
    }

    pub(crate) fn cover_picture<S: Serializer>(image: &Option<ImageRef>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(COVER_PICTURE, image)
    }

    /// Transparent for none: the theme draws it in its neutral look.
    pub(crate) fn color<S: Serializer>(rgb: &Option<[u8; 3]>, s: S) -> Result<S::Ok, S::Error> {
        let argb = rgb.map_or(0, |[r, g, b]| 0xff00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b));
        s.serialize_newtype_struct(COLOR, &argb)
    }

    pub(crate) fn has<T: Serialize, S: Serializer>(value: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(HAS, value)
    }

    pub(crate) fn cards<S: Serializer>(cards: &[GameCardVm], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(CARDS, cards)
    }

    /// A list position, -1 for none.
    pub(crate) fn index<S: Serializer>(index: &Option<usize>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(index.map_or(-1, |i| i as i64))
    }

    /// Text, "" for none.
    pub(crate) fn text<S: Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(text.as_deref().unwrap_or_default())
    }

    /// A count as text, "" for none.
    pub(crate) fn count_text<S: Serializer>(count: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&count.map(|n| n.to_string()).unwrap_or_default())
    }

    /// The value as JSON text, "" for none.
    pub(crate) fn json_text<T: Serialize, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
        let json = serde_json::to_value(value).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&if json.is_null() { String::new() } else { json.to_string() })
    }
}
