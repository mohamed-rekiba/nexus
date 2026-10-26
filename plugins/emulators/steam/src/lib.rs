//! Steam games on Linux: the games Steam installed, from its app manifests, with Steam's art,
//! played through Steam.
//!
//! - Library: each folder of the `steam` grant (Steam's own, and its libraries on other drives)
//!   has `steamapps/appmanifest_<id>.acf` for each game. A fully installed game shows; Steam's
//!   tools (Proton, the Steam Linux Runtime) don't.
//! - Art: from Steam's cache (`appcache/librarycache`), else from Steam's image server.
//! - Play: `steam -silent steam://rungameid/<id>`, which hands the game to Steam's client. A
//!   game in the Flatpak's folder, or any game when only a Flatpak Steam is there, starts with
//!   `flatpak run com.valvesoftware.Steam …`. Steam starts it under
//!   `reaper SteamLaunch AppId=<id> -- …`, and the host follows that process
//!   (`LaunchSpec::game_process`).

mod appinfo;
mod owned;
mod vdf;

use std::collections::BTreeSet;

use nexus_api::serde_json::Value;
use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");
/// The link that asks Steam to uninstall a game: its uninstall dialog.
const UNINSTALL: &str = "steam://uninstall/";
/// What the Library calls the system of Steam's games: their own, beside PC, PS5 and PS4.
const PLATFORM: &str = "Steam";
/// The folder grant: Steam's folder and its libraries.
const STEAM: &str = "steam";
/// Where a Flatpak Steam keeps its folder: a game there starts through the Flatpak.
const FLATPAK_FOLDER: &str = "/.var/app/com.valvesoftware.Steam/";
const FLATPAK_APP: &str = "com.valvesoftware.Steam";
/// Steam's image server, for art its cache doesn't have.
const ART: &str = "https://cdn.cloudflare.steamstatic.com/steam/apps";
/// Steam's art files for a game: its cover, the Game Hub's background, its logo.
const COVER: &str = "library_600x900.jpg";
const HERO: &str = "library_hero.jpg";
const LOGO: &str = "logo.png";
/// The most bytes of an app manifest or library list read.
const MAX_FILE: u64 = 256 * 1024;
/// `StateFlags`' bit for a game whose files are all there.
const FULLY_INSTALLED: u64 = 4;
/// Steamworks Common Redistributables: a tool, whatever its name.
const REDISTRIBUTABLES: &str = "228980";

fn path(p: impl Into<String>) -> PluginResult<RelPath> {
    RelPath::new(p).map_err(|e| PluginError::InvalidInput(e.to_string()))
}

/// Whether the app is one of Steam's tools, not a game.
fn is_tool(id: &str, name: &str) -> bool {
    id == REDISTRIBUTABLES || name.starts_with("Proton") || name.starts_with("Steam Linux Runtime")
}

pub struct SteamPlugin;

