//! Games a catalog offers to download, one list per console. A service plugin reads the
//! lists; the core gives each list to the emulator that plays its console. The host
//! downloads a release the player picks and gives the finished download to the emulator's
//! [`crate::GameInstaller`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Console, GameMetadata, PluginResult};

/// One copy of a game in a catalog: a version, for a region, as one torrent or a store's
/// install. The fields are the ones the host sorts, groups, searches and downloads by; what a
/// catalog only shows goes in `facts`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    /// The torrent's info hash in lowercase hex. It names the release, and its download.
    pub id: String,
    /// The catalog's own name for the release's listing. A torrent uploaded again has a new
    /// `id` and the same topic. Empty when the catalog doesn't say.
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
    /// How many people share the torrent, as the catalog counted them. `None` when the catalog
    /// has no count.
    pub seeders: Option<u64>,
    /// How many people download the torrent, as the catalog counted them.
    pub leechers: Option<u64>,
    /// When the catalog counted the seeders and leechers, an RFC 3339 time. Empty when it
    /// doesn't say.
    pub counted_at: String,
    /// The release's pictures, `https://` addresses. The first one is the cover when
    /// `metadata` has none.
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
    /// What a store says about the release's game: its words and art. The host fills in from
    /// it what the stores' [`crate::Metadata`] plugins leave out.
    pub metadata: Option<GameMetadata>,
    /// What the catalog says about the release, in the order the Game Hub lists them.
    pub facts: Vec<Fact>,
    /// Why the release may not install, shown beside it. The release stays listed.
    pub install_warning: Option<String>,
}

impl Release {
    /// The release's first fact of `kind`.
    pub fn fact(&self, kind: FactKind) -> Option<&Fact> {
        self.facts.iter().find(|f| f.kind == kind)
    }

    /// The value of the release's first fact of `kind`; empty when it has none.
    pub fn fact_text(&self, kind: FactKind) -> &str {
        self.fact(kind).map_or("", |f| f.value.as_str())
    }
}

/// One thing a catalog says about a release: "Publisher: Sony".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fact {
    pub kind: FactKind,
    /// What the player reads before the value, for example "Minimum firmware".
    pub label: String,
    pub value: String,
    /// A page about the fact, an `https://` address. Empty for none.
    pub url: String,
}

impl Fact {
    pub fn new(kind: FactKind, label: &str, value: impl Into<String>) -> Self {
        Self { kind, label: label.into(), value: value.into(), url: String::new() }
    }
}

/// What a [`Fact`] is about. The Game Hub lists every kind but `Genre` and `Compatibility`,
/// with its kebab-case name as the fact's id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactKind {
    Publisher,
    Developer,
    /// When the game came out: `YYYY-MM-DD`, or only a year. The host writes a date in words
    /// and sorts by it.
    ReleaseDate,
    /// Who plays: `Single player`, `Multiplayer`, or the catalog's own words.
    Players,
    /// The game update the release holds.
    Update,
    /// How the release is packed, for example `exfat` or `fpkg`.
    Format,
    /// The console firmware the release needs, for example `5.xx`.
    Firmware,
    InterfaceLanguages,
    AudioLanguages,
    /// The catalog's genre words, for example `Action, Horror`. The host puts the game under
    /// its genre chips by them, and doesn't list them.
    Genre,
    /// How well the game runs through a compatibility layer, as a community list rates it:
    /// the list's word as the value (ProtonDB's `gold`), its page as the url. The Game Hub
    /// shows it on its own, not in the list.
    Compatibility,
    /// Any other fact: listed by its label.
    #[default]
    #[serde(other)]
    Other,
}

impl FactKind {
    /// The kind's name in kebab case: the id of the fact the Game Hub lists.
    pub fn id(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Developer => "developer",
            Self::ReleaseDate => "release-date",
            Self::Players => "players",
            Self::Update => "update",
            Self::Format => "format",
            Self::Firmware => "firmware",
            Self::InterfaceLanguages => "interface-languages",
            Self::AudioLanguages => "audio-languages",
            Self::Genre => "genre",
            Self::Compatibility => "compatibility",
            Self::Other => "other",
        }
    }
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

    /// For a catalog whose store installs its releases: the installs the store is doing now,
    /// started from Nexus or in the store itself. The host shows them in Downloads while they
    /// last. None by default.
    async fn store_installs(&self) -> PluginResult<Vec<StoreInstall>> {
        Ok(Vec::new())
    }
}

/// A game its store is installing, as the store says (Steam's app files).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreInstall {
    /// The release it installs: its id in this plugin's catalog.
    pub release: String,
    pub title: String,
    pub phase: StorePhase,
    /// Bytes done of `total` in this phase.
    pub done: u64,
    pub total: u64,
    /// The link that opens the store's own page of its downloads (`steam://open/downloads`);
    /// the plugin's `permissions.links` must allow it.
    pub open: String,
}

/// Where a store's install is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorePhase {
    /// Waiting for its turn, or for the store to start.
    #[default]
    Queued,
    Downloading,
    Paused,
    /// Downloaded: the store writes it into place.
    Installing,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fact's id is its kind's serde name, and a kind this host doesn't know reads as `Other`.
    #[test]
    fn a_fact_kind_s_id_is_its_wire_name_and_an_unknown_kind_is_other() {
        use FactKind::*;
        let kinds = [
            Publisher,
            Developer,
            ReleaseDate,
            Players,
            Update,
            Format,
            Firmware,
            InterfaceLanguages,
            AudioLanguages,
            Genre,
            Compatibility,
            Other,
        ];
        for kind in kinds {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.id(), "{kind:?}");
        }
        let fact: Fact =
            serde_json::from_value(serde_json::json!({ "kind": "rating-board", "value": "PEGI 12" })).unwrap();
        assert_eq!((fact.kind, fact.value.as_str(), fact.url.as_str()), (Other, "PEGI 12", ""));
    }

    #[test]
    fn a_release_finds_its_facts_by_kind() {
        let release = Release {
            facts: vec![Fact::new(FactKind::Genre, "Genre", "Action"), Fact::new(FactKind::Genre, "Genre", "Horror")],
            ..Release::default()
        };
        assert_eq!(release.fact_text(FactKind::Genre), "Action", "the first of its kind");
        assert_eq!(release.fact_text(FactKind::Publisher), "", "none");
    }
}
