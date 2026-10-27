//! Game installs from PS4 packages and unpacked game folders, and what goes with a game: its
//! update (`<folder>-patch` beside it) and its add-ons (`addcont/<title id>` in shadPS4's data
//! folder).

use std::sync::Arc;

use emulator_kit::installs::{self, Entry, Format, Tree, Writer};
use nexus_api::*;

use crate::{CONSOLE, DATA, GAMES, Shad, path, pkg4};

#[async_trait]
impl Catalog for Shad {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        self.emu.list(page).await
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        self.emu.search(query).await
    }

    /// With its update (`<folder>-patch` beside it) and its add-ons (`addcont/<title id>`),
    /// which go when it is uninstalled.
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let folders = &self.emu.host().folders;
        let mut details = self.emu.details(game).await?;
        let mut also = Vec::new();
        if let Some(dir) = &details.folder {
            also.push(HostPath::new(GAMES, dir.root, path(format!("{}-patch", dir.path))?));
        }
        if let (Some(title_id), Some(data)) = (&details.summary.title_id, self.root_of(DATA).await?) {
            also.push(HostPath::new(DATA, data, path(format!("addcont/{title_id}"))?));
        }
        for folder in also {
            if folders.exists(&folder).await.unwrap_or(false) {
                details.also.push(folder);
            }
        }
        Ok(details)
    }

    async fn artwork(&self, game: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        self.emu.artwork(game, kind).await
    }
}

/// A PS4 package's tree.
struct Ps4Package<R: emulator_kit::read_at::ReadAt> {
    package: pkg4::Package<R>,
    nodes: Vec<pkg4::Node>,
}

impl<R: emulator_kit::read_at::ReadAt> Tree for Ps4Package<R> {
    fn entries(
        &mut self,
        cancel: &dyn Fn() -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<Vec<Entry>> {
        self.nodes = self.package.list(cancel)?;
        Ok(self.nodes.iter().map(|n| Entry { path: n.path.clone(), directory: n.directory, size: n.size }).collect())
    }

    fn copy(
        &mut self,
        index: usize,
        sink: &mut dyn FnMut(&[u8]) -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<()> {
        use emulator_kit::fail::Context;
        let node = self.nodes.get(index).context("No such file in the package")?.clone();
        self.package.copy(&node, sink)
    }
}

impl Shad {
    async fn root_of(&self, folder: &str) -> PluginResult<Option<u32>> {
        Ok(self.emu.host().folders.roots(folder).await?.first().map(|r| r.root))
    }
}

#[async_trait]
impl GameInstaller for Shad {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        self.emu.candidates().await
    }

    /// Opens each package to learn what it is (a game, an update or DLC) and where it goes,
    /// then copies them in the background as one job: the game, then its update, then its
    /// DLC. An update needs its game installed first, or in the same download. A download can
    /// also hold a game folder that is already unpacked: it is copied as it is.
    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.emu.host().folders.clone();
        let files = match installs::Download::scan(folders.as_ref(), source).await? {
            None => {
                let head = folders.read_range(source, 0, 512).await?;
                if installs::format(&head) != Some(Format::Ps4Package) {
                    return Err(PluginError::InvalidInput(format!("{} isn't a PS4 package", source.path)));
                }
                vec![(source.clone(), installs::file_size(folders.as_ref(), source).await?)]
            }
            Some(download) => {
                if let Some(work) = self.begin_folder(&download, job.clone()).await? {
                    return Ok(work);
                }
                let found = download.packages(folders.as_ref(), |f| f == Format::Ps4Package).await?;
                if found.is_empty() {
                    return Err(download.nothing_to_install());
                }
                found.into_iter().map(|(f, _)| (f.path, f.size)).collect()
            }
        };
        let mut packages = Vec::new();
        for (path, size) in files {
            let file = emulator_kit::read_at::HostFile::new(folders.clone(), path.clone(), size);
            let package = installs::blocking(move || pkg4::Package::open(file, &|| Ok(())))
                .await
                .map_err(|e| PluginError::InvalidInput(format!("{}: {e}", path.path)))?;
            let root = package.root().map_err(PluginError::from)?;
            packages.push((package, root));
        }
        let count = |kind: pkg4::Kind| packages.iter().filter(|(p, _)| p.kind() == kind).count();
        let (games, updates) = (count(pkg4::Kind::Game), count(pkg4::Kind::Update));
        if games > 1 {
            return Err(PluginError::InvalidInput(format!(
                "This download has {games} games; install one game at a time"
            )));
        }
        if updates > 1 {
            return Err(PluginError::InvalidInput(format!(
                "This download has {updates} updates for one game; it can install only one"
            )));
        }
        // The game first, then its update, then its DLC.
        packages.sort_by_key(|(p, _)| match p.kind() {
            pkg4::Kind::Game => 0,
            pkg4::Kind::Update => 1,
            pkg4::Kind::Dlc => 2,
        });
        let mut steps: Vec<Step> = Vec::new();
        for (package, root) in packages {
            let game = steps.first().filter(|s| s.kind == pkg4::Kind::Game).map(|s| s.dest.clone());
            let kind = package.kind();
            let dest = self.dest(kind, &root, game.as_ref(), job.as_ref()).await?;
            if folders.exists(&dest).await? {
                if kind == pkg4::Kind::Dlc {
                    continue; // an add-on that is installed stays as it is
                }
                return Err(PluginError::Precondition(format!("{} is already installed", dest.path)));
            }
            let work = HostPath::new(&dest.folder, dest.root, path(format!("{}.part", dest.path))?);
            folders.remove(&work).await?;
            steps.push(Step { package, root, kind, dest, work });
        }
        let Some(first) = steps.first() else {
            return Err(PluginError::Precondition("every add-on in this download is already installed".into()));
        };
        let work = first.work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| install_steps(writer, job, steps));
        Ok(work)
    }
}

