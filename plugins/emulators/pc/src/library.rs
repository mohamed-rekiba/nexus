//! The library: the games in `PC/` of the player's game folders, each by its folder; their
//! Game Hub details and section.
//!
//! A game installed here has a manifest file. A folder the player copied in by hand gets one
//! the first time the library sees it: its files are probed as an install probes them. A
//! folder with nothing that starts a game is left as it is, and isn't probed again until the
//! next run. A `cover.jpg`, `cover.png` or `cover.webp` in a game's folder is its cover.

use nexus_api::serde_json::Value;
use nexus_api::*;

use emulator_kit::installs::Download;

use crate::manifest::{self, GAME_FILE, GameManifest, Start};
use crate::runner::Runner;
use crate::{CONSOLE_FOLDER, GAMES, GOG_INFO, Pc, WRAPPER_INFO, WRAPPER_NAME_LINE, install, probe, settings};

/// A game's own cover, in its folder, in this order.
const COVERS: [&str; 3] = ["cover.jpg", "cover.png", "cover.webp"];
/// Where a game GOG's installer installed names itself, on its first line.
const GOG_GAME_INFO: &str = "gameinfo";

/// Whether the folder `name` in `PC/` is never a game: a hidden one, or one an install is still
/// writing (`<name>.part`).
fn skipped(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(".part")
}

/// An installed game: its folder's name is its id.
#[derive(Clone, Debug)]
pub(crate) struct Game {
    pub id: String,
    pub dir: HostPath,
    pub manifest: GameManifest,
    /// Its own cover, from its folder.
    pub cover: Option<HostPath>,
}

impl Game {
    fn summary(&self) -> GameSummary {
        GameSummary {
            id: self.id.clone(),
            title: self.manifest.name.clone(),
            platform: "PC".into(),
            cover: self.cover.clone().map(ImageSource::Folder),
            tags: Vec::new(),
            release_year: None,
            title_id: None,
            content_id: None,
        }
    }
}

impl Pc {
    /// The `PC/` folder of each of the player's game folders.
    async fn pc_folders(&self) -> PluginResult<Vec<HostPath>> {
        let roots = self.ctx.folders().roots(GAMES).await?;
        roots.into_iter().map(|root| Ok(HostPath::new(GAMES, root.root, RelPath::new(CONSOLE_FOLDER)?))).collect()
    }

    /// The game in the folder `dir` named `id`, when it has a manifest file.
    async fn read_game(&self, dir: &HostPath, id: &str) -> Option<Game> {
        let game_dir = dir.join(id).ok()?;
        let file = game_dir.join(GAME_FILE).ok()?;
        let bytes = self.ctx.folders().read_range(&file, 0, manifest::MAX_GAME_FILE).await.ok()?;
        let mut manifest = GameManifest::read(&bytes)?;
        // Installed before its name came from GOG's or the wrapper's file: named after the
        // installer or the release's folder. A newer install has the name already.
        if manifest.version == 1
            && let Some(name) = self.old_install_name(&game_dir, &manifest).await
        {
            manifest.name = name;
        }
        let cover = self.own_cover(&game_dir).await;
        Some(Game { id: id.to_owned(), dir: game_dir, manifest, cover })
    }

    /// The first of [`COVERS`] in the folder `dir`.
    async fn own_cover(&self, dir: &HostPath) -> Option<HostPath> {
        for name in COVERS {
            let file = dir.join(name).ok()?;
            if self.ctx.folders().exists(&file).await.unwrap_or(false) {
                return Some(file);
            }
        }
        None
    }

