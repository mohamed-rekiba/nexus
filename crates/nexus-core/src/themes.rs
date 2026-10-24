//! Files on disk, and data-only themes found on disk, which become plugins like any other.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nexus_api::{Files, Plugin, PluginByKind, PluginKind, PluginManifest, RelPath, Runtime, ThemePlugin};

use crate::Loader;

/// Files in a folder. Reads never leave it: paths are [`RelPath`]s, and symlinks that
/// point outside are refused.
pub struct DirFiles {
    root: PathBuf,
}

impl DirFiles {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn inside(&self, path: &RelPath) -> Option<PathBuf> {
        let root = self.root.canonicalize().ok()?;
        let real = path.under(&self.root).canonicalize().ok()?;
        real.starts_with(&root).then_some(real)
    }
}

impl Files for DirFiles {
    fn read(&self, path: &RelPath) -> Option<Vec<u8>> {
        std::fs::read(self.inside(path)?).ok()
    }

    fn list(&self) -> Vec<RelPath> {
        fn walk(dir: &Path, prefix: &RelPath, out: &mut Vec<RelPath>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Ok(rel) = prefix.join(&name) else { continue };
                match entry.file_type() {
                    Ok(t) if t.is_dir() => walk(&entry.path(), &rel, out),
                    Ok(t) if t.is_file() => out.push(rel),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &RelPath::root(), &mut out);
        out.sort();
        out
    }

    fn root(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
}

/// A theme with no code: it only provides its files.
pub struct DataTheme {
    manifest: PluginManifest,
    files: Arc<dyn Files>,
}

impl DataTheme {
    pub fn new(manifest: PluginManifest, files: Arc<dyn Files>) -> Self {
        Self { manifest, files }
    }
}

#[async_trait]
impl Plugin for DataTheme {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }
}

impl ThemePlugin for DataTheme {
    fn files(&self) -> Arc<dyn Files> {
        self.files.clone()
    }
}

pub(crate) struct DataThemeLoader;

impl Loader for DataThemeLoader {
    fn accepts(&self, manifest: &PluginManifest) -> bool {
        manifest.kind == PluginKind::Theme && manifest.runtime == Runtime::Data
    }

    fn load(&self, manifest: &PluginManifest, dir: &Path) -> Result<PluginByKind, String> {
        Ok(PluginByKind::theme(DataTheme::new(manifest.clone(), Arc::new(DirFiles::new(dir.to_path_buf())))))
    }
}
