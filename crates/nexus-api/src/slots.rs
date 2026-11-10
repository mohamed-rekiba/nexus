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

/// The topic of the [`crate::Event::Custom`] a plugin publishes when its emulator panel
/// (`emulator.panel`) changed between the core's asks, for example when work in the background
/// ends. The core then asks for the panel again.
pub const SLOTS_CHANGED: &str = "slots.changed";

/// A slot action the host runs itself: opens the Settings screen, for example from a button
/// that says to turn a setting on.
pub const HOST_OPEN_SETTINGS: &str = "host:open-settings";

/// A slot action the host runs itself: starts the emulator's own window without a game, from
/// [`crate::Launcher::open_emulator`]. It runs as a session: no game starts until it closes,
/// and its time is not play time.
pub const HOST_OPEN_EMULATOR: &str = "host:open-emulator";

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

/// A row of a [`SlotContent::Block`]. A row with an `action` is a button: picking it runs the
/// action, as [`SlotContent::Action`] does.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlockRow {
    pub label: String,
    /// On the right of the label, or the state of a switch in words.
    #[serde(default)]
    pub value: String,
    /// A line under the label.
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub tone: Tone,
    /// From 0 to 1: a bar under the label, for work that runs.
    #[serde(default)]
    pub progress: Option<f32>,
    /// Passed back to [`Slots::run_action`] when the player picks the row.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub args: serde_json::Value,
    /// An action row drawn as a switch, on or off.
    #[serde(default)]
    pub switch: Option<bool>,
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
    /// A block on the plugin's page, after its build: rows of facts, progress bars, buttons
    /// and switches, under a title. Other places don't show it.
    Block {
        title: String,
        rows: Vec<BlockRow>,
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
