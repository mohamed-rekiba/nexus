//! A game's settings, in its Game Hub: Starts, the file Play starts, and on Linux a Resolution,
//! which runs the game inside gamescope at a size of its own.

use nexus_api::serde_json::{self, Value};
use nexus_api::*;

use crate::{Pc, probe};

/// The setting of what Play starts: one of the manifest's files, its path from the game's
/// folder. Without it, the manifest's default (see [`crate::manifest::GameManifest::default_start`]).
pub const STARTS: &str = "starts";

/// The setting of the resolution a game runs at: `screen`, or `<width>x<height>`.
pub const RESOLUTION: &str = "resolution";

/// The resolutions a game can run at, inside gamescope.
pub mod resolution {
    /// Each choice, with its label. `screen` is the screen's own: the game runs as it is.
    pub const CHOICES: [(&str, &str); 8] = [
        ("screen", "Your screen"),
        ("640x480", "640 × 480"),
        ("800x600", "800 × 600"),
        ("1024x768", "1024 × 768"),
        ("1280x720", "1280 × 720"),
        ("1280x1024", "1280 × 1024"),
        ("1600x900", "1600 × 900"),
        ("1920x1080", "1920 × 1080"),
    ];

    /// The width and height of a choice; `None` for `screen` or one that isn't a choice.
    pub fn size(choice: &str) -> Option<(u32, u32)> {
        if !CHOICES.iter().any(|(c, _)| *c == choice) {
            return None;
        }
        let (w, h) = choice.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }
}

fn no_settings() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

#[async_trait]
impl Configurator for Pc {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let ConfigScope::Game(id) = scope else { return Ok(no_settings()) };
        let game = self.game(id).await?;
        let paths: Vec<String> = game.manifest.starts.iter().map(|s| s.path.clone()).collect();
        // The row shows each file by its short name; the line under it the whole path.
        let starts: Vec<Value> = paths
            .iter()
            .zip(probe::short_names(&paths))
            .map(|(path, name)| serde_json::json!({ "const": path, "description": name }))
            .collect();
        let start = self.start(&game).await;
        let mut properties = serde_json::Map::new();
        properties.insert(
            STARTS.into(),
            serde_json::json!({
                "title": "Starts",
                "description": format!("Play starts {}, from the game's folder.", start.path),
                "type": "string",
                "oneOf": starts,
                "default": game.manifest.default_start(self.platform).path,
            }),
        );
        // gamescope is a Linux program.
        if self.platform == Platform::Linux {
            let choices: Vec<Value> = resolution::CHOICES
                .iter()
                .map(|(value, label)| serde_json::json!({ "const": value, "description": label }))
                .collect();
            properties.insert(
                RESOLUTION.into(),
                serde_json::json!({
                    "title": "Resolution",
                    "description": "An old game made for a small screen fills yours at its own resolution: it runs inside gamescope, which scales it up.",
                    "type": "string",
                    "oneOf": choices,
                    "default": "screen",
                }),
            );
        }
        Ok(serde_json::json!({ "type": "object", "properties": properties }))
    }

    async fn defaults(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        Ok(serde_json::json!({}))
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        if !value.is_object() {
            return Err(PluginError::InvalidInput("settings are a JSON object".into()));
        }
        if let (ConfigScope::Game(id), Some(chosen)) = (scope, value.get(STARTS).and_then(Value::as_str))
            && !self.game(id).await?.manifest.has(chosen)
        {
            return Err(PluginError::InvalidInput(format!("{chosen} isn't a file this game starts")));
        }
        match value.get(RESOLUTION).and_then(Value::as_str) {
            Some(choice) if !resolution::CHOICES.iter().any(|(c, _)| *c == choice) => {
                Err(PluginError::InvalidInput(format!("{choice} isn't a resolution to choose")))
            }
            Some(choice) if choice != "screen" && self.platform != Platform::Linux => {
                Err(PluginError::InvalidInput("A resolution of its own needs gamescope, which is Linux's".into()))
            }
            _ => Ok(()),
        }
    }
}
