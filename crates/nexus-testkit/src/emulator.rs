//! [`FakeEmulator`]: a small emulator a test scripts, to drive the core through a whole
//! emulator's life: its catalog, the download and install of its build, and its games' launch
//! and exit. It passes [`crate::contract::emulator`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::*;

use crate::StubPlugin;

/// The permissions [`FakeEmulator::plugin`] declares: its builds' host, and its program in each
/// build's folder.
pub const FAKE_EMULATOR_PERMISSIONS: &str = "network = [\"builds.example\"]\nprocess = [\"builds/*/emu\"]";

/// An emulator for driving the core: a catalog of titles, one build to download, an installer
/// that writes into the plugin's storage, and a launcher that runs `builds/<version>/emu`.
///
/// Its games are `G0`, `G1`, … in the order given, on PS4, each with a cover in the plugin's
/// assets and a `region` badge. The test sets the public fields before it builds the plugin;
/// the newest build and the exits stay shared with every clone.
#[derive(Clone)]
pub struct FakeEmulator {
    pub titles: Vec<&'static str>,
    /// Where its build downloads from.
    pub build_url: String,
    /// The build's SHA-256, as hex: what [`crate::FakeHttp::serve`] gives back.
    pub build_sha: String,
    /// The build's size, when its plan says one.
    pub build_size: Option<u64>,
    /// The catalog fails to list, as a server that is down.
    pub broken_catalog: bool,
    /// Its launch hands the game over to a process with this text in its command line, as
    /// `steam steam://rungameid/…` does.
    pub game_process: Option<String>,
    /// A game its launcher won't start, and why.
    pub refuses: Option<(&'static str, &'static str)>,
    storage: Arc<Mutex<Option<Arc<dyn Storage>>>>,
    exits: Arc<Mutex<Vec<(String, ExitInfo)>>>,
    /// The newest build GitHub would list.
    latest: Arc<Mutex<String>>,
}

impl FakeEmulator {
    /// An emulator whose catalog has `titles`, and whose newest build is 1.0 at
    /// `https://builds.example/emu-1.0.bin`, with no SHA-256 until the test serves it.
    pub fn new(titles: Vec<&'static str>) -> Self {
        Self {
            titles,
            build_url: "https://builds.example/emu-1.0.bin".into(),
            build_sha: String::new(),
            build_size: None,
            broken_catalog: false,
            game_process: None,
            refuses: None,
            storage: Arc::default(),
            exits: Arc::default(),
            latest: Arc::new(Mutex::new("1.0".into())),
        }
    }

    /// Its launch hands the game over, as `steam steam://rungameid/…` does.
    pub fn handing_over(mut self, game_process: &str) -> Self {
        self.game_process = Some(game_process.into());
        self
    }

    /// A newer build comes out: the newest the plugin lists is now `version`.
    pub fn release(&self, version: &str) {
        *self.latest.lock().unwrap() = version.into();
    }

    /// The exits the plugin heard of, in order: each game and how it exited.
    pub fn exits(&self) -> Vec<(String, ExitInfo)> {
        self.exits.lock().unwrap().clone()
    }

    /// The plugin, as `plugin_id`, with [`FAKE_EMULATOR_PERMISSIONS`].
    pub fn plugin(&self, plugin_id: &str) -> StubPlugin {
        self.plugin_with(plugin_id, FAKE_EMULATOR_PERMISSIONS)
    }

    /// The plugin, as `plugin_id`, with `permissions`: TOML for its `[permissions]` section.
    pub fn plugin_with(&self, plugin_id: &str, permissions: &str) -> StubPlugin {
        self.plugin_named(plugin_id, &format!("name = \"{plugin_id}\""), permissions)
    }

