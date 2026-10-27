//! The core's side of the library: it asks the plugins for the games, their badges, details and
//! metadata, and hands the answers to the library subsystem ([`crate::library`]). A read
//! crosses areas (the emulators' builds, notices, the selection), so the core runs it.

use std::collections::HashMap;

use nexus_api::{Capability, ConfigScope, Event, GameKey, PageRequest, PluginId, Provided, SlotId, SlotRequest};
use serde_json::Value;

use crate::CoreResult;
use crate::app::Core;
use crate::changes::Area;
use crate::error::require;
use crate::library::{Detail, Entry, GameSettings, InstalledOffer};
use crate::releases::Offer;

/// How many games the store is asked about at once.
const DESCRIBE_AT_ONCE: usize = 6;
/// The longest the boot waits for the store's descriptions.
const BOOT_DESCRIBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

impl Core {
    /// The offer each of the `library`'s installed games is, with the offer's cover.
    pub(crate) fn installed_offers<'a>(
        &self,
        library: impl IntoIterator<Item = &'a Entry>,
        offers: &[Offer],
    ) -> Vec<InstalledOffer> {
        // Collected first: the games outlive the snapshots read here.
        let library: Vec<&Entry> = library.into_iter().collect();
        let (snapshot, torrents, installs) =
            (self.library().snapshot(), self.torrents().snapshot(), self.installs().snapshot());
        crate::ownership::Ownership::over(library, &snapshot, &torrents.jobs, &installs).installed_offers(offers)
    }

    pub(crate) async fn refresh_library(&self) -> CoreResult<()> {
        self.library().start_loading();
        let kernel = &self.inner.kernel;
        let mut library = Vec::new();
        let mut badges = HashMap::new();
        for (plugin, provided) in self.providers(Capability::Catalog).await {
            let catalog = provided.catalog.expect("declared and checked at init");
            let page = match kernel.guard(&plugin, "catalog.list", catalog.list(PageRequest::ALL)).await {
                Ok(page) => page,
                Err(e) => {
                    self.problem(e.to_string());
                    continue;
                }
            };
            for summary in page.items {
                let key = GameKey::new(plugin.clone(), summary.id.clone());
                if let Some(slots) = &provided.slots {
                    let request = SlotRequest { slot: SlotId::GameBadge, game: Some(summary.id.clone()) };
                    match kernel.guard(&plugin, "slots.items", slots.items(&request)).await {
                        Ok(items) => {
                            badges.insert(key.clone(), items);
                        }
                        Err(e) => self.problem(e.to_string()),
                    }
                }
                library.push(Entry { key, summary });
            }
        }
        crate::library::sort_by_title(&mut library);
        // The home screen shows the selected game with its details. With no selection, or one
        // that is gone, the first game is selected, so the first screen is a whole one. The
        // selection changes before the games do, so a reader that sees the new games sees it.
        let first = {
            let mut state = self.state();
            let kept = state.selected.as_ref().is_some_and(|s| library.iter().any(|e| &e.key == s));
            if kept {
                None
            } else {
                state.selected = library.first().map(|e| e.key.clone());
                state.selected.clone()
            }
        };
        let installed = self.installed_offers(&library, &self.library().snapshot().offers);
        self.library().set_library(library, badges, installed);
        kernel.events().publish(Event::LibraryChanged);
        self.changed(Area::Library);
        if let Some(game) = first
            && self.library().snapshot().detail(&game).is_none()
        {
            if let Err(e) = self.load_detail(&game).await {
                self.library().detail_failed(&game);
                self.problem(e.to_string());
            }
            self.changed(Area::Library);
        }
        if self.library().booted() {
            // Later loads: the store's art and the details, all in the background.
            let core = self.clone();
            tokio::spawn(async move {
                core.describe_library().await;
                core.read_all_details().await;
            });
            return Ok(());
        }
        // The first load of a run is the boot: the emulators' builds for the Welcome
        // checklist, and the store's art and details, as the old launcher's boot did, so Home
        // opens whole. A slow network doesn't hold it for more than BOOT_DESCRIBE_LIMIT.
        self.refresh_emulators().await;
        if self.kernel().clock().timeout(BOOT_DESCRIBE_LIMIT, self.describe_library()).await.is_none() {
            tracing::info!("the store's descriptions are slow: Home opens without the rest");
            let core = self.clone();
            tokio::spawn(async move { core.describe_library().await });
        }
        self.library().set_booted();
        let core = self.clone();
        tokio::spawn(async move {
            // The catalogs, read once a run, after Home is up.
            if !core.library().has_catalogs() {
                core.refresh_releases().await;
            }
            core.read_all_details().await;
        });
        Ok(())
    }

    /// Asks the metadata plugins about every game they haven't described yet.
    async fn describe_library(&self) {
        self.library().set_describing(Some((0, 0)));
        let todo: Vec<(GameKey, nexus_api::GameRef)> = {
            let library = self.library().snapshot();
            library
                .library()
                .filter(|e| library.meta(&e.key).is_none())
                .filter_map(|e| {
                    let title_id = e.summary.title_id.clone()?;
                    Some((
                        e.key.clone(),
                        nexus_api::GameRef {
                            platform: e.summary.platform.clone(),
                            title_id,
                            content_id: e.summary.content_id.clone().unwrap_or_default(),
                            title: e.summary.title.clone(),
                            region: String::new(),
                        },
                    ))
                })
                .collect()
        };
        let described = self.describe_with(todo, true).await;
        self.library().set_described(described);
    }

    /// Asks the metadata plugins about these games, and keeps what they say.
    pub(crate) async fn describe(&self, todo: Vec<(GameKey, nexus_api::GameRef)>) {
        self.describe_with(todo, false).await;
    }

    /// `false` when no plugin describes games. `boot` counts the answers for the boot screen.
    async fn describe_with(&self, todo: Vec<(GameKey, nexus_api::GameRef)>, boot: bool) -> bool {
        let providers: Vec<_> = self
            .providers(Capability::Metadata)
            .await
            .into_iter()
            .filter_map(|(plugin, provided)| provided.metadata.map(|m| (plugin, m)))
            .collect();
        if providers.is_empty() {
            return false;
        }
        // Six at a time, as the old launcher's fetch threads.
        let total = todo.len();
        let done = std::sync::atomic::AtomicUsize::new(0);
        let providers = &providers;
        let answers = futures::stream::iter(todo.into_iter().map(|(key, game)| async move {
            let mut failed = false;
            for (plugin, metadata) in providers {
                match self.inner.kernel.guard(plugin, "metadata.describe", metadata.describe(&game)).await {
                    Ok(Some(found)) => return (key, Some((plugin.clone(), found)), false),
                    Ok(None) => {}
                    Err(e) => {
                        failed = true;
                        tracing::info!(plugin_id = %plugin, error = %e, "could not describe a game");
                    }
                }
            }
            (key, None, failed)
        }));
        let mut answers = futures::StreamExt::buffer_unordered(answers, DESCRIBE_AT_ONCE);
        while let Some((key, found, failed)) = futures::StreamExt::next(&mut answers).await {
            let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let library = self.library();
            library.described(&key, found, failed);
            if boot {
                library.set_describing(Some((n, total)));
            }
            library.set_task(Some(format!("Downloading game art {n}/{total}")));
        }
        self.library().set_task(None);
        true
    }

    /// Reads every game's details, for the Library's sizes.
    pub(crate) async fn read_all_details(&self) {
        let keys: Vec<GameKey> = {
            let library = self.library().snapshot();
            library.library().map(|e| e.key.clone()).filter(|k| library.detail(k).is_none()).collect()
        };
        for (n, key) in keys.iter().enumerate() {
            self.library().set_task(Some(format!("Reading game details {}/{}", n + 1, keys.len())));
            if self.load_detail(key).await.is_err() {
                self.library().detail_failed(key);
            }
        }
        self.library().set_task(None);
    }

    /// Asks the game's plugin for its details, compatibility and slots.
    pub(crate) async fn load_detail(&self, game: &GameKey) -> CoreResult<()> {
        let offer = self.library().snapshot().offer(game).cloned();
        if let Some(offer) = offer {
            let detail = self.offer_detail(&offer);
            self.library().set_detail(game, detail, None);
            return Ok(());
        }
        let kernel = &self.inner.kernel;
        let plugin = game.plugin.clone();
        let provided = kernel.provided(&plugin).await?;
        let catalog = require(&plugin, Capability::Catalog, provided.catalog.clone())?;
        let details = kernel.guard(&plugin, "catalog.details", catalog.details(&game.local)).await?;
        let mut detail = Detail::of(details);
        let mut badges = None;
        if let Some(compat) = &provided.compatibility {
            match kernel.guard(&plugin, "compatibility.status", compat.status(&game.local)).await {
                Ok(info) => detail.compat = Some(info),
                Err(e) => self.problem(e.to_string()),
            }
        }
        if let Some(slots) = &provided.slots {
            for slot in [SlotId::GameAction, SlotId::GameDetailSection, SlotId::GameBadge] {
                let request = SlotRequest { slot, game: Some(game.local.clone()) };
                match kernel.guard(&plugin, "slots.items", slots.items(&request)).await {
                    Ok(items) => match slot {
                        SlotId::GameAction => detail.actions = items,
                        SlotId::GameDetailSection => detail.sections = items,
                        _ => badges = Some(items),
                    },
                    Err(e) => self.problem(e.to_string()),
                }
            }
        }
        detail.settings = self.game_settings(&plugin, &provided, game).await;
        self.refresh_emulator(&plugin).await;
        self.library().set_detail(game, detail, badges);
        Ok(())
    }

    /// A game's own settings, from its emulator's schema for it. `None` when it has none.
    pub(crate) async fn game_settings(
        &self,
        plugin: &PluginId,
        provided: &Provided,
        game: &GameKey,
    ) -> Option<GameSettings> {
        let configurator = provided.configurator.as_ref()?;
        let scope = ConfigScope::Game(game.local.clone());
        let schema = self.inner.kernel.guard(plugin, "configurator.schema", configurator.schema(&scope)).await.ok()?;
        if schema["properties"].as_object().is_none_or(|p| p.is_empty()) {
            return None;
        }
        let values = self.config_value(plugin, &scope).await.unwrap_or(Value::Null);
        Some(GameSettings { schema, values })
    }
}
