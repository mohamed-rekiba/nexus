use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{PluginResult, RelPath};

/// An image a plugin points at: a web address, a file in the plugin's `assets` folder, or a
/// file in a host folder the plugin may read (a game's own icon).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum ImageSource {
    Url(String),
    Asset(RelPath),
    Folder(crate::HostPath),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub offset: u32,
    pub limit: u32,
}

impl PageRequest {
    pub const ALL: Self = Self { offset: 0, limit: u32::MAX };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// The size of the whole list, if the catalog knows it.
    pub total: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSummary {
    /// The plugin's own id for the game. The host makes it global with a [`crate::GameKey`].
    pub id: String,
    pub title: String,
    /// A short platform label shown on cards, for example `PS4`.
    pub platform: String,
    #[serde(default)]
    pub cover: Option<ImageSource>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub release_year: Option<u16>,
    /// The console's id for the game (`PPSA01234`, `CUSA01234`), for [`Metadata`] plugins.
    #[serde(default)]
    pub title_id: Option<String>,
    /// The store's id for this copy (`EP0002-PPSA02433_00-…`), which also gives its region.
    #[serde(default)]
    pub content_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDetails {
    pub summary: GameSummary,
    pub description: String,
    #[serde(default)]
    pub developer: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub background: Option<ImageSource>,
    /// Short facts about the game, for example its region or age rating. A theme picks the
    /// ones it shows by id; see [`GameFact`].
    #[serde(default)]
    pub facts: Vec<GameFact>,
    /// The game's folder, for "Open game folder". `None` when it has none on disk.
    #[serde(default)]
    pub folder: Option<crate::HostPath>,
    /// Other folders that are the game's, such as its update or its add-ons beside it:
    /// uninstalling the game deletes them with its folder.
    #[serde(default)]
    pub also: Vec<crate::HostPath>,
}

/// One fact about a game: "Region: Europe". Themes know these ids: `region`, `age`
/// (an age rating), `version`, `title-id`, `content-id` and `folder`. Others are shown by
/// their label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameFact {
    pub id: String,
    pub label: String,
    pub value: String,
}

impl GameFact {
    pub fn new(id: &str, label: &str, value: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), value: value.into() }
    }
}

/// A game a [`Metadata`] plugin is asked about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameRef {
    /// A short platform label, for example `PS5`.
    pub platform: String,
    pub title_id: String,
    /// Empty when the catalog doesn't know it.
    pub content_id: String,
    pub title: String,
    /// The copy's region, for example `EUR` or `USA`, when there is no content id to give it.
    /// Empty when unknown.
    #[serde(default)]
    pub region: String,
}

/// A star rating: `score` out of 5, from `total` players.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub score: f32,
    pub total: u64,
}

/// What a store's catalog says about a game. Every field may be empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameMetadata {
    pub title: String,
    /// The full text about the game, for its page.
    pub description: String,
    /// One line about the game, for Home. Empty when the store has none worth showing.
    pub tagline: String,
    pub publisher: String,
    pub genres: Vec<String>,
    /// `YYYY-MM-DD`.
    pub release: String,
    pub rating: Option<Rating>,
    /// For example `PEGI 12`.
    pub age_rating: String,
    /// Square art, for tiles.
    pub icon: Option<ImageSource>,
    /// Portrait art, 2:3.
    pub cover: Option<ImageSource>,
    /// The art behind the game's screens.
    pub background: Option<ImageSource>,
    /// The game's name as a picture, with a transparent background.
    pub logo: Option<ImageSource>,
    pub screenshots: Vec<ImageSource>,
    /// The game's page in the store.
    pub store_url: String,
    /// The game's trailer, a video file's address; empty for none.
    pub trailer: String,
}

/// Describes games from a store's catalog, by their console ids.
#[async_trait]
pub trait Metadata: Send + Sync {
    /// `None` when the catalog has no such game.
    async fn describe(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkKind {
    Cover,
    Background,
    Logo,
    Icon,
}

/// Lists, searches and describes the games one emulator can run.
#[async_trait]
pub trait Catalog: Send + Sync {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>>;

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>>;

    /// `NotFound` when the catalog has no such game.
    async fn details(&self, game: &str) -> PluginResult<GameDetails>;

    async fn artwork(&self, game: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>>;
}
