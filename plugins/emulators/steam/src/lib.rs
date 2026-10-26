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
mod store;
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
/// Steam's art files for a game: its cover, the Game Hub's background, its logo, and the store's
/// wide header. Steam's server has the cover and the header of every game, but an older game
/// may have no hero or logo there: those come from Steam's cache only.
const COVER: &str = "library_600x900.jpg";
const HERO: &str = "library_hero.jpg";
const LOGO: &str = "logo.png";
const HEADER: &str = "header.jpg";
/// The most bytes of an app manifest or library list read.
const MAX_FILE: u64 = 256 * 1024;
/// `StateFlags`' bits (Steam's `EAppState`): all its files are there; its update is paused;
/// it downloads; it writes the download into place (staging, then committing).
const FULLY_INSTALLED: u64 = 4;
const UPDATE_PAUSED: u64 = 512;
const DOWNLOADING: u64 = 1 << 20;
const STAGING: u64 = 1 << 21;
const COMMITTING: u64 = 1 << 22;
/// The link that opens Steam's own Downloads page.
const DOWNLOADS: &str = "steam://open/downloads";

/// An app file's `StateFlags`.
fn flags(app: &vdf::Block) -> u64 {
    app.text("StateFlags").and_then(|f| f.parse().ok()).unwrap_or(0)
}
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
            metadata: Some(steam.clone()),
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

    /// Every app file of every library: its `AppState` keys, its library's `steamapps` folder,
    /// and whether that library is a Flatpak Steam's. An app in two libraries (Steam's folder
    /// both as itself and as `~/.steam/steam`) once.
    async fn apps(&self) -> PluginResult<Vec<(vdf::Block, HostPath, bool)>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for library in self.libraries().await? {
            let apps = HostPath::new(STEAM, library.root, path("steamapps")?);
            for entry in self.ctx.folders().list(&apps).await.unwrap_or_default() {
                if !(entry.name.starts_with("appmanifest_") && entry.name.ends_with(".acf")) {
                    continue;
                }
                let Some(file) = self.read_vdf(&apps.join(&entry.name)?).await else { continue };
                let Some(app) = file.block("AppState").cloned() else { continue };
                let (Some(id), Some(name)) = (app.text("appid"), app.text("name")) else { continue };
                if is_tool(id, name) || !seen.insert(id.to_owned()) {
                    continue;
                }
                out.push((app, apps.clone(), library.path.contains(FLATPAK_FOLDER)));
            }
        }
        Ok(out)
    }

    /// Every fully installed game, by name.
    async fn games(&self) -> PluginResult<Vec<Game>> {
        let mut out = Vec::new();
        for (app, apps, flatpak) in self.apps().await? {
            let (Some(id), Some(name), Some(dir)) = (app.text("appid"), app.text("name"), app.text("installdir"))
            else {
                continue;
            };
            if flags(&app) & FULLY_INSTALLED == 0 {
                continue;
            }
            out.push(Game {
                id: id.to_owned(),
                name: name.to_owned(),
                dir: apps.join(&format!("common/{dir}"))?,
                size: app.text("SizeOnDisk").and_then(|s| s.parse().ok()).filter(|s| *s > 0),
                flatpak,
            });
        }
        out.sort_by_cached_key(|g| g.name.to_lowercase());
        Ok(out)
    }

    /// The games Steam is installing: every app not fully installed yet, with its phase and
    /// bytes from its app file.
    pub(crate) async fn installing(&self) -> PluginResult<Vec<StoreInstall>> {
        let mut out = Vec::new();
        for (app, _, _) in self.apps().await? {
            let state = flags(&app);
            if state & FULLY_INSTALLED != 0 {
                continue;
            }
            let (Some(id), Some(name)) = (app.text("appid"), app.text("name")) else { continue };
            let bytes = |key: &str| app.text(key).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            let (got, to_get) = (bytes("BytesDownloaded"), bytes("BytesToDownload"));
            let (staged, to_stage) = (bytes("BytesStaged"), bytes("BytesToStage"));
            let downloaded = to_get > 0 && got >= to_get;
            let (phase, done, total) = if state & UPDATE_PAUSED != 0 {
                (StorePhase::Paused, got, to_get)
            } else if downloaded || state & (STAGING | COMMITTING) != 0 {
                (StorePhase::Installing, staged, to_stage)
            } else if got > 0 || state & DOWNLOADING != 0 {
                (StorePhase::Downloading, got, to_get)
            } else {
                (StorePhase::Queued, got, to_get)
            };
            out.push(StoreInstall {
                release: format!("steam-{id}"),
                title: name.to_owned(),
                phase,
                done,
                total,
                open: DOWNLOADS.into(),
            });
        }
        out.sort_by(|a, b| a.title.cmp(&b.title));
        Ok(out)
    }

    async fn game(&self, id: &str) -> PluginResult<Game> {
        let games = self.games().await?;
        games.into_iter().find(|g| g.id == id).ok_or_else(|| PluginError::NotFound(id.into()))
    }

    /// Game `id`'s art of `kind`: Steam's cached file, else the server's when every game has it
    /// there. The cover is the cached or the server's [`COVER`]; the background the cached
    /// [`HERO`], else the server's [`HEADER`]; the logo only the cached [`LOGO`]. `None` for an
    /// icon, and for a logo Steam hasn't cached.
    async fn art(&self, id: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        let server = |file: &str| Some(ImageSource::Url(format!("{ART}/{id}/{file}")));
        Ok(match kind {
            ArtworkKind::Cover => self.cached(id, COVER).await?.or_else(|| server(COVER)),
            ArtworkKind::Background => self.cached(id, HERO).await?.or_else(|| server(HEADER)),
            ArtworkKind::Logo => self.cached(id, LOGO).await?,
            ArtworkKind::Icon => None,
        })
    }

    /// Steam's art `file` for game `id` in a library's cache, as Steam keeps it since 2024
    /// (`librarycache/<id>/…`, at times one folder deeper) or before (`librarycache/<id>_…`).
    async fn cached(&self, id: &str, file: &str) -> PluginResult<Option<ImageSource>> {
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
                    return Ok(Some(ImageSource::Folder(place)));
                }
            }
        }
        Ok(None)
    }

    async fn summary(&self, game: &Game) -> PluginResult<GameSummary> {
        Ok(GameSummary {
            id: game.id.clone(),
            title: game.name.clone(),
            platform: PLATFORM.into(),
            cover: self.art(&game.id, ArtworkKind::Cover).await?,
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
            background: self.art(&game.id, ArtworkKind::Background).await?,
            facts: Vec::new(),
            folder: Some(game.dir),
            also: Vec::new(),
            // Steam removes its games: Uninstall asks it to, as Steam's own menu does.
            uninstall_link: Some(format!("{UNINSTALL}{}", game.id)),
        })
    }

    async fn artwork(&self, id: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        self.art(id, kind).await
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

    async fn store_installs(&self) -> PluginResult<Vec<StoreInstall>> {
        self.installing().await
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
