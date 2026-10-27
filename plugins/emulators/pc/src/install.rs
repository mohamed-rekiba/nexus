//! Install: a PC game from a download folder the host has unpacked. The archives that are the
//! release's packaging come from their unpacked folders; everything else, a game's own data
//! archives too, is copied as it is. Files are probed by their first bytes, programs get the
//! program flag again, and the manifest file lists every file that can start the game.

use std::sync::Arc;

use emulator_kit::archive_sets::{UNPACKED, set_name};
use emulator_kit::installs::{self, Download, DownloadFile, HostTree, Tree};
use nexus_api::serde_json;
use nexus_api::*;

use crate::manifest::{GAME_FILE, GameManifest, Start};
use crate::probe::{self, Probe};
use crate::{CONSOLE_FOLDER, GAMES, GOG_INFO, Pc, WRAPPER_INFO, WRAPPER_NAME_LINE, path};

/// The most files of a release whose first bytes are read: the nearest the game's folder, so a
/// start script beside a bundled runtime's thousands of files is read.
const PROBED: usize = 4000;

/// `files`, nearest the game's folder first, at most [`PROBED`]: the ones to read.
pub(crate) fn to_probe<'a>(files: impl IntoIterator<Item = &'a DownloadFile>) -> Vec<&'a DownloadFile> {
    let mut files: Vec<&DownloadFile> = files.into_iter().collect();
    files.sort_by(|a, b| (a.rel.matches('/').count(), &a.rel).cmp(&(b.rel.matches('/').count(), &b.rel)));
    files.truncate(PROBED);
    files
}

/// What a set's folder holds beside the set, among the release's own files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Beside {
    Nothing,
    Script,
    Program,
}

/// The files of a release to install: each from where it is, to its path in the game.
struct Plan {
    name: String,
    files: Vec<DownloadFile>,
    /// The download's unpacked folder that alone is the game, from the download's folder: the
    /// host may move it in whole, which keeps its links and empty folders (a bundled Proton's
    /// libraries and Wine prefix need both).
    whole: Option<String>,
}

/// A script installer's name without its ending (`.sh`, `.run`), at the download's top or one
/// folder down; the host unpacks GOG's and those that say their archive's size.
fn script_stem(rel: &str) -> Option<&str> {
    let lower = rel.to_ascii_lowercase();
    let ext = [".sh", ".run"].into_iter().find(|ext| lower.ends_with(ext))?;
    (rel.matches('/').count() <= 1).then(|| &rel[..rel.len() - ext.len()])
}

/// The one folder all of `paths` are in, without its `/`; `None` when they aren't in one.
fn one_folder<'a>(paths: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut top = None;
    for path in paths {
        let (dir, _) = path.split_once('/')?;
        match top {
            None => top = Some(dir),
            Some(t) if t == dir => {}
            Some(_) => return None,
        }
    }
    top
}

/// The folder all of `files` are in, one level down (a torrent's own folder), or `""`.
fn common_top(files: &[&DownloadFile]) -> String {
    let mut top: Option<&str> = None;
    for f in files {
        let Some((dir, _)) = f.rel.split_once('/') else { return String::new() };
        match top {
            None => top = Some(dir),
            Some(t) if t == dir => {}
            Some(_) => return String::new(),
        }
    }
    top.map(|t| format!("{t}/")).unwrap_or_default()
}

impl Pc {
    /// What the download's file `file` is, from its first bytes.
    async fn probe_file(&self, file: &DownloadFile) -> Probe {
        let bytes = self.ctx.folders().read_range(&file.path, 0, probe::HEAD).await.unwrap_or_default();
        probe::probe(&bytes)
    }

