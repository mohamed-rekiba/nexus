//! The optional compatibility capability: how far a game gets in the emulator.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PluginResult;

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

impl CompatLevel {
    /// The levels a player can give a game after playing it, best first.
    pub const RESULTS: [CompatLevel; 4] =
        [CompatLevel::Ingame, CompatLevel::Menus, CompatLevel::Boots, CompatLevel::Nothing];

    /// Its name, as it reads and writes with `serde`: `unknown`, `nothing`, `boots`, `menus`,
    /// `ingame` or `playable`.
    pub fn as_str(self) -> &'static str {
        match self {
            CompatLevel::Unknown => "unknown",
            CompatLevel::Nothing => "nothing",
            CompatLevel::Boots => "boots",
            CompatLevel::Menus => "menus",
            CompatLevel::Ingame => "ingame",
            CompatLevel::Playable => "playable",
        }
    }
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