#[async_trait]
impl Plugin for SteamPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl EmulatorPlugin for SteamPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let steam = std::sync::Arc::new(Steam { ctx });
        Ok(Provided {
            catalog: Some(steam.clone()),
            launcher: Some(steam.clone()),
            releases: Some(steam.clone()),
            slots: Some(steam),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::emulator(|| Box::new(SteamPlugin)) }

struct Steam {
    ctx: PluginContext,
}

/// A game Steam installed.
#[derive(Clone, Debug)]
struct Game {
    /// Its Steam app id.
    id: String,
    name: String,
    /// Its folder, `steamapps/common/<installdir>` in its library.
    dir: HostPath,
    size: Option<u64>,
    /// Its library is a Flatpak Steam's.
    flatpak: bool,
}

/// One folder of the `steam` grant.
struct Library {
    root: u32,
    /// Its real path.
    path: String,
}

impl Steam {
    async fn libraries(&self) -> PluginResult<Vec<Library>> {
        let roots = self.ctx.folders().roots(STEAM).await?;
        Ok(roots.into_iter().map(|r| Library { root: r.root, path: r.path }).collect())
    }

    /// A file's keys, `None` when it isn't there or isn't Valve's text.
    async fn read_vdf(&self, file: &HostPath) -> Option<vdf::Block> {
        let bytes = self.ctx.folders().read_range(file, 0, MAX_FILE).await.ok()?;
        vdf::parse(&String::from_utf8_lossy(&bytes))
    }

    /// Every installed game of every library, by name; a game in two of them (Steam's folder
    /// both as itself and as `~/.steam/steam`) once.
    async fn games(&self) -> PluginResult<Vec<Game>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for library in self.libraries().await? {
            let apps = HostPath::new(STEAM, library.root, path("steamapps")?);
            for entry in self.ctx.folders().list(&apps).await.unwrap_or_default() {
                if !(entry.name.starts_with("appmanifest_") && entry.name.ends_with(".acf")) {
                    continue;
                }
                let Some(file) = self.read_vdf(&apps.join(&entry.name)?).await else { continue };
                let Some(app) = file.block("AppState") else { continue };
                let (Some(id), Some(name), Some(dir)) = (app.text("appid"), app.text("name"), app.text("installdir"))
                else {
                    continue;
                };
                let flags = app.text("StateFlags").and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
                if flags & FULLY_INSTALLED == 0 || is_tool(id, name) || !seen.insert(id.to_owned()) {
                    continue;
                }
                out.push(Game {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    dir: apps.join(&format!("common/{dir}"))?,
                    size: app.text("SizeOnDisk").and_then(|s| s.parse().ok()).filter(|s| *s > 0),
                    flatpak: library.path.contains(FLATPAK_FOLDER),
                });
            }
        }
        out.sort_by_cached_key(|g| g.name.to_lowercase());
        Ok(out)
    }

    async fn game(&self, id: &str) -> PluginResult<Game> {
        let games = self.games().await?;
        games.into_iter().find(|g| g.id == id).ok_or_else(|| PluginError::NotFound(id.into()))
    }

    /// Steam's art `file` ([`COVER`], [`HERO`], [`LOGO`]) for game `id`: from a library's cache, as Steam
    /// keeps it since 2024 (`librarycache/<id>/…`, at times one folder deeper) or before
    /// (`librarycache/<id>_…`), else from Steam's server.
    async fn art(&self, id: &str, file: &str) -> PluginResult<ImageSource> {
        let folders = self.ctx.folders();
        for library in self.libraries().await? {
            let cache = HostPath::new(STEAM, library.root, path("appcache/librarycache")?);
            let own = cache.join(id)?;
            let mut places = vec![own.join(file)?, cache.join(&format!("{id}_{file}"))?];
            for entry in folders.list(&own).await.unwrap_or_default() {
                if entry.is_dir {
                    places.push(own.join(&format!("{}/{file}", entry.name))?);
                }
            }
            for place in places {
                if folders.exists(&place).await.unwrap_or(false) {
                    return Ok(ImageSource::Folder(place));
                }
            }
        }
        Ok(ImageSource::Url(format!("{ART}/{id}/{file}")))
    }

    async fn summary(&self, game: &Game) -> PluginResult<GameSummary> {
        Ok(GameSummary {
            id: game.id.clone(),
            title: game.name.clone(),
            platform: PLATFORM.into(),
            cover: Some(self.art(&game.id, COVER).await?),
            tags: Vec::new(),
            release_year: None,
            // The app id: the Catalog's Steam game of the same id is this one, installed.
            title_id: Some(game.id.clone()),
            content_id: None,
        })
    }

    /// The plugin page's "Steam libraries": each library Steam lists, and whether its games
    /// show here.
    async fn libraries_panel(&self) -> PluginResult<Vec<SlotItem>> {
        let libraries = self.libraries().await?;
        let read: BTreeSet<&str> = libraries.iter().map(|l| l.path.trim_end_matches('/')).collect();
        let mut rows = Vec::new();
        for library in &libraries {
            let list = HostPath::new(STEAM, library.root, path("steamapps/libraryfolders.vdf")?);
            let Some(file) = self.read_vdf(&list).await else { continue };
            let Some(folders) = file.block("libraryfolders") else { continue };
            for (_, folder) in &folders.0 {
                let vdf::Value::Block(folder) = folder else { continue };
                let Some(path) = folder.text("path") else { continue };
                if rows.iter().any(|r: &SectionRow| r.label == path) {
                    continue;
                }
                let value = if read.contains(path.trim_end_matches('/')) {
                    "Read".into()
                } else {
                    "Add it to Steam libraries in Settings to see its games".into()
                };
                rows.push(SectionRow { label: path.into(), value });
            }
        }
        if rows.is_empty() {
            let value =
                if libraries.is_empty() { "Steam isn't installed, or its folder isn't found" } else { "None yet" };
            rows.push(SectionRow { label: "Libraries".into(), value: value.into() });
        }
        let content = SlotContent::Section { title: "Steam libraries".into(), rows };
        Ok(vec![SlotItem { id: "libraries".into(), order: Order::default(), content }])
    }
}