    /// Which files of the download install, and where: the release's own files, with each
    /// archive set that is its packaging replaced by what the host unpacked from it.
    async fn plan(&self, download: &Download) -> PluginResult<Plan> {
        let unpacked_prefix = format!("{UNPACKED}/");
        let own: Vec<&DownloadFile> = download.files.iter().filter(|f| !f.rel.starts_with(&unpacked_prefix)).collect();
        // The sets the host unpacked, by name, with the parts that make them.
        let mut sets: std::collections::BTreeMap<String, Vec<&DownloadFile>> = Default::default();
        for f in &own {
            let name = set_name(&f.rel).or_else(|| script_stem(&f.rel).map(str::to_owned));
            // A set's parts can be spelled in another case: the host names its folder as the
            // first part spells it.
            let folder = name.and_then(|name| {
                let wanted = format!("{unpacked_prefix}{name}/").to_ascii_lowercase();
                download
                    .files
                    .iter()
                    .find(|u| u.rel.to_ascii_lowercase().starts_with(&wanted))
                    .map(|u| u.rel[unpacked_prefix.len()..wanted.len() - 1].to_owned())
            });
            if let Some(folder) = folder {
                sets.entry(folder).or_default().push(f);
            }
        }
        let in_set = |f: &DownloadFile| sets.values().any(|parts| parts.iter().any(|p| p.rel == f.rel));
        // What each folder that holds a set has beside its sets.
        let mut beside: std::collections::BTreeMap<String, Beside> = Default::default();
        for f in own.iter().filter(|f| !in_set(f) && f.rel.matches('/').count() <= 1) {
            let folder = f.rel.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
            if !sets.keys().any(|name| name.rsplit_once('/').map(|(d, _)| d).unwrap_or("") == folder) {
                continue;
            }
            let what = match self.probe_file(f).await {
                probe if probe.is_program() => Beside::Program,
                Probe::Script => Beside::Script,
                _ => continue,
            };
            let best = beside.entry(folder).or_insert(what);
            *best = (*best).max(what);
        }
        let mut packaging = Vec::new();
        for (name, parts) in &sets {
            let folder = name.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
            // An installer is the packaging, whatever is beside it.
            let installer = parts.iter().any(|p| script_stem(&p.rel).is_some());
            let unpack = match beside.get(&folder).copied().unwrap_or(Beside::Nothing) {
                Beside::Nothing => true,
                // The script starts what the archive holds.
                Beside::Script => {
                    let inside = format!("{unpacked_prefix}{name}/");
                    let mut holds_program = false;
                    for f in to_probe(download.files.iter().filter(|f| f.rel.starts_with(&inside))) {
                        if self.probe_file(f).await.is_program() {
                            holds_program = true;
                            break;
                        }
                    }
                    holds_program
                }
                // A game's own data beside its program: it reads the archive as it is.
                Beside::Program => false,
            };
            if installer || unpack {
                packaging.push(name.clone());
            }
        }
        let packed = |f: &DownloadFile| packaging.iter().any(|name| sets[name].iter().any(|p| p.rel == f.rel));
        let kept: Vec<&DownloadFile> = own.iter().copied().filter(|f| !packed(f)).collect();
        // The torrent's own folder is the game's folder.
        let top = common_top(&own);
        let mut files: Vec<DownloadFile> = kept
            .iter()
            .map(|f| DownloadFile { path: f.path.clone(), rel: f.rel[top.len()..].to_owned(), size: f.size })
            .collect();
        // The game's folder an installer's archive holds: its files go at the game's top, and
        // it names the game. A GOG installer's `data/noarch` keeps its shape.
        let mut installer_folder = None;
        let mut whole = None;
        for name in &packaging {
            let inside = format!("{unpacked_prefix}{name}/");
            // An archive's files go where it was, in the game's folder.
            let at = name.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
            let at = at.strip_prefix(&top).unwrap_or(&at).to_owned();
            let unpacked: Vec<&DownloadFile> = download.files.iter().filter(|f| f.rel.starts_with(&inside)).collect();
            let installer = sets[name].iter().any(|p| script_stem(&p.rel).is_some());
            let folder = one_folder(unpacked.iter().map(|f| &f.rel[inside.len()..]))
                .filter(|_| installer && !unpacked.iter().any(|f| f.rel == format!("{inside}{GOG_INFO}")));
            let cut = folder.map_or(inside.len(), |folder| inside.len() + folder.len() + 1);
            if let Some(folder) = folder {
                installer_folder.get_or_insert_with(|| folder.to_owned());
            }
            if packaging.len() == 1 && kept.is_empty() && at.is_empty() {
                whole = Some(folder.map_or(inside.trim_end_matches('/').to_owned(), |f| format!("{inside}{f}")));
            }
            for f in unpacked {
                files.push(DownloadFile { path: f.path.clone(), rel: format!("{at}{}", &f.rel[cut..]), size: f.size });
            }
        }
        // A GOG installer names its game in its `gameinfo` file.
        let mut gog_name = None;
        for name in packaging.iter().filter(|n| sets[*n].iter().any(|p| script_stem(&p.rel).is_some())) {
            let info = format!("{unpacked_prefix}{name}/{GOG_INFO}");
            if let Some(f) = download.files.iter().find(|f| f.rel == info) {
                gog_name = self.named_in(&f.path, 0).await;
                break;
            }
        }
        let title = if let Some(name) = gog_name.or(installer_folder) {
            name
        } else if !top.is_empty() {
            top.trim_end_matches('/').to_owned()
        } else if let Some(name) = packaging.first().or(sets.keys().next()) {
            name.rsplit('/').next().unwrap_or(name).to_owned()
        } else {
            "Game".into()
        };
        Ok(Plan { name: probe::display_name(&title), files, whole })
    }
}