    /// The game a player copied into the folder `id` of `dir` by hand: its files probed as an
    /// install probes them, and its manifest file written. `None` when nothing in it starts a
    /// game; the folder isn't probed again this run.
    async fn adopt(&self, dir: &HostPath, id: &str) -> Option<Game> {
        let key = format!("{}/{}", dir.root, id);
        let not_games = || self.not_games.lock().unwrap_or_else(|p| p.into_inner());
        if not_games().contains(&key) {
            return None;
        }
        let game_dir = dir.join(id).ok()?;
        let Some((manifest, runnable)) = self.probe_folder(&game_dir, id).await else {
            not_games().insert(key);
            return None;
        };
        let folders = self.ctx.folders();
        // A copy can lose the program flag, as a download does. Without it the game says so
        // when it starts.
        if self.platform != Platform::Windows {
            for rel in &runnable {
                let flagged = match game_dir.join(rel) {
                    Ok(file) => folders.set_executable(&file).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = flagged {
                    self.ctx.log().warn(&format!("{rel} in {id} couldn't be made a program: {e}"));
                }
            }
        }
        let written = match serde_json::to_vec_pretty(&manifest) {
            Ok(bytes) => folders.write(&game_dir.join(GAME_FILE).ok()?, &bytes).await,
            Err(e) => Err(PluginError::Other(e.to_string())),
        };
        if let Err(e) = written {
            self.ctx.log().warn(&format!("{id} couldn't be added: {e}"));
            return None;
        }
        self.ctx.log().info(&format!("added {} from the folder {id}", manifest.name));
        let cover = self.own_cover(&game_dir).await;
        Some(Game { id: id.to_owned(), dir: game_dir, manifest, cover })
    }

    /// What starts the game in `game_dir`, and its name: GOG's `gameinfo`, a portable Wine
    /// wrapper's, else the folder's `id`. Also the files that should be programs.
    async fn probe_folder(&self, game_dir: &HostPath, id: &str) -> Option<(GameManifest, Vec<String>)> {
        let folders = self.ctx.folders();
        let files = Download::scan(folders.as_ref(), game_dir).await.ok()??.files;
        let mut probed = Vec::new();
        for file in install::to_probe(&files) {
            let head = folders.read_range(&file.path, 0, probe::HEAD).await.unwrap_or_default();
            probed.push((file.rel.clone(), probe::probe(&head)));
        }
        let ranked = probe::rank(&probed);
        let (first, _) = ranked.first()?;
        let script_dir = first.rsplit_once('/').map_or(String::new(), |(dir, _)| format!("{dir}/"));
        let name = match self.named_in(&game_dir.join(GOG_GAME_INFO).ok()?, 0).await {
            Some(name) => name,
            None => self
                .named_in(&game_dir.join(&format!("{script_dir}{WRAPPER_INFO}")).ok()?, WRAPPER_NAME_LINE)
                .await
                .unwrap_or_else(|| probe::display_name(id)),
        };
        let runnable = probed.iter().filter(|(_, p)| p.runnable()).map(|(rel, _)| rel.clone()).collect();
        let starts = ranked.into_iter().map(|(path, kind)| Start { path, kind }).collect();
        Some((GameManifest::new(name, starts), runnable))
    }

    /// The name GOG's `gameinfo` or the wrapper's `game_info.txt` gives a game installed with
    /// the first manifest.
    async fn old_install_name(&self, game_dir: &HostPath, manifest: &GameManifest) -> Option<String> {
        if let Some(name) = self.named_in(&game_dir.join(GOG_INFO).ok()?, 0).await {
            return Some(name);
        }
        let first = &manifest.starts[0].path;
        let script_dir = first.rsplit_once('/').map_or(String::new(), |(dir, _)| format!("{dir}/"));
        self.named_in(&game_dir.join(&format!("{script_dir}{WRAPPER_INFO}")).ok()?, WRAPPER_NAME_LINE).await
    }

    /// Every installed game, in name order.
    pub(crate) async fn games(&self) -> PluginResult<Vec<Game>> {
        let mut out = Vec::new();
        for dir in self.pc_folders().await? {
            for entry in self.ctx.folders().list(&dir).await.unwrap_or_default() {
                if !entry.is_dir || skipped(&entry.name) {
                    continue;
                }
                let game = match self.read_game(&dir, &entry.name).await {
                    Some(game) => Some(game),
                    None => self.adopt(&dir, &entry.name).await,
                };
                out.extend(game);
            }
        }
        out.sort_by_cached_key(|g| g.manifest.name.to_lowercase());
        Ok(out)
    }

    /// The installed game `id`: the first game folder that has it.
    pub(crate) async fn game(&self, id: &str) -> PluginResult<Game> {
        let dirs = self.pc_folders().await?;
        for dir in &dirs {
            if let Some(game) = self.read_game(dir, id).await {
                return Ok(game);
            }
        }
        if !skipped(id) {
            for dir in &dirs {
                if let Some(game) = self.adopt(dir, id).await {
                    return Ok(game);
                }
            }
        }
        Err(PluginError::NotFound(id.into()))
    }

    /// What Play starts for `game`: its Starts setting, else the manifest's default.
    pub(crate) async fn start(&self, game: &Game) -> Start {
        let config = self.ctx.config().get(&ConfigScope::Game(game.id.clone())).await.unwrap_or_default();
        let chosen = config.get(settings::STARTS).and_then(Value::as_str);
        game.manifest.start(chosen, self.platform).clone()
    }

    /// The name the file at `file` gives its game on its line `line` (from 0): the first of a
    /// GOG installer's `gameinfo`, the sixth of a portable Wine wrapper's `game_info.txt`.
    pub(crate) async fn named_in(&self, file: &HostPath, line: usize) -> Option<String> {
        let bytes = self.ctx.folders().read_range(file, 0, 4096).await.ok()?;
        let named = String::from_utf8_lossy(&bytes).lines().nth(line)?.trim().to_owned();
        Some(probe::clean_name(&named)).filter(|n| !n.is_empty())
    }
}

#[async_trait]
impl Catalog for Pc {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games().await?;
        let total = all.len() as u32;
        let items = all.iter().skip(page.offset as usize).take(page.limit as usize).map(Game::summary).collect();
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let text = query.text.to_lowercase();
        Ok(self
            .games()
            .await?
            .iter()
            .filter(|g| g.manifest.name.to_lowercase().contains(&text))
            .take(query.limit as usize)
            .map(Game::summary)
            .collect())
    }

    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let game = self.game(game).await?;
        let start = self.start(&game).await;
        let size = self.ctx.folders().size(&game.dir).await.unwrap_or(0);
        Ok(GameDetails {
            summary: game.summary(),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: (size > 0).then_some(size),
            background: None,
            facts: vec![GameFact::new("starts", "Starts", &start.path)],
            folder: Some(game.dir),
            also: Vec::new(),
            uninstall_link: None,
        })
    }

    async fn artwork(&self, _game: &str, _kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Slots for Pc {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        if request.slot == SlotId::EmulatorPanel {
            return Ok(self.windows_panel().await);
        }
        let (SlotId::GameDetailSection, Some(id)) = (&request.slot, request.game.as_deref()) else {
            return Ok(Vec::new());
        };
        let game = self.game(id).await?;
        let start = self.start(&game).await;
        let kind = start.kind;
        let runs = if !kind.runs_on(self.platform) {
            kind.not_here(self.platform)
        } else if kind.needs_runner(self.platform) {
            match self.runner().await {
                Some(Runner { umu: true, .. }) => "With Proton, through umu-launcher".into(),
                Some(_) => "With Wine".into(),
                None => self.runner_hint().into(),
            }
        } else {
            "As it is".into()
        };
        let rows = vec![
            SectionRow { label: "Starts".into(), value: start.path },
            SectionRow { label: "Runs".into(), value: runs },
        ];
        let content = SlotContent::Section { title: "PC".into(), rows };
        Ok(vec![SlotItem { id: "pc".into(), order: Order::default(), content }])
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        if action == "install-umu" {
            return self.install_umu().await;
        }
        Err(PluginError::Unsupported(format!("no action {action}")))
    }
}
