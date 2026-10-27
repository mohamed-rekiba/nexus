//! A plugin's data folder in memory, for the tests of plugins and of the code they share.

use std::collections::BTreeMap;
use std::sync::Mutex;

use nexus_api::{PluginError, PluginResult, RelPath, Storage, async_trait};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A data folder in memory: files by path, folders implied by the files in them. Every call
/// yields once first, so calls running at the same time interleave as on a real disk.
#[derive(Default)]
pub struct MemStorage {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    executable: Mutex<Vec<String>>,
}

fn inside(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

impl MemStorage {
    pub fn put(&self, path: &str, data: &[u8]) {
        lock(&self.files).insert(path.to_owned(), data.to_vec());
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        lock(&self.files).get(path).cloned()
    }

    /// Every file path, sorted.
    pub fn paths(&self) -> Vec<String> {
        lock(&self.files).keys().cloned().collect()
    }

    pub fn is_executable(&self, path: &str) -> bool {
        lock(&self.executable).iter().any(|p| p == path)
    }
}

#[async_trait]
impl Storage for MemStorage {
    async fn read(&self, path: &RelPath) -> PluginResult<Vec<u8>> {
        tokio::task::yield_now().await;
        self.file(path.as_str()).ok_or_else(|| PluginError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &RelPath, data: &[u8]) -> PluginResult<()> {
        tokio::task::yield_now().await;
        self.put(path.as_str(), data);
        Ok(())
    }

    async fn write_at(&self, path: &RelPath, offset: u64, data: &[u8]) -> PluginResult<()> {
        tokio::task::yield_now().await;
        let mut files = lock(&self.files);
        let file = files.entry(path.as_str().to_owned()).or_default();
        let start = usize::try_from(offset).map_err(|e| PluginError::InvalidInput(e.to_string()))?;
        if file.len() < start + data.len() {
            file.resize(start + data.len(), 0);
        }
        file[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    async fn exists(&self, path: &RelPath) -> PluginResult<bool> {
        tokio::task::yield_now().await;
        Ok(lock(&self.files).keys().any(|p| inside(p, path.as_str())))
    }

    async fn list(&self, dir: &RelPath) -> PluginResult<Vec<String>> {
        tokio::task::yield_now().await;
        let prefix = if dir.is_root() { String::new() } else { format!("{dir}/") };
        let names: std::collections::BTreeSet<String> = lock(&self.files)
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter_map(|rest| rest.split('/').next())
            .map(str::to_owned)
            .collect();
        Ok(names.into_iter().collect())
    }

    async fn remove(&self, path: &RelPath) -> PluginResult<()> {
        tokio::task::yield_now().await;
        lock(&self.files).retain(|p, _| !inside(p, path.as_str()));
        Ok(())
    }

    async fn rename(&self, from: &RelPath, to: &RelPath) -> PluginResult<()> {
        tokio::task::yield_now().await;
        let mut files = lock(&self.files);
        let moved: Vec<String> = files.keys().filter(|p| inside(p, from.as_str())).cloned().collect();
        if moved.is_empty() {
            return Err(PluginError::NotFound(from.to_string()));
        }
        for old in moved {
            let data = files.remove(&old).unwrap_or_default();
            files.insert(format!("{to}{}", &old[from.as_str().len()..]), data);
        }
        Ok(())
    }

    async fn set_executable(&self, path: &RelPath) -> PluginResult<()> {
        lock(&self.executable).push(path.to_string());
        Ok(())
    }
}
