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

/// A built-in emulator's system pictures as its `Provided.assets`: `assets/icon.png` and
/// `assets/cover.jpg` in the crate that calls it, served as `icon.png` and `cover.jpg`, the names
/// its manifest's `[system]` gives (`scripts/make-system-art.sh` makes them).
#[macro_export]
macro_rules! system_art {
    () => {{
        const FILES: &[(&str, &[u8])] = &[
            ("icon.png", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.png"))),
            ("cover.jpg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cover.jpg"))),
        ];
        let files: std::sync::Arc<dyn $crate::Files> = std::sync::Arc::new($crate::EmbeddedFiles::new(FILES));
        files
    }};
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
