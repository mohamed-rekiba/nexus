//! The disks the catalogs' downloads go to: each console's download folder and the free space
//! on the disk that holds it. A Game Hub says when a release doesn't fit.
//!
//! The view reads these on every render, so they are read ahead: [`refresh`] runs when a Game
//! Hub opens, the catalogs are read, a folder changes and a download starts or ends, and keeps
//! the answer in the core's state.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use nexus_api::PluginId;

use crate::Core;
use crate::changes::Area;

/// Where a console's downloads go, and the free space there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadFolder {
    /// The console's folder in the emulator's packages folder. It may not be there yet.
    pub path: PathBuf,
    /// The free bytes on the disk that holds it; `None` when the system doesn't say.
    pub free: Option<u64>,
}

/// Each console's download folder, by the emulator and the console a catalog's offers name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Disks {
    folders: Arc<BTreeMap<(PluginId, String), DownloadFolder>>,
}

impl Disks {
    /// Where `emulator`'s downloads of `console`'s games go; `None` when it has no packages
    /// folder, or the catalogs offered none of its games when the disks were last read.
    pub fn download_folder(&self, emulator: &PluginId, console: &str) -> Option<&DownloadFolder> {
        self.folders.get(&(emulator.clone(), console.to_owned()))
    }
}

/// Reads the download folder of every emulator and console the catalogs offer, with the free
/// space there, and keeps them. Says so when they changed.
pub(crate) fn refresh(core: &Core) {
    let disks = read(core);
    let changed = {
        let mut state = core.state();
        let changed = state.disks != disks;
        state.disks = disks;
        changed
    };
    if changed {
        core.changed(Area::Hardware);
    }
}

fn read(core: &Core) -> Disks {
    let library = core.library().snapshot();
    let mut folders = BTreeMap::new();
    for offer in library.offers.iter() {
        let console = &offer.releases[0].platform;
        let at = (offer.emulator.clone(), console.clone());
        if folders.contains_key(&at) {
            continue;
        }
        // The folder of a download in it, less the download's own folder.
        let Ok((_, download)) = core.torrent_folder(&offer.emulator, console, "x", None) else { continue };
        let Some(path) = download.parent().map(std::path::Path::to_path_buf) else { continue };
        // The console's folder may not be there yet: the disk that holds it is the one the
        // nearest folder above is on.
        let free = path.ancestors().find(|p| p.is_dir()).and_then(|there| core.kernel().hardware().free_bytes(there));
        folders.insert(at, DownloadFolder { path, free });
    }
    Disks { folders: Arc::new(folders) }
}
