//! The flows of the game files (`game_file.rs`): the library reads each game's file before it
//! asks any store, and writes one into the folder of every game that has none.
//!
//! - At the library's refresh, the files of the games whose folders the records keep are read
//!   first. A game with a file isn't sent to the stores.
//! - When a game's details come in, its folder is kept, and its file is read.
//! - Once the details are read, every game whose folder its plugin may write, and that has no
//!   file, gets one: what the library knows of it, with its art. An install ends with a refresh,
//!   so a game Nexus installs gets its file too, with the offer it came from.
//!
//! A game whose folder can't be found, isn't a folder, or is in a folder its plugin may only
//! read gets no file. A file that can't be written is logged and left for the next run.

use std::path::PathBuf;

use nexus_api::{GameKey, GameMetadata, HostPath, PluginId};

use crate::app::{Core, resolve_game_folder};
use crate::changes::Area;
use crate::game_file::{self, Art, ArtKind, GameFile, Source};
use crate::library::image_ref;

/// Reads the game files of the Library's games whose folders the records keep.
pub(crate) async fn read_files(core: &Core) {
    let folders = core.library().game_folders();
    let games: Vec<(GameKey, HostPath)> = {
        let library = core.library().snapshot();
        library.library().filter_map(|e| Some((e.key.clone(), folders.get(&e.key)?.clone()))).collect()
    };
    for (game, folder) in games {
        read_one(core, &game, &folder).await;
    }
    core.changed(Area::Library);
}

/// The details of `game` came in with its folder: keeps the folder, and reads its game file.
pub(crate) async fn found_folder(core: &Core, game: &GameKey, folder: &HostPath) {
    core.library().keep_folder(game, folder);
    if core.library().snapshot().game_file(game).is_none() {
        read_one(core, game, folder).await;
    }
}

/// Writes a game file for each game of the Library that has none, in a folder its plugin may
/// write.
pub(crate) async fn write_missing(core: &Core) {
    let todo: Vec<(GameKey, HostPath)> = {
        let library = core.library().snapshot();
        library
            .library()
            .filter(|e| library.game_file(&e.key).is_none())
            .filter_map(|e| {
                let folder = library.detail(&e.key).and_then(|d| d.details.folder.clone());
                if folder.is_none() {
                    tracing::debug!(game = %e.key, "no game file: its details name no folder");
                }
                Some((e.key.clone(), folder?))
            })
            .collect()
    };
    for (game, folder) in todo {
        if let Err(e) = write_one(core, &game, &folder).await {
            tracing::warn!(game = %game, error = %e, "could not write a game file");
        }
    }
}

/// Reads the game file of `game` in `folder`, as the game's metadata layer.
async fn read_one(core: &Core, game: &GameKey, folder: &HostPath) {
    let Some(dir) = resolve_game_folder(core.kernel(), game, folder) else { return };
    let file = tokio::task::spawn_blocking(move || game_file::read(&dir)).await.ok().flatten();
    // A file with a plugin's part alone says nothing for the Library yet.
    if let Some(file) = file.filter(GameFile::has_details) {
        core.library().set_game_file(game, Some((game.plugin.clone(), file.metadata(folder))));
    }
}

/// The folder of `game` on disk, when its plugin may write there.
fn writable_dir(core: &Core, game: &GameKey, folder: &HostPath) -> Option<PathBuf> {
    let manifest = core.kernel().manifest(&game.plugin).ok()?;
    manifest.permissions.folder(&folder.folder).filter(|grant| grant.write)?;
    resolve_game_folder(core.kernel(), game, folder)
}

