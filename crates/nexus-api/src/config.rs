use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PluginResult;

/// Which settings: the emulator's own, or one game's.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "game", rename_all = "lowercase")]
pub enum ConfigScope {
    Emulator,
    Game(String),
}

/// Describes and checks settings. The host stores the values in the plugin's scoped config,
/// and themes draw the settings form from the schema.
#[async_trait]
pub trait Configurator: Send + Sync {
    /// A JSON Schema, usually from `schemars::schema_for!`.
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<serde_json::Value>;

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<serde_json::Value>;

    /// `InvalidInput` with a message the player can read.
    async fn validate(&self, scope: &ConfigScope, value: &serde_json::Value) -> PluginResult<()>;
}
