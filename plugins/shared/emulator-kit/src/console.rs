//! A console emulator as a plugin: its games in the player's game folders, the community
//! compatibility list, its settings, its packages to install, and its page on the Emulators
//! screen. The plugin gives a [`ConsoleSpec`] and a [`Flavor`]; it keeps its launch arguments
//! and its game installs.

use std::collections::HashMap;
use std::sync::Arc;

use nexus_api::serde_json::{self, Value};
use nexus_api::{
    ArtworkKind, Catalog, CompatInfo, CompatLevel, Compatibility, ConfigScope, Configurator, GAMES_FOLDER, GameDetails,
    GameSummary, HOST_INSTALL_GAME, HostPath, ImageSource, Installable, Order, PACKAGES_FOLDER, Page, PageRequest,
    Platform, PluginResult, RelPath, SearchQuery, SectionRow, SlotContent, SlotId, SlotItem, SlotRequest, Slots,
    async_trait,
};

use crate::builds::{BuildManager, Flavor, LISTS_TTL};
use crate::cache;
use crate::controls::Controls;
use crate::games::{self, Console, GameIndex};
use crate::host::Host;
use crate::installs::{self, Format};

/// A community compatibility list.
pub struct CompatList {
    pub url: &'static str,
    /// The list's page, for the player.
    pub page: &'static str,
    /// Reads the list for a system, by upper-case title id.
    pub parse: fn(&Value, Option<Platform>) -> HashMap<String, CompatInfo>,
}

/// What sets one console emulator apart, besides its builds.
pub struct ConsoleSpec {
    pub console: Console,
    /// The package formats it installs games from.
    pub packages: &'static [Format],
    pub compat: CompatList,
    /// Its default keyboard keys, as (button, keys).
    pub keys: &'static [(&'static str, &'static str)],
    /// The id of its section on a game's page.
    pub section: &'static str,
    /// The section lists the game's version.
    pub version_row: bool,
    /// The section's build row says "Your own build" while it is on.
    pub own_row: bool,
}

pub struct ConsoleEmulator<F: Flavor> {
    spec: ConsoleSpec,
    host: Host,
    games: GameIndex,
    builds: Arc<BuildManager<F>>,
}

impl<F: Flavor> ConsoleEmulator<F> {
    /// The emulator, with settings stored in an older shape rewritten first.
    pub async fn start(flavor: F, spec: ConsoleSpec, host: Host) -> PluginResult<Self> {
        let builds = Arc::new(BuildManager::new(flavor, host.clone()));
        builds.settings().migrate().await?;
        let games = GameIndex::new(GAMES_FOLDER, spec.console);
        Ok(Self { spec, host, games, builds })
    }

    pub fn builds(&self) -> &Arc<BuildManager<F>> {
        &self.builds
    }

    pub fn games(&self) -> &GameIndex {
        &self.games
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn controls(&self) -> Controls {
        Controls { keys: self.spec.keys }
    }

    /// The packages to install in this console's folder of each packages folder, as installs go
    /// to its folder of the games folder.
    pub async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        let folders = &self.host.folders;
        let mut out = Vec::new();
        for root in folders.roots(PACKAGES_FOLDER).await? {
            let dir = HostPath::new(PACKAGES_FOLDER, root.root, RelPath::new(F::CONSOLE)?);
            let Ok(entries) = folders.list(&dir).await else { continue };
            for entry in entries {
                if entry.is_dir {
                    continue;
                }
                let file = dir.join(&entry.name)?;
                let head = folders.read_range(&file, 0, 512).await.unwrap_or_default();
                if installs::format(&head).is_some_and(|f| self.spec.packages.contains(&f)) {
                    out.push(Installable { source: file, label: entry.name });
                }
            }
        }
        Ok(out)
    }

