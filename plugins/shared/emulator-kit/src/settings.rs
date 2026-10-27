//! An emulator's settings: the type the plugin declares, stored by the host, read over this
//! system's defaults.

use std::marker::PhantomData;
use std::sync::Arc;

use nexus_api::schemars::{JsonSchema, SchemaGenerator};
use nexus_api::serde::Serialize;
use nexus_api::serde::de::DeserializeOwned;
use nexus_api::serde_json::{self, Value};
use nexus_api::{ConfigScope, ConfigStore, Configurator, Platform, PluginError, PluginResult, async_trait};

/// The stored field that turns your own build on. Both emulators name it so.
pub const OWN_BUILD: &str = "own_build";

/// What an emulator plugin's settings type tells the kit.
pub trait Settings: Serialize + DeserializeOwned + JsonSchema + Default + Send + Sync + 'static {
    /// Per-game settings: each field optional, and a set one wins over the emulator's.
    type Game: Serialize + DeserializeOwned + JsonSchema + Default + Send + Sync + 'static;

    /// The settings before the player changes any, on `platform`.
    fn defaults(_platform: Option<Platform>) -> Self {
        Self::default()
    }

    /// Whether your own build runs instead of a downloaded one.
    fn own_build(&self) -> bool;

    /// Refuses settings that read but make no sense, as a release repo that isn't owner/repo.
    fn check(&self, _platform: Option<Platform>) -> PluginResult<()> {
        Ok(())
    }

    /// Settings stored in an older shape, rewritten. `None` when there is nothing to change.
    fn migrate(_stored: &Value) -> Option<Value> {
        None
    }
}

/// The emulator settings in the host's store.
pub struct SettingsStore<S> {
    config: Arc<dyn ConfigStore>,
    platform: Option<Platform>,
    settings: PhantomData<fn() -> S>,
}

impl<S: Settings> SettingsStore<S> {
    pub fn new(config: Arc<dyn ConfigStore>, platform: Option<Platform>) -> Self {
        Self { config, platform, settings: PhantomData }
    }

    pub fn defaults(&self) -> S {
        S::defaults(self.platform)
    }

    async fn stored(&self) -> Value {
        self.config.get(&ConfigScope::Emulator).await.unwrap_or(Value::Null)
    }

    /// Rewrites settings stored in an older shape. The plugin calls it once, at `init`.
    pub async fn migrate(&self) -> PluginResult<()> {
        match S::migrate(&self.stored().await) {
            Some(migrated) => self.config.set(&ConfigScope::Emulator, migrated).await,
            None => Ok(()),
        }
    }

    /// The stored settings over this system's defaults. Settings that don't read give the
    /// defaults.
    pub async fn load(&self) -> S {
        over(self.defaults(), &self.stored().await, false)
    }

    /// The settings a game runs with: `config` (the emulator's settings with the game's on top,
    /// from the host) over the stored ones. A null field falls back to the stored one.
    pub async fn for_launch(&self, config: &Value) -> S {
        over(self.load().await, config, true)
    }

    pub async fn set_own_build(&self, on: bool) -> PluginResult<()> {
        let mut stored = self.stored().await;
        if !stored.is_object() {
            stored = serde_json::to_value(self.defaults())?;
        }
        stored[OWN_BUILD] = Value::Bool(on);
        self.config.set(&ConfigScope::Emulator, stored).await
    }

    /// Reads `value` as settings, or says why not.
    pub fn validate(&self, value: &Value) -> PluginResult<()> {
        let settings =
            serde_json::from_value::<S>(value.clone()).map_err(|e| PluginError::InvalidInput(e.to_string()))?;
        settings.check(self.platform)
    }
}