/// One package of an install, and where it goes.
struct Step {
    package: pkg4::Package<emulator_kit::read_at::HostFile>,
    root: String,
    kind: pkg4::Kind,
    dest: HostPath,
    work: HostPath,
}

/// Copies each step's package into its work folder, then moves it into place, with one
/// progress for them all. On an error the work folders go; what is in place stays.
fn install_steps(
    writer: &Writer,
    job: &dyn InstallJob,
    steps: Vec<Step>,
) -> emulator_kit::fail::Result<(HostPath, String)> {
    let cancel = installs::cancel_check(job);
    let works: Vec<HostPath> = steps.iter().map(|s| s.work.clone()).collect();
    let run = || -> emulator_kit::fail::Result<(HostPath, String)> {
        let mut trees = Vec::new();
        for step in steps {
            let mut tree = Ps4Package { package: step.package, nodes: Vec::new() };
            let entries = tree.entries(&cancel)?;
            trees.push((tree, entries, step.root, step.dest, step.work));
        }
        let total = trees.iter().map(|(_, entries, ..)| installs::tree_size(entries, "")).sum();
        let mut done = 0;
        job.progress(0, total);
        let mut first: Option<(HostPath, String)> = None;
        for (mut tree, entries, root, dest, into) in trees {
            let title = entries
                .iter()
                .position(|e| e.path == "sce_sys/param.sfo")
                .and_then(|at| Writer::read_entry(&mut tree, at, 1 << 20).ok())
                .and_then(|bytes| emulator_kit::sfo::parse(&bytes))
                .and_then(|sfo| sfo.get("TITLE").map(|t| t.text().to_owned()))
                .filter(|t| !t.is_empty())
                .unwrap_or(root);
            installs::write_tree_counted(writer, &mut tree, &entries, "", &into, job, (&mut done, total))?;
            writer.rename(&into, &dest)?;
            first.get_or_insert((dest, title));
        }
        first.ok_or_else(|| emulator_kit::fail::Error::msg("nothing to install"))
    };
    let result = run();
    if result.is_err() {
        for work in works.iter().skip(1) {
            let _ = writer.remove(work);
        }
    }
    result
}