#[async_trait]
impl GameInstaller for Pc {
    /// PC games install from their downloads only.
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.ctx.folders().clone();
        let download = Download::scan(folders.as_ref(), source).await?.ok_or_else(|| {
            PluginError::InvalidInput(format!("{} isn't a download folder: a PC game installs from one", source.path))
        })?;
        let plan = self.plan(&download).await?;
        let mut probed = Vec::new();
        for f in to_probe(&plan.files) {
            probed.push((f.rel.clone(), self.probe_file(f).await));
        }
        let ranked = probe::rank(&probed);
        let Some((first, _)) = ranked.first() else {
            return Err(PluginError::InvalidInput("Nothing in this download starts a game".into()));
        };
        // A portable Wine wrapper names its game beside its start script.
        let script_dir = first.rsplit_once('/').map_or(String::new(), |(dir, _)| format!("{dir}/"));
        let wrapper = format!("{script_dir}{WRAPPER_INFO}");
        let named = match plan.files.iter().find(|f| f.rel == wrapper) {
            Some(f) => self.named_in(&f.path, WRAPPER_NAME_LINE).await,
            None => None,
        };
        let name = named.unwrap_or_else(|| plan.name.clone());
        let into = installs::games_folder(folders.as_ref(), job.as_ref(), CONSOLE_FOLDER).await?;
        let dest = into.join(&probe::folder_name(&name))?;
        if folders.exists(&dest).await? {
            return Err(PluginError::Precondition(format!("{name} is already installed")));
        }
        let work = HostPath::new(GAMES, dest.root, path(format!("{}.part", dest.path))?);
        folders.remove(&work).await?;
        let manifest = GameManifest::new(name, ranked.into_iter().map(|(path, kind)| Start { path, kind }).collect());
        let runnable: Vec<String> = probed.iter().filter(|(_, p)| p.runnable()).map(|(rel, _)| rel.clone()).collect();
        let flags = self.platform != Platform::Windows;
        let whole = plan.whole.as_deref().map(|rel| download.source.join(rel)).transpose()?;
        let mut tree = HostTree::of(folders.clone(), plan.files);
        let into = work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            use emulator_kit::fail::Error;
            let entries = tree.entries(&installs::cancel_check(job))?;
            // Moved in whole when the host allows; copied file by file otherwise.
            if whole.as_ref().is_some_and(|from| job.take(from, &into)) {
                let total = installs::tree_size(&entries, "");
                job.progress(total, total);
            } else {
                installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
            }
            let io = |call: std::pin::Pin<Box<dyn std::future::Future<Output = PluginResult<()>> + Send + '_>>| {
                writer.runtime.block_on(call).map_err(|e| Error::msg(e.to_string()))
            };
            // Downloads drop the program flag.
            if flags {
                for rel in &runnable {
                    let file = into.join(rel).map_err(|e| Error::msg(e.to_string()))?;
                    io(Box::pin(writer.folders.set_executable(&file)))?;
                }
            }
            let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| Error::msg(e.to_string()))?;
            let file = into.join(GAME_FILE).map_err(|e| Error::msg(e.to_string()))?;
            io(Box::pin(writer.folders.write(&file, &bytes)))?;
            writer.rename(&into, &dest)?;
            Ok((dest, manifest.name))
        });
        Ok(work)
    }
}