/// `fields` over `base`, or `base` when the result doesn't read. `skip_null`: a null field
/// keeps the base's value.
fn over<S: Settings>(base: S, fields: &Value, skip_null: bool) -> S {
    let (Ok(Value::Object(mut merged)), Some(fields)) = (serde_json::to_value(&base), fields.as_object()) else {
        return base;
    };
    for (key, value) in fields {
        if !(skip_null && value.is_null()) {
            merged.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or(base)
}

/// The default settings capability of a plugin with a settings type: the schema of `S` and of
/// its per-game settings, their defaults, and the checks they read with.
#[async_trait]
impl<S: Settings> Configurator for SettingsStore<S> {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let schema = match scope {
            ConfigScope::Emulator => SchemaGenerator::default().into_root_schema_for::<S>(),
            ConfigScope::Game(_) => SchemaGenerator::default().into_root_schema_for::<S::Game>(),
        };
        Ok(serde_json::to_value(schema)?)
    }

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Ok(match scope {
            ConfigScope::Emulator => serde_json::to_value(SettingsStore::defaults(self))?,
            ConfigScope::Game(_) => serde_json::to_value(S::Game::default())?,
        })
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        match scope {
            ConfigScope::Emulator => SettingsStore::validate(self, value),
            ConfigScope::Game(_) => serde_json::from_value::<S::Game>(value.clone())
                .map(|_| ())
                .map_err(|e| PluginError::InvalidInput(e.to_string())),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testing::MemConfig;
    use nexus_api::serde::Deserialize;
    use nexus_api::serde_json::json;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(crate = "nexus_api::serde", deny_unknown_fields)]
    #[schemars(crate = "nexus_api::schemars")]
    pub struct TestSettings {
        #[serde(default)]
        pub fast: bool,
        #[serde(default)]
        pub own_build: bool,
        #[serde(default)]
        pub repo: String,
    }

    impl Default for TestSettings {
        fn default() -> Self {
            Self { fast: false, own_build: false, repo: "upstream".into() }
        }
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
    #[serde(crate = "nexus_api::serde", deny_unknown_fields)]
    #[schemars(crate = "nexus_api::schemars")]
    pub struct TestGameSettings {
        #[serde(default)]
        pub fast: Option<bool>,
    }

    impl Settings for TestSettings {
        type Game = TestGameSettings;

        fn defaults(platform: Option<Platform>) -> Self {
            Self { fast: platform == Some(Platform::Macos), ..Self::default() }
        }

        fn own_build(&self) -> bool {
            self.own_build
        }

        fn check(&self, _platform: Option<Platform>) -> PluginResult<()> {
            if self.repo.contains(' ') {
                return Err(PluginError::InvalidInput("no spaces".into()));
            }
            Ok(())
        }
    }

    fn store(platform: Option<Platform>) -> (Arc<MemConfig>, SettingsStore<TestSettings>) {
        let config = Arc::new(MemConfig::default());
        (config.clone(), SettingsStore::new(config, platform))
    }

    #[tokio::test]
    async fn stored_fields_go_over_this_system_s_defaults() {
        let (config, settings) = store(Some(Platform::Macos));
        assert!(settings.load().await.fast, "the macOS default");
        config.set(&ConfigScope::Emulator, json!({ "repo": "fork" })).await.unwrap();
        let loaded = settings.load().await;
        assert_eq!((loaded.fast, loaded.repo.as_str()), (true, "fork"), "a field not stored keeps its default");
        config.set(&ConfigScope::Emulator, json!({ "unknown": 1 })).await.unwrap();
        assert_eq!(settings.load().await, TestSettings::defaults(Some(Platform::Macos)));
    }

    #[tokio::test]
    async fn a_game_s_null_field_falls_back_to_the_emulator_s() {
        let (config, settings) = store(None);
        config.set(&ConfigScope::Emulator, json!({ "fast": true })).await.unwrap();
        assert!(settings.for_launch(&json!({ "fast": null })).await.fast);
        assert!(!settings.for_launch(&json!({ "fast": false })).await.fast);
    }

    #[tokio::test]
    async fn turning_own_build_on_keeps_the_other_settings() {
        let (config, settings) = store(Some(Platform::Macos));
        settings.set_own_build(true).await.unwrap();
        assert_eq!(config.emulator(), json!({ "fast": true, "own_build": true, "repo": "upstream" }));
        config.set(&ConfigScope::Emulator, json!({ "repo": "fork" })).await.unwrap();
        settings.set_own_build(false).await.unwrap();
        assert_eq!(config.emulator(), json!({ "repo": "fork", "own_build": false }));
    }

    #[tokio::test]
    async fn validate_reads_then_checks() {
        let (_, settings) = store(None);
        assert!(settings.validate(&json!({ "repo": "a b" })).is_err());
        assert!(settings.validate(&json!({ "nope": 1 })).is_err());
        settings.validate(&json!({ "repo": "fork" })).unwrap();
    }
}