#[async_trait]
impl Catalog for Steam {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games().await?;
        let total = all.len() as u32;
        let mut items = Vec::new();
        for game in all.iter().skip(page.offset as usize).take(page.limit as usize) {
            items.push(self.summary(game).await?);
        }
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let text = query.text.to_lowercase();
        let mut out = Vec::new();
        for game in self.games().await?.iter().filter(|g| g.name.to_lowercase().contains(&text)) {
            if out.len() >= query.limit as usize {
                break;
            }
            out.push(self.summary(game).await?);
        }
        Ok(out)
    }

    async fn details(&self, id: &str) -> PluginResult<GameDetails> {
        let game = self.game(id).await?;
        Ok(GameDetails {
            summary: self.summary(&game).await?,
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: game.size,
            background: Some(self.art(&game.id, HERO).await?),
            facts: Vec::new(),
            folder: Some(game.dir),
            also: Vec::new(),
            // Steam removes its games: Uninstall asks it to, as Steam's own menu does.
            uninstall_link: Some(format!("{UNINSTALL}{}", game.id)),
        })
    }

    async fn artwork(&self, id: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        let file = match kind {
            ArtworkKind::Cover => COVER,
            ArtworkKind::Background => HERO,
            ArtworkKind::Logo => LOGO,
            ArtworkKind::Icon => return Ok(None),
        };
        Ok(Some(self.art(id, file).await?))
    }
}

#[async_trait]
impl Releases for Steam {
    /// The player's Steam library for the Catalog: the games in their Steam collections.
    async fn releases(&self, console: &Console) -> PluginResult<Option<ReleaseList>> {
        if !console.as_str().eq_ignore_ascii_case(PLATFORM) {
            return Ok(None);
        }
        Ok(Some(self.library_releases().await?))
    }
}

#[async_trait]
impl Launcher for Steam {
    async fn prepare(&self, id: &str, _config: &Value) -> PluginResult<LaunchSpec> {
        let game = self.game(id).await?;
        let url = format!("steam://rungameid/{}", game.id);
        // A Flatpak Steam's library on another drive isn't in the Flatpak's folder: without
        // Steam's own command, a Flatpak Steam starts it.
        let flatpak = game.flatpak || {
            let native = matches!(self.ctx.programs().find(&Location::System("steam".into())).await, Ok(Some(_)));
            !native && self.libraries().await?.iter().any(|l| l.path.contains(FLATPAK_FOLDER))
        };
        let (program, args) = if flatpak {
            ("flatpak", vec!["run".into(), FLATPAK_APP.into(), url.into()])
        } else {
            // -silent: a Steam that wasn't running starts without its window.
            ("steam", vec!["-silent".into(), url.into()])
        };
        let program = Location::System(program.into());
        if !matches!(self.ctx.programs().find(&program).await, Ok(Some(_))) {
            return Err(PluginError::Precondition("Install Steam to play this game".into()));
        }
        Ok(LaunchSpec {
            program,
            args,
            env: Default::default(),
            cwd: Some(Location::Folder(game.dir)),
            // Steam's reaper runs the game, and ends with it.
            game_process: Some(format!("SteamLaunch AppId={} ", game.id)),
        })
    }
}

#[async_trait]
impl Slots for Steam {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        match (&request.slot, request.game.as_deref()) {
            (SlotId::EmulatorPanel, _) => self.libraries_panel().await,
            (SlotId::GameDetailSection, Some(id)) => {
                let game = self.game(id).await?;
                let rows = vec![
                    SectionRow { label: "Runs".into(), value: "Through Steam".into() },
                    SectionRow { label: "Steam app".into(), value: game.id },
                ];
                let content = SlotContent::Section { title: "Steam".into(), rows };
                Ok(vec![SlotItem { id: "steam".into(), order: Order::default(), content }])
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        Err(PluginError::Unsupported(format!("no action {action}")))
    }
}