/// Writes the core's part of the game file of `game` into `folder`: what the library knows of
/// it, and its art. Nothing when the folder isn't writable, the file has the core's part
/// already, or the library knows nothing beyond the game's own listing. A plugin's part in the
/// file stays.
async fn write_one(core: &Core, game: &GameKey, folder: &HostPath) -> std::io::Result<()> {
    let Some(dir) = writable_dir(core, game, folder) else {
        tracing::debug!(game = %game, folder = ?folder, "no game file: the folder isn't one its plugin may write");
        return Ok(());
    };
    let on_disk = dir.clone();
    let described = tokio::task::spawn_blocking(move || game_file::read(&on_disk).is_some_and(|f| f.has_details()))
        .await
        .unwrap_or(false);
    if described {
        read_one(core, game, folder).await;
        return Ok(());
    }
    let (about, art, title, platform, title_id, source) = {
        let library = core.library().snapshot();
        let Some(entry) = library.listed(game) else { return Ok(()) };
        let layers = library.metadata(game);
        let about = merge(layers.iter().map(|(_, m)| m));
        let source = library.offer_of(game).and_then(|offer| library.offer(offer)).map(|offer| {
            let snapshot = core.snapshot();
            let release = snapshot.ownership().best_release(offer).id.clone();
            Source { offer: offer.entry.key.to_string(), release }
        });
        let summary = &entry.summary;
        let art = library.art(game);
        let nothing = art.cover.is_none() && art.background.is_none() && art.logo.is_none();
        if layers.is_empty() && source.is_none() && nothing {
            // Only the game's own listing: the stores may answer on a later run.
            tracing::debug!(game = %game, "no game file: nothing is known beyond its listing");
            return Ok(());
        }
        (about, art, summary.title.clone(), summary.platform.clone(), summary.title_id.clone(), source)
    };
    let mut pieces: Vec<(ArtKind, (PluginId, nexus_api::ImageSource))> = Vec::new();
    pieces.extend(art.cover.map(|i| (ArtKind::Cover, i)));
    pieces.extend(art.background.map(|i| (ArtKind::Background, i)));
    pieces.extend(art.logo.map(|i| (ArtKind::Logo, i)));
    for (n, shot) in art.screenshots.into_iter().take(game_file::MAX_SCREENSHOTS).enumerate() {
        pieces.push((ArtKind::Screenshot(n + 1), shot));
    }
    let mut saved = Art::default();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (kind, (plugin, source)) in pieces {
        let bytes = match core.image_bytes(&image_ref(&plugin, &source)).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::info!(game = %game, error = %e, "a picture for a game file didn't load");
                continue;
            }
        };
        let Some(path) = game_file::art_path(kind, &bytes) else { continue };
        match kind {
            ArtKind::Cover => saved.cover = Some(path.clone()),
            ArtKind::Background => saved.background = Some(path.clone()),
            ArtKind::Logo => saved.logo = Some(path.clone()),
            ArtKind::Screenshot(_) => saved.screenshots.push(path.clone()),
        }
        files.push((path, bytes));
    }
    let file = GameFile::new(&title, &platform, title_id.as_deref(), &about, source, saved);
    let layer = file.metadata(folder);
    tokio::task::spawn_blocking(move || game_file::write(&dir, &file, &files))
        .await
        .map_err(std::io::Error::other)??;
    core.library().set_game_file(game, Some((game.plugin.clone(), layer)));
    Ok(())
}

/// One metadata from layers, first layer first: each field from the first layer that has it.
fn merge<'a>(layers: impl Iterator<Item = &'a GameMetadata>) -> GameMetadata {
    let mut merged = GameMetadata::default();
    for layer in layers {
        let text = |into: &mut String, from: &String| {
            if into.is_empty() {
                into.clone_from(from);
            }
        };
        text(&mut merged.title, &layer.title);
        text(&mut merged.description, &layer.description);
        text(&mut merged.tagline, &layer.tagline);
        text(&mut merged.publisher, &layer.publisher);
        text(&mut merged.release, &layer.release);
        text(&mut merged.age_rating, &layer.age_rating);
        text(&mut merged.store_url, &layer.store_url);
        text(&mut merged.trailer, &layer.trailer);
        if merged.genres.is_empty() {
            merged.genres.clone_from(&layer.genres);
        }
        if merged.rating.is_none() {
            merged.rating = layer.rating;
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_field_comes_from_the_first_layer_that_has_it() {
        let store = GameMetadata {
            title: "Store".into(),
            description: "From the store.".into(),
            genres: vec!["Sports".into()],
            ..GameMetadata::default()
        };
        let release = GameMetadata {
            title: "Release".into(),
            publisher: "Nintendo".into(),
            genres: vec!["Party".into()],
            rating: Some(nexus_api::Rating { score: 4.0, total: 9 }),
            ..GameMetadata::default()
        };

        let merged = merge([&store, &release].into_iter());

        assert_eq!(merged.title, "Store");
        assert_eq!(merged.description, "From the store.");
        assert_eq!(merged.publisher, "Nintendo", "the store had none");
        assert_eq!(merged.genres, ["Sports"]);
        assert_eq!(merged.rating, Some(nexus_api::Rating { score: 4.0, total: 9 }));
    }
}
