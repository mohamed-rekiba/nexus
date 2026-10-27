//! Game installs from PS5 packages, exFAT images and unpacked game folders.

use std::sync::Arc;

use emulator_kit::games;
use emulator_kit::installs::{self, ExfatImage, Format, Ps5Package, Tree, Writer};
use emulator_kit::read_at::HostFile;
use nexus_api::*;

use crate::{CONSOLE, Kyty};

/// What a game installs from: a package or image file, or an unpacked game folder in a
/// download (its tree and its path in the download).
enum Source {
    File(HostFile, Format),
    Folder(installs::HostTree, String),
}

/// The package or image in `download` that holds the game: the one nearest the top. Update
/// and add-on packages hold no game, so they are skipped. A package that fails to open at or
/// above that depth might be the game, so its error stops the install; two at the same depth
/// are never guessed.
async fn game_package(folders: &Arc<dyn Folders>, download: &installs::Download) -> PluginResult<Source> {
    let mut best: Option<(usize, Vec<String>, HostFile, Format)> = None;
    let mut failure: Option<(usize, PluginError)> = None;
    for (file, format) in
        download.packages(folders.as_ref(), |f| matches!(f, Format::Ps5Package | Format::Exfat)).await?
    {
        let depth = file.depth();
        let reader = HostFile::new(folders.clone(), file.path.clone(), file.size);
        let opened = installs::blocking(move || {
            let mut tree: Box<dyn Tree> = match format {
                Format::Ps5Package => Box::new(Ps5Package::open(reader, &|| Ok(()))?),
                _ => Box::new(ExfatImage::open(reader)?),
            };
            Ok(installs::holds_game(&tree.entries(&|| Ok(()))?))
        })
        .await;
        match opened {
            Ok(true) => match &mut best {
                Some((top, names, ..)) if depth == *top => names.push(file.rel.clone()),
                Some((top, ..)) if depth > *top => (),
                _ => {
                    let reader = HostFile::new(folders.clone(), file.path.clone(), file.size);
                    best = Some((depth, vec![file.rel.clone()], reader, format));
                }
            },
            Ok(false) => (),
            Err(e) => {
                if failure.as_ref().is_none_or(|f| depth < f.0) {
                    failure = Some((depth, PluginError::InvalidInput(format!("{}: {e}", file.rel))));
                }
            }
        }
    }
    if let Some((depth, e)) = failure
        && best.as_ref().is_none_or(|b| depth <= b.0)
    {
        return Err(e);
    }
    let Some((_, names, reader, format)) = best else { return Err(download.nothing_to_install()) };
    if names.len() > 1 {
        return Err(PluginError::InvalidInput(format!(
            "Several packages hold a game ({}); choose a single release folder",
            names.join(", ")
        )));
    }
    Ok(Source::File(reader, format))
}

#[async_trait]
impl GameInstaller for Kyty {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        self.emu.candidates().await
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.emu.host().folders.clone();
        let base = installs::games_folder(folders.as_ref(), job.as_ref(), CONSOLE).await?;
        // A download is a folder: an unpacked game in it, else the package that holds the game.
        let from = match installs::Download::scan(folders.as_ref(), source).await? {
            None => {
                let head = folders.read_range(source, 0, 512).await?;
                match installs::format(&head) {
                    Some(f @ (Format::Ps5Package | Format::Exfat)) => {
                        let size = installs::file_size(folders.as_ref(), source).await?;
                        Source::File(HostFile::new(folders.clone(), source.clone(), size), f)
                    }
                    _ => {
                        return Err(PluginError::InvalidInput(format!(
                            "{} isn't a PS5 package or exFAT image",
                            source.path
                        )));
                    }
                }
            }
            Some(download) => {
                let found = download.game_folders("param.json");
                let depth = |p: &String| p.matches('/').count();
                // Two at the same depth are never guessed, as for packages.
                if let [first, second, ..] = found.as_slice()
                    && depth(first) == depth(second)
                {
                    return Err(PluginError::InvalidInput(format!(
                        "Several folders hold a game ({first}, {second}); choose a single release folder"
                    )));
                }
                match found.first() {
                    Some(prefix) => Source::Folder(download.tree(folders.clone(), prefix), prefix.clone()),
                    None => game_package(&folders, &download).await?,
                }
            }
        };
        let stem = match &from {
            Source::File(file, _) => {
                let name = file.path.path.file_name().unwrap_or("game");
                name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_owned()
            }
            Source::Folder(_, prefix) => {
                let prefix = prefix.trim_end_matches('/');
                prefix.rsplit('/').next().filter(|p| !p.is_empty()).unwrap_or("game").to_owned()
            }
        };
        // An unpacked folder may move rather than be copied, when the host made it.
        let folder = match &from {
            Source::Folder(_, prefix) => Some(source.join(prefix.trim_end_matches('/'))?),
            Source::File(..) => None,
        };
        let work = base.join(&format!("{stem}.part"))?;
        folders.remove(&work).await?;
        let into = work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            use emulator_kit::fail::Context;
            let cancel = installs::cancel_check(job);
            let mut tree: Box<dyn Tree> = match from {
                Source::File(file, Format::Ps5Package) => Box::new(Ps5Package::open(file, &cancel)?),
                Source::File(file, _) => Box::new(ExfatImage::open(file)?),
                Source::Folder(tree, _) => Box::new(tree),
            };
            let entries = tree.entries(&cancel)?;
            let prefix = installs::game_folder(&entries, "param.json")
                .context("This holds no PS5 game: it has no sce_sys/param.json")?;
            let at =
                entries.iter().position(|e| e.path == format!("{prefix}sce_sys/param.json")).context("param.json")?;
            let param = Writer::read_entry(tree.as_mut(), at, 1 << 20)?;
            let (title_id, title) = games::ps5_param(&param).context("Its param.json has no usable title id")?;
            let dest = base.join(&format!("{title_id}-app0")).map_err(emulator_kit::fail::Error::msg)?;
            emulator_kit::ensure!(!writer.exists(&dest)?, "{title} is already installed");
            let total = installs::tree_size(&entries, &prefix);
            if prefix.is_empty() && folder.is_some_and(|f| job.take(&f, &into)) {
                job.progress(total, total);
            } else {
                installs::write_tree(writer, tree.as_mut(), &entries, &prefix, &into, job)?;
            }
            writer.rename(&into, &dest)?;
            Ok((dest, title))
        });
        Ok(work)
    }
}