    /// Like [`FakeEmulator::plugin_with`], with `names`: the manifest's `name` line, and an
    /// `emulator` line if it has one.
    pub fn plugin_named(&self, plugin_id: &str, names: &str, permissions: &str) -> StubPlugin {
        let emu = self.clone();
        StubPlugin::from_toml(&format!(
            "id = \"{plugin_id}\"\nkind = \"emulator\"\n{names}\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
             capabilities = [\"catalog\", \"downloader\", \"installer\", \"launcher\", \"slots\"]\n[permissions]\n{permissions}\n"
        ))
        .provides(move |ctx| {
            *emu.storage.lock().unwrap() = Some(ctx.storage().clone());
            let e = Arc::new(emu.clone());
            Ok(Provided {
                catalog: Some(e.clone()),
                downloader: Some(e.clone()),
                installer: Some(e.clone()),
                launcher: Some(e.clone()),
                slots: Some(e),
                ..Provided::default()
            })
        })
    }

    fn storage(&self) -> Arc<dyn Storage> {
        self.storage.lock().unwrap().clone().expect("init ran")
    }

    fn summary(&self, i: usize) -> GameSummary {
        GameSummary {
            id: format!("G{i}"),
            title: self.titles[i].into(),
            platform: "PS4".into(),
            cover: Some(ImageSource::Asset(RelPath::new(format!("covers/{i}.png")).unwrap())),
            tags: vec![],
            release_year: None,
            title_id: None,
            content_id: None,
        }
    }
}

#[async_trait]
impl Catalog for FakeEmulator {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        if self.broken_catalog {
            return Err(PluginError::Network("catalog server is down".into()));
        }
        let all = (0..self.titles.len()).map(|i| self.summary(i));
        Ok(Page { items: all.skip(page.offset as usize).take(page.limit as usize).collect(), total: None })
    }
    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let q = query.text.to_lowercase();
        Ok((0..self.titles.len())
            .filter(|i| self.titles[*i].to_lowercase().contains(&q))
            .map(|i| self.summary(i))
            .take(query.limit as usize)
            .collect())
    }
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let i: usize = game.trim_start_matches('G').parse().map_err(|_| PluginError::NotFound(game.into()))?;
        let summary = self.titles.get(i).map(|_| self.summary(i)).ok_or_else(|| PluginError::NotFound(game.into()))?;
        Ok(GameDetails {
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
            summary,
            description: "A game.".into(),
            developer: Some("Dev".into()),
            genres: vec!["Action".into()],
            size_bytes: None,
            background: None,
            facts: vec![],
        })
    }
    async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Downloader for FakeEmulator {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan> {
        match request {
            DownloadRequest::EmulatorBuild { version } => {
                // Each version downloads to its own file, as a real build would.
                let version = version.clone().unwrap_or_else(|| self.latest.lock().unwrap().clone());
                Ok(DownloadPlan {
                    label: format!("Emu {version}"),
                    artifacts: vec![Artifact {
                        url: self.build_url.clone(),
                        size: self.build_size,
                        sha256: self.build_sha.clone(),
                        signature: None,
                        dest: RelPath::new(format!("downloads/emu-{version}.bin")).unwrap(),
                        unpack: None,
                    }],
                })
            }
        }
    }
}

#[async_trait]
impl Installer for FakeEmulator {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(vec![VersionInfo { version: self.latest.lock().unwrap().clone(), notes: None }])
    }
    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        let names = self.storage().list(&RelPath::new("builds").unwrap()).await?;
        Ok(names
            .into_iter()
            .map(|v| InstalledVersion { path: RelPath::new(format!("builds/{v}")).unwrap(), version: v, own: false })
            .collect())
    }
    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let path = RelPath::new(format!("builds/{version}")).unwrap();
        let bin = path.join("emu").unwrap();
        let bytes = self.storage().read(&files[0].dest).await?;
        self.storage().write(&bin, &bytes).await?;
        self.storage().set_executable(&bin).await?;
        Ok(InstalledVersion { version: version.into(), path, own: false })
    }
    async fn uninstall(&self, version: &str) -> PluginResult<()> {
        self.storage().remove(&RelPath::new(format!("builds/{version}")).unwrap()).await
    }
}

#[async_trait]
impl Launcher for FakeEmulator {
    async fn prepare(&self, game: &str, config: &serde_json::Value) -> PluginResult<LaunchSpec> {
        let installed = self.installed().await?;
        let build = installed.last().ok_or_else(|| PluginError::Precondition("install the emulator first".into()))?;
        if let Some((refused, why)) = self.refuses
            && refused == game
        {
            return Err(PluginError::Precondition(why.into()));
        }
        let mut env = BTreeMap::new();
        if let Some(scale) = config.get("scale") {
            env.insert("EMU_SCALE".into(), scale.to_string().into());
        }
        Ok(LaunchSpec {
            program: build.path.join("emu").unwrap().into(),
            args: vec![game.into()],
            env,
            cwd: None,
            game_process: self.game_process.clone(),
        })
    }
    async fn on_exit(&self, game: &str, exit: &ExitInfo) -> PluginResult<()> {
        self.exits.lock().unwrap().push((game.into(), exit.clone()));
        Ok(())
    }
}

#[async_trait]
impl Slots for FakeEmulator {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        Ok(match request.slot {
            SlotId::GameBadge => vec![SlotItem {
                id: "region".into(),
                order: Order::default(),
                content: SlotContent::Badge { label: "EU".into(), short: String::new(), tone: Tone::Info, icon: None },
            }],
            _ => vec![],
        })
    }
    async fn run_action(&self, _: &str, _: Option<&str>, _: &serde_json::Value) -> PluginResult<()> {
        Ok(())
    }
}
