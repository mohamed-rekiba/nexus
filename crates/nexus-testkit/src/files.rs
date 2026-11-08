//! Files held in memory.

use nexus_api::{Files, RelPath};

/// Files held in memory: a plugin's assets in a test.
pub(crate) struct MemFiles(Vec<(RelPath, Vec<u8>)>);

impl MemFiles {
    /// Every path must be a valid [`RelPath`].
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = (&'a str, Vec<u8>)>) -> Self {
        Self(files.into_iter().map(|(path, bytes)| (RelPath::new(path).expect("a valid path"), bytes)).collect())
    }
}

impl Files for MemFiles {
    fn read(&self, path: &RelPath) -> Option<Vec<u8>> {
        self.0.iter().find(|(p, _)| p == path).map(|(_, bytes)| bytes.clone())
    }

    fn list(&self) -> Vec<RelPath> {
        let mut paths: Vec<RelPath> = self.0.iter().map(|(p, _)| p.clone()).collect();
        paths.sort();
        paths
    }
}
