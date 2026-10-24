//! UI that emulator plugins contribute as data. Themes decide where and how it renders.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{ConfigScope, ImageSource, Order, PluginResult};

/// A named place in the UI that plugins fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SlotId {
    /// Small labels on a game's card and detail page.
    #[serde(rename = "game.badge")]
    GameBadge,
    /// Buttons on a game's detail page.
    #[serde(rename = "game.action")]
    GameAction,
    /// Blocks of extra information on a game's detail page.
    #[serde(rename = "game.detail-section")]
    GameDetailSection,
    /// A settings form in the Settings screen.
    #[serde(rename = "settings.panel")]
    SettingsPanel,
    /// Blocks on a plugin's page, for an emulator and for a service, and in the Emulator
    /// Management screen.
    #[serde(rename = "emulator.panel")]
    EmulatorPanel,
    /// Buttons on the Catalog screen, and badges that say what they did, from a service with
    /// the `releases` capability: for example a catalog refresh and when it last ran.
    #[serde(rename = "catalog.action")]
    CatalogAction,
}

/// A slot action the host runs itself: opens the Settings screen, for example from a button
/// that says to turn a setting on.
pub const HOST_OPEN_SETTINGS: &str = "host:open-settings";

impl SlotId {
    pub const ALL: [SlotId; 6] = [
        Self::GameBadge,
        Self::GameAction,
        Self::GameDetailSection,
        Self::SettingsPanel,
        Self::EmulatorPanel,
        Self::CatalogAction,
    ];

    /// Slots that belong to one game. The host asks only the plugin that owns the game.
    pub fn is_per_game(self) -> bool {
        matches!(self, Self::GameBadge | Self::GameAction | Self::GameDetailSection)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GameBadge => "game.badge",
            Self::GameAction => "game.action",
            Self::GameDetailSection => "game.detail-section",
            Self::SettingsPanel => "settings.panel",
            Self::EmulatorPanel => "emulator.panel",
            Self::CatalogAction => "catalog.action",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRequest {
    pub slot: SlotId,
    /// The plugin's own game id, for per-game slots.
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionStyle {
    Primary,
    #[default]
    Secondary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionRow {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlotContent {
    Badge {
        label: String,
        /// The label for small places, such as a game's card. Empty for the same as `label`.
        #[serde(default)]
        short: String,
        #[serde(default)]
        tone: Tone,
        #[serde(default)]
        icon: Option<ImageSource>,
    },
    Action {
        label: String,
        /// Passed back to [`Slots::run_action`] when the player picks the button.
        action: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default)]
        style: ActionStyle,
        #[serde(default)]
        icon: Option<ImageSource>,
    },
    Section {
        title: String,
        rows: Vec<SectionRow>,
    },
    /// A settings form. The theme draws it from the plugin's [`crate::Configurator`] schema.
    SettingsPanel {
        title: String,
        scope: ConfigScope,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotItem {
    /// Unique within the plugin. Themes label the rendered item `<plugin-id>:<id>`.
    pub id: String,
    #[serde(default)]
    pub order: Order,
    pub content: SlotContent,
}

#[async_trait]
pub trait Slots: Send + Sync {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>>;

    async fn run_action(&self, action: &str, game: Option<&str>, args: &serde_json::Value) -> PluginResult<()>;
}
