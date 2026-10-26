//! Games a catalog offers to download, one list per console. A service plugin reads the
//! lists; the core gives each list to the emulator that plays its console. The host
//! downloads a release the player picks and gives the finished download to the emulator's
//! [`crate::GameInstaller`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Console, PluginResult};

/// One copy of a game in a catalog: a version, for a region, as one torrent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    /// The torrent's info hash in lowercase hex. It names the release, and its download.
    pub id: String,
    /// The catalog's own name for the release's topic, for example `6912182`. A torrent
    /// uploaded again has a new `id` and the same topic. Empty when the catalog doesn't say.
    pub topic: String,
    /// The console's id for the game, for example `PPSA01234`. Releases with the same title id
    /// are versions of one game.
    pub title_id: String,
    /// More title ids the release carries, for a release that holds several regions.
    pub title_ids: Vec<String>,
    /// A short platform label, for example `PS5` or `PC`.
    pub platform: String,
    /// For a PC release, the system it is for: `linux`, `windows` or `macos`. Empty when the
    /// catalog doesn't say, and for consoles.
    pub system: String,
    pub title: String,
    pub version: String,
    /// For example `EUR`, `USA`, `JPN` or `ASIA`. Empty when the catalog doesn't say.
    pub region: String,
    pub size_bytes: Option<u64>,
    /// When the catalog got it, `YYYY-MM-DD`.
    pub added: String,
    pub description: String,
    /// A magnet link.
    pub torrent: String,
    /// The game on Steam, when the catalog found it there: its words and art.
    pub steam: Option<SteamInfo>,
    /// A Windows game packed to run with Wine.
    pub wine_packed: bool,
    /// The release is a PKG package, not a folder, an archive or an image.
    pub package: bool,
    /// How many people share the torrent, as the catalog counted them. `None` when the catalog
    /// has no count.
    pub seeders: Option<u64>,
    /// How many people download the torrent, as the catalog counted them.
    pub leechers: Option<u64>,
    /// When the catalog counted the seeders and leechers, an RFC 3339 time. Empty when it
    /// doesn't say.
    pub counted_at: String,
    /// The catalog's genre text, for example `Action, Horror`.
    pub genre: String,
    pub developer: String,
    pub publisher: String,
    /// The game's release date as the catalog writes it: a date, or only a year.
    pub release_date: String,
    /// The year the game came out, for example `2023`.
    pub release_year: String,
    /// The game update the release holds, as the catalog writes it.
    pub update: String,
    /// Who plays: `Single player`, `Multiplayer`, or the catalog's own words, such as `2x`.
    pub players: String,
    /// The console firmware the release needs, for example `5.xx`.
    pub minimum_firmware: String,
    /// The languages of the game's text, as the catalog writes them.
    pub interface_languages: String,
    /// The languages of the game's speech, as the catalog writes them.
    pub audio_languages: String,
    /// How the release is packed, as the catalog writes it, for example `exfat` or `fpkg`.
    pub format: String,
    /// The release topic's pictures, `https://` addresses. The first one is the cover.
    pub image_urls: Vec<String>,
    /// A release a store installs, not a torrent: the link that asks the store to, for
    /// example `steam://install/440`. Its Install opens the link (the plugin's manifest must
    /// allow it in `permissions.links`) instead of downloading. Empty for a torrent.
    pub store: String,
    /// The player hid the game in its store (Steam's Hidden collection): the Catalog shows it
    /// only under its "Hidden" filter, as a game the player hid in Nexus.
    pub hidden: bool,
    /// The player marked the game a favorite in its store: its card says so.
    pub favorite: bool,
}

/// What Steam says about a release's game. Text is empty, and an address is empty, when
/// Steam doesn't say.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamInfo {
    /// The game's Steam app id.
    pub app_id: u64,
    /// Steam's short description.
    pub short: String,
    pub genres: Vec<String>,
    /// `YYYY-MM-DD`.
    pub release: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    /// The store's wide header image, an `https://` address.
    pub header: String,
    pub screenshots: Vec<String>,
    /// The library's 2:3 portrait.
    pub portrait: String,
    /// The library's wide art, for behind the game's screens.
    pub hero: String,
    /// The game's name as a picture, with a transparent background.
    pub logo: String,
    /// ProtonDB's tier for the Windows version: `platinum`, `gold`, `silver`, `bronze`,
    /// `borked`, `native` or `pending`. Empty when ProtonDB has none.
    pub protondb: String,
}

/// Which copy of a catalog a [`ReleaseList`] came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseSource {
    /// Fetched just now from the catalog's address.
    Fetched,
    /// The last fetched copy: the address didn't answer, or its answer didn't read.
    Cached,
    /// The copy the plugin ships with.
    #[default]
    Bundled,
}

/// One console's catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReleaseList {
    pub releases: Vec<Release>,
    pub source: ReleaseSource,
    /// The catalog's own version, for example `2026.10.18`.
    pub version: String,
    /// When the catalog was written, as the file says: an RFC 3339 time or `YYYY-MM-DD`.
    pub updated: String,
    /// Where it was fetched from, or would have been: a host name. Empty for a bundled copy
    /// with no address set.
    pub host: String,
    /// Why the fetch didn't give the list, for a cached or bundled copy when an address is set.
    pub problem: Option<String>,
}

/// The topic of the [`crate::Event::Custom`] a [`Releases`] plugin publishes when its catalogs
/// change between the core's asks, for example when a refresh in the background ends. The core
/// then asks for the catalogs again.
pub const RELEASES_CHANGED: &str = "releases.changed";

/// Lists the games one console's catalog offers to download.
#[async_trait]
pub trait Releases: Send + Sync {
    /// The catalog for `console`, the console an emulator's manifest names (`PS5`, `PS4`,
    /// `PC`). `None` when this plugin has no catalog for it.
    async fn releases(&self, console: &Console) -> PluginResult<Option<ReleaseList>>;
}
