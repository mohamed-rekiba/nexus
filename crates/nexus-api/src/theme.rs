use std::path::PathBuf;

use crate::RelPath;

/// Read-only files: a theme's layouts and assets, or a plugin's assets. The host reads them;
/// plugins and themes never touch the filesystem for them.
pub trait Files: Send + Sync {
    fn read(&self, path: &RelPath) -> Option<Vec<u8>>;

    /// Every file, sorted.
    fn list(&self) -> Vec<RelPath>;

    /// The folder on disk, for themes loaded from one (used for dev-mode reload).
    fn root(&self) -> Option<PathBuf> {
        None
    }
}

/// Files compiled into the binary.
#[derive(Clone, Copy)]
pub struct EmbeddedFiles {
    files: &'static [(&'static str, &'static [u8])],
}

impl EmbeddedFiles {
    /// Every path must be a valid [`RelPath`].
    pub const fn new(files: &'static [(&'static str, &'static [u8])]) -> Self {
        Self { files }
    }
}

impl Files for EmbeddedFiles {
    fn read(&self, path: &RelPath) -> Option<Vec<u8>> {
        self.files.iter().find(|(p, _)| *p == path.as_str()).map(|(_, data)| data.to_vec())
    }

    fn list(&self) -> Vec<RelPath> {
        let mut paths: Vec<RelPath> = self.files.iter().filter_map(|(p, _)| RelPath::new(*p).ok()).collect();
        paths.sort();
        paths
    }
}
