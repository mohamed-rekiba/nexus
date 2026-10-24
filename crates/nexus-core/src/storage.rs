use std::path::{Path, PathBuf};

use async_trait::async_trait;
use nexus_api::{PluginError, PluginResult, RelPath, Storage};

/// A plugin's data folder. [`RelPath`] keeps every path inside it, and existing paths are also
/// checked after symlinks are resolved.
pub struct DirStorage {
    root: PathBuf,
}

impl DirStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The real path, refusing one that a symlink takes outside the folder.
    pub fn resolve(&self, path: &RelPath) -> PluginResult<PathBuf> {
        let full = path.under(&self.root);
        // Check the deepest part that exists.
        let mut probe = full.as_path();
        while !probe.exists() {
            match probe.parent() {
                Some(parent) => probe = parent,
                None => return Ok(full),
            }
        }
        if !probe.starts_with(&self.root) {
            // Nothing under the root exists yet.
            return Ok(full);
        }
        let root = self.root.canonicalize().map_err(io)?;
        let real = probe.canonicalize().map_err(io)?;
        if real.starts_with(&root) {
            Ok(full)
        } else {
            Err(PluginError::PermissionDenied(format!("{path} leaves the plugin's data folder")))
        }
    }
}

fn io(e: std::io::Error) -> PluginError {
    PluginError::Storage(e.to_string())
}

#[async_trait]
impl Storage for DirStorage {
    async fn read(&self, path: &RelPath) -> PluginResult<Vec<u8>> {
        let full = self.resolve(path)?;
        tokio::fs::read(&full).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => PluginError::NotFound(path.to_string()),
            _ => io(e),
        })
    }

    async fn write(&self, path: &RelPath, data: &[u8]) -> PluginResult<()> {
        if path.is_root() {
            return Err(PluginError::InvalidInput("can't write to the data folder itself".into()));
        }
        let full = self.resolve(path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        tokio::fs::write(&full, data).await.map_err(io)
    }

    async fn write_at(&self, path: &RelPath, offset: u64, data: &[u8]) -> PluginResult<()> {
        use tokio::io::{AsyncSeekExt, AsyncWriteExt};
        if path.is_root() {
            return Err(PluginError::InvalidInput("can't write to the data folder itself".into()));
        }
        if data.len() as u64 > nexus_api::MAX_FOLDER_READ {
            return Err(PluginError::InvalidInput(format!(
                "write at most {} bytes at a time",
                nexus_api::MAX_FOLDER_READ
            )));
        }
        let full = self.resolve(path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let mut file =
            tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&full).await.map_err(io)?;
        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(io)?;
        file.write_all(data).await.map_err(io)?;
        file.flush().await.map_err(io)
    }

    async fn exists(&self, path: &RelPath) -> PluginResult<bool> {
        Ok(self.resolve(path)?.exists())
    }

    async fn list(&self, dir: &RelPath) -> PluginResult<Vec<String>> {
        let full = self.resolve(dir)?;
        let mut names = Vec::new();
        let mut entries = match tokio::fs::read_dir(&full).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(io(e)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    async fn remove(&self, path: &RelPath) -> PluginResult<()> {
        if path.is_root() {
            return Err(PluginError::InvalidInput("can't remove the data folder itself".into()));
        }
        let full = self.resolve(path)?;
        let result = match tokio::fs::symlink_metadata(&full).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io(e)),
            Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(&full).await,
            Ok(_) => tokio::fs::remove_file(&full).await,
        };
        result.map_err(io)
    }

    async fn rename(&self, from: &RelPath, to: &RelPath) -> PluginResult<()> {
        let (from, to) = (self.resolve(from)?, self.resolve(to)?);
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        tokio::fs::rename(from, to).await.map_err(io)
    }

    async fn set_executable(&self, path: &RelPath) -> PluginResult<()> {
        let full = self.resolve(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&full).await.map_err(io)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            tokio::fs::set_permissions(&full, perms).await.map_err(io)?;
        }
        #[cfg(not(unix))]
        let _ = full;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_symlink_out_of_the_folder_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"x").unwrap();
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let storage = DirStorage::new(root.path().to_path_buf());

        let err = storage.read(&RelPath::new("link/secret").unwrap()).await.unwrap_err();
        assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
    }

    #[tokio::test]
    async fn files_round_trip_and_removal_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let storage = DirStorage::new(root.path().to_path_buf());
        let p = RelPath::new("a/b/c.txt").unwrap();
        storage.write(&p, b"hello").await.unwrap();
        assert_eq!(storage.read(&p).await.unwrap(), b"hello");
        assert_eq!(storage.list(&RelPath::new("a/b").unwrap()).await.unwrap(), vec!["c.txt"]);
        storage.remove(&RelPath::new("a").unwrap()).await.unwrap();
        storage.remove(&RelPath::new("a").unwrap()).await.unwrap();
        assert!(!storage.exists(&p).await.unwrap());
    }
}
