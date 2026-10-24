use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::{ConfigScope, ConfigStore, PluginError, PluginId, PluginResult};
use serde_json::{Map, Value};

/// All settings in one JSON file: the app's own under `app`, each plugin's under
/// `plugins.<id>.<scope>`. Every change is written straight away.
pub struct ConfigService {
    path: Option<PathBuf>,
    doc: Mutex<Map<String, Value>>,
}

impl ConfigService {
    /// Loads `path`, or starts empty if it doesn't exist or can't be read.
    pub fn open(path: PathBuf) -> Self {
        let doc = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
            .unwrap_or_default();
        Self { path: Some(path), doc: Mutex::new(doc) }
    }

    pub fn in_memory() -> Self {
        Self { path: None, doc: Mutex::new(Map::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Map<String, Value>> {
        self.doc.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `Null` when unset.
    pub fn get(&self, keys: &[&str]) -> Value {
        let doc = self.lock();
        let mut node = doc.get(keys[0]);
        for key in &keys[1..] {
            node = node.and_then(|n| n.get(*key));
        }
        node.cloned().unwrap_or(Value::Null)
    }

    pub fn set(&self, keys: &[&str], value: Value) -> std::io::Result<()> {
        let mut doc = self.lock();
        let mut map = &mut *doc;
        for key in &keys[..keys.len() - 1] {
            let entry = map.entry((*key).to_owned()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            map = entry.as_object_mut().expect("just made it an object");
        }
        map.insert(keys[keys.len() - 1].to_owned(), value);
        self.save(&doc)
    }

    /// Removes a key. No error if it isn't there.
    pub fn remove(&self, keys: &[&str]) -> std::io::Result<()> {
        let mut doc = self.lock();
        let mut map = &mut *doc;
        for key in &keys[..keys.len() - 1] {
            match map.get_mut(*key).and_then(Value::as_object_mut) {
                Some(inner) => map = inner,
                None => return Ok(()),
            }
        }
        if map.remove(keys[keys.len() - 1]).is_none() {
            return Ok(());
        }
        self.save(&doc)
    }

    /// Writes `doc`. The caller holds the lock: two saves at once would share the temporary
    /// file, and an older copy could land after a newer one.
    fn save(&self, doc: &Map<String, Value>) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(doc).expect("JSON values serialize"))?;
        std::fs::rename(tmp, path)
    }

    pub fn scope_key(scope: &ConfigScope) -> String {
        match scope {
            ConfigScope::Emulator => "emulator".to_owned(),
            ConfigScope::Game(game) => format!("game:{game}"),
        }
    }
}

/// One plugin's view of the [`ConfigService`].
pub(crate) struct ScopedConfig {
    pub plugin: PluginId,
    pub service: Arc<ConfigService>,
}

#[async_trait]
impl ConfigStore for ScopedConfig {
    async fn get(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Ok(self.service.get(&["plugins", self.plugin.as_str(), &ConfigService::scope_key(scope)]))
    }

    async fn set(&self, scope: &ConfigScope, value: Value) -> PluginResult<()> {
        self.service
            .set(&["plugins", self.plugin.as_str(), &ConfigService::scope_key(scope)], value)
            .map_err(|e| PluginError::Storage(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        ConfigService::open(path.clone()).set(&["app", "theme"], Value::from("tv-carousel")).unwrap();
        assert_eq!(ConfigService::open(path).get(&["app", "theme"]), Value::from("tv-carousel"));
    }

    #[test]
    fn changes_from_many_threads_all_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = Arc::new(ConfigService::open(path.clone()));
        let writers: Vec<_> = (0..8)
            .map(|t| {
                let config = config.clone();
                std::thread::spawn(move || {
                    for n in 0..50 {
                        config.set(&["records", &format!("{t}-{n}")], Value::from(n)).expect("saved");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("no save failed");
        }
        let records = ConfigService::open(path).get(&["records"]);
        assert_eq!(records.as_object().map(|r| r.len()), Some(400), "the last save has every change");
    }
}
