//! Optional capabilities: compatibility, saves, controllers, shaders.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{PluginResult, RelPath};

/// How far a game gets in the emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompatLevel {
    Unknown,
    Nothing,
    Boots,
    Menus,
    Ingame,
    Playable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatInfo {
    pub level: CompatLevel,
    #[serde(default)]
    pub notes: Option<String>,
    /// The system `level` was tested on, for example `macOS`. `None` when no one says.
    #[serde(default)]
    pub source: Option<String>,
    /// The community list's page, for the player to read or report.
    #[serde(default)]
    pub page: Option<String>,
    /// When `level` was last tested, as `YYYY-MM-DD`. `None` when the list doesn't say.
    #[serde(default)]
    pub tested: Option<String>,
}

#[async_trait]
pub trait Compatibility: Send + Sync {
    async fn status(&self, game: &str) -> PluginResult<CompatInfo>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSlot {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub modified_unix: Option<u64>,
    pub size_bytes: u64,
}

#[async_trait]
pub trait Saves: Send + Sync {
    async fn list(&self, game: &str) -> PluginResult<Vec<SaveSlot>>;

    /// Copies the save into the plugin's data folder and returns where.
    async fn backup(&self, game: &str, slot: &str) -> PluginResult<RelPath>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerProfile {
    pub id: String,
    pub name: String,
    /// For example `DualSense` or `Xbox`.
    #[serde(default)]
    pub device_hint: Option<String>,
}

#[async_trait]
pub trait Controllers: Send + Sync {
    async fn profiles(&self) -> PluginResult<Vec<ControllerProfile>>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderCacheInfo {
    pub entries: u64,
    pub size_bytes: u64,
}

#[async_trait]
pub trait Shaders: Send + Sync {
    async fn cache(&self, game: &str) -> PluginResult<ShaderCacheInfo>;

    async fn clear_cache(&self, game: &str) -> PluginResult<()>;
}