    async fn compat_list(&self) -> HashMap<String, CompatInfo> {
        let Ok(cache) = RelPath::new("cache/compatibility.json") else { return Default::default() };
        let host = &self.host;
        let list = &self.spec.compat;
        match cache::cached_json(host.storage.as_ref(), host.http.as_ref(), &host.log, list.url, &cache, LISTS_TTL)
            .await
        {
            Ok(json) => (list.parse)(&json, host.platform),
            Err(e) => {
                host.log.warn(&format!("compatibility list: {e}"));
                Default::default()
            }
        }
    }
}

#[async_trait]
impl<F: Flavor> Catalog for ConsoleEmulator<F> {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games.refresh(self.host.folders.as_ref()).await?;
        let total = all.len() as u32;
        let items =
            all.iter().skip(page.offset as usize).take(page.limit as usize).map(|g| g.summary(F::CONSOLE)).collect();
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let all = self.games.refresh(self.host.folders.as_ref()).await?;
        Ok(games::search(all, query).iter().map(|g| g.summary(F::CONSOLE)).collect())
    }

    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        self.games.details(self.host.folders.as_ref(), game, F::CONSOLE).await
    }

    async fn artwork(&self, game: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        let found = self.games.find(self.host.folders.as_ref(), game).await?;
        Ok(match kind {
            ArtworkKind::Background => found.background,
            _ => found.icon,
        }
        .map(ImageSource::Folder))
    }
}

#[async_trait]
impl<F: Flavor> Compatibility for ConsoleEmulator<F> {
    async fn status(&self, game: &str) -> PluginResult<CompatInfo> {
        let found = self.games.find(self.host.folders.as_ref(), game).await?;
        let page = Some(self.spec.compat.page.to_owned());
        Ok(match self.compat_list().await.remove(&found.title_id.to_uppercase()) {
            Some(info) => CompatInfo { page, ..info },
            None => CompatInfo { level: CompatLevel::Unknown, notes: None, source: None, page, tested: None },
        })
    }
}

#[async_trait]
impl<F: Flavor> Configurator for ConsoleEmulator<F> {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Configurator::schema(self.builds.settings(), scope).await
    }

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Configurator::defaults(self.builds.settings(), scope).await
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        Configurator::validate(self.builds.settings(), scope, value).await
    }
}

#[async_trait]
impl<F: Flavor> Slots for ConsoleEmulator<F> {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        Ok(match (request.slot, request.game.as_deref()) {
            (SlotId::GameBadge, Some(game)) => {
                vec![item("compat", games::compat_badge(&self.status(game).await?, self.host.platform))]
            }
            (SlotId::GameDetailSection, Some(game)) => {
                let found = self.games.find(self.host.folders.as_ref(), game).await?;
                let build = match self.spec.own_row {
                    true => self.builds.build_label().await,
                    false => self.builds.current_label().await,
                };
                let mut rows = vec![SectionRow { label: "Title id".into(), value: found.title_id }];
                if self.spec.version_row {
                    rows.push(SectionRow { label: "Version".into(), value: found.version });
                }
                rows.push(SectionRow { label: "Build".into(), value: build.unwrap_or_else(|| "not installed".into()) });
                vec![item(self.spec.section, SlotContent::Section { title: F::NAME.into(), rows })]
            }
            (SlotId::EmulatorPanel, None) => {
                let mut items = self.builds.switch_items().await;
                for found in self.candidates().await.unwrap_or_default() {
                    items.push(item(
                        &format!("install-{}", found.label),
                        SlotContent::Action {
                            label: format!("Install {}", found.label),
                            action: HOST_INSTALL_GAME.into(),
                            args: serde_json::to_value(&found.source)?,
                            style: nexus_api::ActionStyle::Secondary,
                            icon: None,
                        },
                    ));
                }
                items.push(self.controls().keyboard_panel());
                items.extend(self.builds.update_items().await);
                items
            }
            (SlotId::SettingsPanel, None) => vec![item(
                "settings",
                SlotContent::SettingsPanel { title: F::NAME.into(), scope: ConfigScope::Emulator },
            )],
            _ => Vec::new(),
        })
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        self.builds.run_action(action).await
    }
}