impl Shad {
    /// Where a package of `kind` with `root` goes. An update goes next to its game: `game`
    /// when the same download installs it, else the installed one.
    async fn dest(
        &self,
        kind: pkg4::Kind,
        root: &str,
        game: Option<&HostPath>,
        job: &dyn InstallJob,
    ) -> PluginResult<HostPath> {
        let need = |what: &str, folder: &str| {
            PluginError::Precondition(format!("map your {folder} folder in Settings first: {what} goes there"))
        };
        Ok(match kind {
            pkg4::Kind::Game => {
                if job.games_folder().is_none() && self.root_of(GAMES).await?.is_none() {
                    return Err(need("the game", GAMES));
                }
                installs::games_folder(self.emu.host().folders.as_ref(), job, CONSOLE).await?.join(root)?
            }
            pkg4::Kind::Update => {
                let title_id = root.trim_end_matches("-patch");
                if let Some(game) = game {
                    if game.path.file_name() != Some(title_id) {
                        return Err(PluginError::InvalidInput(format!(
                            "this download's update is for {title_id}, not for its game"
                        )));
                    }
                    return Ok(HostPath::new(GAMES, game.root, path(format!("{}-patch", game.path))?));
                }
                let installed = self.emu.games().refresh(self.emu.host().folders.as_ref()).await?;
                let game = installed.iter().find(|g| g.title_id == title_id).ok_or_else(|| {
                    PluginError::Precondition(format!("this is an update for {title_id}: install the game first"))
                })?;
                let folder = game.dir.path.file_name().unwrap_or(title_id).to_owned();
                let beside = game.dir.path.as_str().rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                HostPath::new(GAMES, game.dir.root, path(format!("{beside}{folder}-patch"))?)
            }
            pkg4::Kind::Dlc => {
                let data = self.root_of(DATA).await?.ok_or_else(|| need("DLC", DATA))?;
                HostPath::new(DATA, data, path(format!("addcont/{root}"))?)
            }
        })
    }

    /// Installs the unpacked game folder in `download`, if it has one: a game's
    /// `sce_sys/param.sfo` (not an update's) with a non-empty `eboot.bin`. `None` when it has
    /// none.
    async fn begin_folder(
        &self,
        download: &installs::Download,
        job: Arc<dyn InstallJob>,
    ) -> PluginResult<Option<HostPath>> {
        let folders = self.emu.host().folders.clone();
        for prefix in download.game_folders("param.sfo") {
            let bytes = download.read(folders.as_ref(), &format!("{prefix}sce_sys/param.sfo"), 1 << 20).await?;
            let Some(sfo) = emulator_kit::sfo::parse(&bytes) else { continue };
            let text = |key: &str| sfo.get(key).map(|v| v.text().to_owned()).unwrap_or_default();
            if !text("CATEGORY").starts_with("gd") || text("TITLE_ID").is_empty() {
                continue;
            }
            let title_id = text("TITLE_ID");
            let title = Some(text("TITLE")).filter(|t| !t.is_empty()).unwrap_or_else(|| title_id.clone());
            let dest = self.dest(pkg4::Kind::Game, &title_id, None, job.as_ref()).await?;
            if folders.exists(&dest).await? {
                return Err(PluginError::Precondition(format!("{title} is already installed")));
            }
            let work = HostPath::new(&dest.folder, dest.root, path(format!("{}.part", dest.path))?);
            folders.remove(&work).await?;
            let mut tree = download.tree(folders.clone(), &prefix);
            let into = work.clone();
            installs::spawn(folders, work.clone(), job, move |writer, job| {
                let entries = tree.entries(&installs::cancel_check(job))?;
                installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
                writer.rename(&into, &dest)?;
                Ok((dest, title))
            });
            return Ok(Some(work));
        }
        Ok(None)
    }
}
