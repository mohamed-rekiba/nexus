//! Game installs from Switch game files: a download's `.nsp`, `.nsz`, `.xci`, `.xcz` and `.nup`
//! files, or one such file. Each install goes in a folder of its own, `Switch/<title>`, with
//! the game's update and DLC files beside it. The files are copied as they are, Lichii reads
//! them with the player's keys, but for a solid NSZ: Lichii plays only NSZs compressed in
//! blocks, so a solid one is made back into its NSP ([`crate::nsz`]).

use std::sync::Arc;

use emulator_kit::fail::{Context, Result};
use emulator_kit::games::{is_switch_file, switch_file};
use emulator_kit::installs::{self, DownloadFile, Entry, HostTree, Tree};
use emulator_kit::read_at::HostFile;
use nexus_api::*;

use crate::nsz::Plan;
use crate::{FOLDER, Lichii};

/// The files to install, with each solid NSZ made back into its NSP.
struct SwitchFiles {
    files: HostTree,
    /// Each file that is an NSZ, by index: where its NCZs are read from.
    nsz: Vec<Option<HostFile>>,
    /// Each NSZ's NSP, from the last [`Tree::entries`]; `None` for a file copied as it is.
    plans: Vec<Option<Plan>>,
}

impl SwitchFiles {
    /// Call it from async code: the NSZs are read later, from blocking code.
    fn new(folders: Arc<dyn Folders>, files: Vec<DownloadFile>) -> Self {
        let nsz = files
            .iter()
            .map(|f| {
                let is_nsz = f.rel.to_ascii_lowercase().ends_with(".nsz");
                is_nsz.then(|| HostFile::new(folders.clone(), f.path.clone(), f.size))
            })
            .collect();
        Self { files: HostTree::of(folders, files), nsz, plans: Vec::new() }
    }
}

impl Tree for SwitchFiles {
    fn entries(&mut self, cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>> {
        let mut entries = self.files.entries(cancel)?;
        self.plans.clear();
        for (entry, nsz) in entries.iter_mut().zip(&self.nsz) {
            let plan = match nsz {
                Some(file) => Plan::of(file).with_context(|| format!("Read {}", entry.path))?,
                None => None,
            };
            if let Some(plan) = &plan {
                entry.size = plan.size();
                if plan.all_nca()
                    && let Some(stem) = entry.path.get(..entry.path.len() - 4)
                {
                    entry.path = format!("{stem}.nsp");
                }
            }
            self.plans.push(plan);
        }
        Ok(entries)
    }

    fn copy(&mut self, index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        match (self.plans.get(index), self.nsz.get(index)) {
            (Some(Some(plan)), Some(Some(file))) => plan.write(file, sink, &|| Ok(())),
            _ => self.files.copy(index, sink),
        }
    }
}

/// The file name of `path`.
fn file_name(path: &HostPath) -> String {
    path.path.file_name().unwrap_or_default().to_owned()
}

/// The Switch files to install from `source`, each named by its own file name, and the title of
/// the one game among them. A download with no game, with two games, or with two files of one
/// name in different folders is refused.
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
    let mut names = std::collections::HashSet::new();
    if let Some(twice) = files.iter().find(|f| !names.insert(f.rel.to_ascii_lowercase())) {
        return Err(PluginError::InvalidInput(format!(
            "Two files are named {} in {}; choose a single release folder",
            twice.rel, source.path
        )));
    }
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
        let mut tree = SwitchFiles::new(folders.clone(), files);
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            let entries = tree.entries(&installs::cancel_check(job))?;
            installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
            writer.rename(&into, &dest)?;
            Ok((dest, title))
        });
        Ok(work)
    }
}
