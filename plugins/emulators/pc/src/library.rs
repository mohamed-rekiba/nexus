//! The library: the games in `PC/` of the player's game folders that have a manifest file,
//! each named by its folder; their Game Hub details and section.

use nexus_api::serde_json::Value;
use nexus_api::*;

use crate::manifest::{self, GAME_FILE, GameManifest, Start};
use crate::runner::Runner;
use crate::{CONSOLE_FOLDER, GAMES, GOG_INFO, Pc, WRAPPER_INFO, WRAPPER_NAME_LINE, path, probe, settings};

/// An installed game: its folder's name is its id.
#[derive(Clone, Debug)]
pub(crate) struct Game {
    pub id: String,
    pub dir: HostPath,
    pub manifest: GameManifest,
}

impl Game {
    fn summary(&self) -> GameSummary {
        GameSummary {
            id: self.id.clone(),
            title: self.manifest.name.clone(),
            platform: "PC".into(),
            cover: None,
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
        roots.into_iter().map(|root| Ok(HostPath::new(GAMES, root.root, path(CONSOLE_FOLDER)?))).collect()
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
        Some(Game { id: id.to_owned(), dir: game_dir, manifest })
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
                if entry.is_dir
                    && let Some(game) = self.read_game(&dir, &entry.name).await
                {
                    out.push(game);
                }
            }
        }
        out.sort_by_cached_key(|g| g.manifest.name.to_lowercase());
        Ok(out)
    }

    /// The installed game `id`: the first game folder that has it.
    pub(crate) async fn game(&self, id: &str) -> PluginResult<Game> {
        for dir in self.pc_folders().await? {
            if let Some(game) = self.read_game(&dir, id).await {
                return Ok(game);
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
