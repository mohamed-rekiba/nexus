//! Emulator versions, one folder each under `builds/<tag>`, with the one games start (the
//! current one) and the one before it, for a rollback. Saved in the plugin's `versions.json`.
//!
//! Each change reads `versions.json`, changes it and writes it back under one lock, so two
//! changes at the same time never lose one of them.

use std::sync::Arc;

use nexus_api::serde_json::{self, json};
use nexus_api::{PluginError, PluginResult, RelPath, Storage};
use tokio::sync::Mutex;

const STATE: &str = "versions.json";
pub const BUILDS: &str = "builds";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub current: Option<String>,
    pub previous: Option<String>,
}

fn path(p: &str) -> PluginResult<RelPath> {
    Ok(RelPath::new(p)?)
}

/// The folder of a version.
pub fn folder(tag: &str) -> PluginResult<RelPath> {
    path(BUILDS)?.join(tag).map_err(Into::into)
}

/// The versions in one plugin's storage.
pub struct Versions {
    storage: Arc<dyn Storage>,
    /// Held across each read-modify-write of `versions.json`.
    lock: Mutex<()>,
}

impl Versions {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage, lock: Mutex::new(()) }
    }

    pub async fn state(&self) -> State {
        let Ok(bytes) = self.storage.read(&RelPath::new(STATE).expect("a plain name")).await else {
            return State::default();
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        State { current: text("current"), previous: text("previous") }
    }

    async fn save(&self, state: &State) -> PluginResult<()> {
        let body = json!({ "current": state.current, "previous": state.previous });
        self.storage.write(&path(STATE)?, body.to_string().as_bytes()).await
    }

    /// Installed versions, the current one last (the host shows the last one as installed).
    pub async fn installed(&self) -> PluginResult<Vec<String>> {
        let mut tags: Vec<String> =
            self.storage.list(&path(BUILDS)?).await?.into_iter().filter(|t| !t.ends_with(".unpacking")).collect();
        let current = self.state().await.current;
        tags.sort_by_key(|t| Some(t) == current.as_ref());
        Ok(tags)
    }

    /// Whether `tag` is a downloaded version.
    pub async fn has(&self, tag: &str) -> PluginResult<bool> {
        Ok(self.installed().await?.iter().any(|t| t == tag))
    }

    /// Makes `tag` current; the old current becomes the previous one.
    pub async fn activate(&self, tag: &str) -> PluginResult<()> {
        let _held = self.lock.lock().await;
        let mut s = self.state().await;
        if s.current.as_deref() != Some(tag) {
            s.previous = s.current.take();
        }
        s.current = Some(tag.to_owned());
        self.save(&s).await
    }

    /// Goes back to the previous version. Returns it.
    pub async fn rollback(&self) -> PluginResult<String> {
        let _held = self.lock.lock().await;
        let mut s = self.state().await;
        let previous =
            s.previous.clone().ok_or_else(|| PluginError::Precondition("no earlier version to go back to".into()))?;
        if !self.storage.exists(&folder(&previous)?).await? {
            return Err(PluginError::Precondition(format!("{previous} is no longer installed")));
        }
        s.previous = s.current.replace(previous.clone());
        self.save(&s).await?;
        Ok(previous)
    }

    /// Removes every version but the current and the previous one.
    pub async fn prune(&self) -> PluginResult<()> {
        let _held = self.lock.lock().await;
        let s = self.state().await;
        for tag in self.installed().await? {
            if Some(&tag) != s.current.as_ref() && Some(&tag) != s.previous.as_ref() {
                self.storage.remove(&folder(&tag)?).await?;
            }
        }
        Ok(())
    }

    /// Removes a version's folder and forgets it.
    pub async fn remove(&self, tag: &str) -> PluginResult<()> {
        let _held = self.lock.lock().await;
        self.forget(tag).await
    }

    async fn forget(&self, tag: &str) -> PluginResult<()> {
        self.storage.remove(&folder(tag)?).await?;
        let mut s = self.state().await;
        if s.previous.as_deref() == Some(tag) {
            s.previous = None;
        }
        if s.current.as_deref() == Some(tag) {
            s.current = s.previous.take();
        }
        self.save(&s).await
    }

    /// Removes `tag` after its install failed, so a broken build is never listed. The current
    /// build stays: games start from it.
    pub async fn discard(&self, tag: &str) -> PluginResult<()> {
        let _held = self.lock.lock().await;
        if self.state().await.current.as_deref() == Some(tag) {
            return Ok(());
        }
        self.forget(tag).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MemStorage;

    fn versions() -> (Arc<MemStorage>, Versions) {
        let storage = Arc::new(MemStorage::default());
        (storage.clone(), Versions::new(storage))
    }

    #[tokio::test]
    async fn two_activations_at_once_both_count() {
        let (_, versions) = versions();
        versions.activate("x").await.unwrap();
        // The storage double yields in each read and write, so without the lock both calls
        // read {current: x} and the second write drops the first.
        let (a, b) = tokio::join!(versions.activate("a"), versions.activate("b"));
        a.unwrap();
        b.unwrap();
        let state = versions.state().await;
        assert_eq!(state.current.as_deref(), Some("b"));
        assert_eq!(state.previous.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn the_file_keeps_its_format() {
        let (storage, versions) = versions();
        versions.activate("v1").await.unwrap();
        versions.activate("v2").await.unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&storage.file("versions.json").unwrap()).unwrap();
        assert_eq!(saved, json!({ "current": "v2", "previous": "v1" }));
    }

    #[tokio::test]
    async fn removing_the_current_version_makes_the_previous_one_current() {
        let (storage, versions) = versions();
        storage.put("builds/v1/emu", b"");
        storage.put("builds/v2/emu", b"");
        versions.activate("v1").await.unwrap();
        versions.activate("v2").await.unwrap();
        versions.remove("v2").await.unwrap();
        assert_eq!(versions.state().await, State { current: Some("v1".into()), previous: None });
        assert_eq!(versions.installed().await.unwrap(), ["v1"]);
    }
}
