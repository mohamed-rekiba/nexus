//! Game installs from Switch game files: a download's `.nsp`, `.nsz`, `.xci`, `.xcz` and `.nup`
//! files, or one such file. Each install goes in a folder of its own, `Switch/<title>`, with
//! the game's update and DLC files beside it. The files are copied as they are: Lichii reads
//! them, with the player's keys.

use std::sync::Arc;

use emulator_kit::games::{SWITCH_FILES, switch_file};
use emulator_kit::installs::{self, DownloadFile, HostTree, Tree};
use nexus_api::*;

use crate::{FOLDER, Lichii};

/// Whether `name` is a Switch game file of any kind: a game, an update or DLC.
fn is_switch_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| SWITCH_FILES.iter().any(|s| e.eq_ignore_ascii_case(s)))
}

/// The file name of `path`.
fn file_name(path: &HostPath) -> String {
    path.path.file_name().unwrap_or_default().to_owned()
}

/// The Switch files to install from `source`, each named by its own file name, and the title of
/// the one game among them. A download with no game, or with two games, is refused.
async fn files(folders: &dyn Folders, source: &HostPath) -> PluginResult<(Vec<DownloadFile>, String)> {
    let files: Vec<DownloadFile> = match installs::Download::scan(folders, source).await? {
        Some(download) => download
            .files
            .into_iter()
            .filter(|f| is_switch_file(&f.rel))
            .map(|f| DownloadFile { rel: file_name(&f.path), ..f })
            .collect(),
        None => {
            let size = installs::file_size(folders, source).await?;
            vec![DownloadFile { path: source.clone(), rel: file_name(source), size }]
        }
    };
    let games: Vec<String> = files.iter().filter_map(|f| switch_file(&f.rel).map(|(title, ..)| title)).collect();
    match games.as_slice() {
        [] => Err(PluginError::InvalidInput(format!(
            "{} holds no Switch game file (.nsp, .nsz, .xci, .xcz or .nup) to install",
            source.path
        ))),
        [title] => Ok((files, title.clone())),
        [first, second, ..] => Err(PluginError::InvalidInput(format!(
            "Several files hold a game ({first}, {second}); choose a single release folder"
        ))),
    }
}

#[async_trait]
impl GameInstaller for Lichii {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        self.emu.candidates().await
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.emu.host().folders.clone();
        let (files, title) = files(folders.as_ref(), source).await?;
        let base = installs::games_folder(folders.as_ref(), job.as_ref(), FOLDER).await?;
        let dest = base.join(&title)?;
        if folders.exists(&dest).await? {
            return Err(PluginError::InvalidInput(format!("{title} is already installed")));
        }
        let work = base.join(&format!("{title}.part"))?;
        folders.remove(&work).await?;
        let into = work.clone();
        let mut tree = HostTree::of(folders.clone(), files);
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            let entries = tree.entries(&installs::cancel_check(job))?;
            installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
            writer.rename(&into, &dest)?;
            Ok((dest, title))
        });
        Ok(work)
    }
}
