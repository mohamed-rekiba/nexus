//! Capability contract suites: the behaviour the host relies on, checked against any emulator
//! plugin. Each check panics with `contract <capability>: ...` when the plugin breaks a rule.
//!
//! Run [`emulator`] on a started core with the plugin loaded.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use nexus_api::*;
use nexus_core::Command;

use crate::TestCore;

/// Runs every contract for the capabilities `plugin` declares.
pub async fn emulator(test: &TestCore, plugin: &PluginId) {
    assert!(test.report().rejection(plugin).is_none(), "contract boot: {plugin} was rejected: {:?}", test.report());
    let kernel = test.core.kernel();
    let manifest = kernel.manifest(plugin).expect("loaded").clone();
    let provided = kernel.provided(plugin).await.unwrap_or_else(|e| panic!("contract init: {e}"));
    let caps = &manifest.capabilities;

    let mut first_game = None;
    if let Some(catalog) = &provided.catalog {
        first_game = Some(catalog_contract(catalog.as_ref()).await);
    }
    if let Some(downloader) = &provided.downloader {
        downloader_contract(downloader.as_ref()).await;
    }
    if let Some(configurator) = &provided.configurator {
        configurator_contract(configurator.as_ref(), first_game.as_deref()).await;
    }
    if let Some(compatibility) = &provided.compatibility {
        let game = first_game.as_deref().expect("contract compatibility: needs a catalog game");
        compatibility.status(game).await.unwrap_or_else(|e| panic!("contract compatibility: status({game}): {e}"));
        assert!(
            compatibility.status("__no_such_game__").await.is_err(),
            "contract compatibility: unknown game must fail"
        );
    }
    if let Some(saves) = &provided.saves {
        let game = first_game.as_deref().expect("contract saves: needs a catalog game");
        saves.list(game).await.unwrap_or_else(|e| panic!("contract saves: list({game}): {e}"));
    }
    if let Some(controllers) = &provided.controllers {
        let profiles = controllers.profiles().await.unwrap_or_else(|e| panic!("contract controllers: profiles: {e}"));
        let ids: BTreeSet<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), profiles.len(), "contract controllers: profile ids are unique");
        assert!(
            profiles.iter().all(|p| !p.id.is_empty() && !p.name.is_empty()),
            "contract controllers: empty id or name"
        );
    }
    if let Some(shaders) = &provided.shaders {
        shaders_contract(shaders.as_ref(), first_game.as_deref()).await;
    }
    if let Some(slots) = &provided.slots {
        slots_contract(slots.as_ref(), first_game.as_deref()).await;
    }
    if caps.contains(&Capability::Installer) {
        install_contract(test, plugin, provided.installer.as_ref().expect("declared")).await;
    }
    if caps.contains(&Capability::Launcher) {
        launch_contract(test, plugin, first_game.as_deref().expect("contract launcher: needs a catalog game")).await;
    }
}

/// Returns the first game's id.
async fn catalog_contract(catalog: &dyn Catalog) -> String {
    let all = catalog.list(PageRequest::ALL).await.unwrap_or_else(|e| panic!("contract catalog: list: {e}")).items;
    assert!(!all.is_empty(), "contract catalog: the catalog lists no games");
    let ids: BTreeSet<&str> = all.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids.len(), all.len(), "contract catalog: game ids must be unique");
    assert!(all.iter().all(|g| !g.id.is_empty() && !g.title.is_empty()), "contract catalog: empty id or title");

    if all.len() > 1 {
        let page = catalog.list(PageRequest { offset: 1, limit: 1 }).await.expect("paged list").items;
        assert_eq!(page, all[1..2], "contract catalog: offset and limit page through the same list");
    }
    let first = &all[0];
    let details = catalog.details(&first.id).await.unwrap_or_else(|e| panic!("contract catalog: details: {e}"));
    assert_eq!(details.summary.id, first.id, "contract catalog: details describe the asked-for game");
    assert!(
        matches!(catalog.details("__no_such_game__").await, Err(PluginError::NotFound(_))),
        "contract catalog: an unknown game is NotFound"
    );
    let found = catalog.search(&SearchQuery { text: first.title.clone(), limit: 50 }).await.expect("search");
    assert!(found.iter().any(|g| g.id == first.id), "contract catalog: searching a title finds it");
    let one = catalog.search(&SearchQuery { text: String::new(), limit: 1 }).await.expect("search");
    assert!(one.len() <= 1, "contract catalog: search honours its limit");
    first.id.clone()
}

async fn downloader_contract(downloader: &dyn Downloader) {
    let plan = downloader
        .plan(&DownloadRequest::EmulatorBuild { version: None })
        .await
        .unwrap_or_else(|e| panic!("contract downloader: plan for the newest build: {e}"));
    assert!(!plan.label.is_empty(), "contract downloader: a plan has a label");
    assert!(!plan.artifacts.is_empty(), "contract downloader: a plan has files");
    for artifact in &plan.artifacts {
        let hex = artifact.sha256.len() == 64 && artifact.sha256.chars().all(|c| c.is_ascii_hexdigit());
        assert!(hex, "contract downloader: {} needs a hex SHA-256", artifact.dest);
        assert!(!artifact.dest.is_root(), "contract downloader: a file needs a destination");
    }
}

async fn configurator_contract(configurator: &dyn Configurator, game: Option<&str>) {
    let mut scopes = vec![ConfigScope::Emulator];
    scopes.extend(game.map(|g| ConfigScope::Game(g.into())));
    for scope in scopes {
        let schema = configurator.schema(&scope).await.unwrap_or_else(|e| panic!("contract configurator: schema: {e}"));
        assert!(schema.is_object(), "contract configurator: the {scope:?} schema is a JSON object");
        let defaults = configurator.defaults(&scope).await.expect("defaults");
        configurator
            .validate(&scope, &defaults)
            .await
            .unwrap_or_else(|e| panic!("contract configurator: the {scope:?} defaults must validate: {e}"));
        assert!(
            matches!(
                configurator.validate(&scope, &serde_json::json!("not settings")).await,
                Err(PluginError::InvalidInput(_))
            ),
            "contract configurator: garbage is InvalidInput"
        );
    }
}

async fn shaders_contract(shaders: &dyn Shaders, game: Option<&str>) {
    let game = game.expect("contract shaders: needs a catalog game");
    shaders.cache(game).await.unwrap_or_else(|e| panic!("contract shaders: cache({game}): {e}"));
    assert!(shaders.cache("__no_such_game__").await.is_err(), "contract shaders: an unknown game must fail");
    shaders.clear_cache(game).await.unwrap_or_else(|e| panic!("contract shaders: clear_cache({game}): {e}"));
    let after = shaders.cache(game).await.expect("cache");
    assert_eq!((after.entries, after.size_bytes), (0, 0), "contract shaders: a clear empties the cache");
    shaders.clear_cache(game).await.expect("contract shaders: clearing an empty cache is fine");
}

async fn slots_contract(slots: &dyn Slots, game: Option<&str>) {
    for slot in SlotId::ALL {
        let request = SlotRequest { slot, game: slot.is_per_game().then(|| game.map(str::to_owned)).flatten() };
        let items = slots.items(&request).await.unwrap_or_else(|e| panic!("contract slots: {}: {e}", slot.as_str()));
        let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "contract slots: ids are unique within {}", slot.as_str());
    }
}

async fn install_contract(test: &TestCore, plugin: &PluginId, installer: &Arc<dyn Installer>) {
    let available = installer.available().await.unwrap_or_else(|e| panic!("contract installer: available: {e}"));
    let newest = available.first().expect("contract installer: at least one version is available").version.clone();
    test.core
        .dispatch(Command::InstallEmulator { plugin: plugin.clone(), version: None })
        .await
        .unwrap_or_else(|e| panic!("contract installer: install through the host: {e}"));
    let installed = installer.installed().await.expect("installed");
    assert!(installed.iter().any(|v| v.version == newest), "contract installer: {newest} shows as installed");
    for v in &installed {
        assert!(!v.path.is_root(), "contract installer: an install has its own folder");
    }
    test.core
        .dispatch(Command::UninstallEmulator { plugin: plugin.clone(), version: newest.clone() })
        .await
        .unwrap_or_else(|e| panic!("contract installer: uninstall: {e}"));
    assert!(
        !installer.installed().await.expect("installed").iter().any(|v| v.version == newest),
        "contract installer: uninstall removes {newest}"
    );
    // Leave it installed for the launch contract.
    test.core.dispatch(Command::InstallEmulator { plugin: plugin.clone(), version: None }).await.expect("reinstall");
}

async fn launch_contract(test: &TestCore, plugin: &PluginId, game: &str) {
    let mut events = test.core.events();
    let key = GameKey::new(plugin.clone(), game);
    // Installs may have run programs (an AppImage's extract); the game is the next one.
    let before = test.spawner.spawned().len();
    test.core
        .dispatch(Command::Launch(key.clone()))
        .await
        .unwrap_or_else(|e| panic!("contract launcher: launch through the host: {e}"));
    let process =
        tokio::time::timeout(Duration::from_secs(5), test.spawner.wait_for(before + 1)).await.expect("spawned");
    // The program is in the plugin's data folder, in a host folder it was granted, or a
    // system program by name.
    let program = &process.command.program;
    let data = test.data_dir().join("plugins").join(plugin.as_str());
    let grants = &test.core.kernel().manifest(plugin).expect("loaded").permissions.filesystem;
    let folders = test.core.view().settings.folders;
    let in_granted_folder = folders
        .iter()
        .filter(|f| grants.iter().any(|g| g.folder == f.name))
        .flat_map(|f| f.paths.iter())
        .any(|root| program.starts_with(root));
    let system = program.components().count() == 1;
    assert!(
        program.starts_with(&data) || in_granted_folder || system,
        "contract launcher: {} is not in a place the plugin may run programs from",
        program.display()
    );
    process.exit(0);
    let exited = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if let Event::ProcessExited { game, .. } = event {
                return game;
            }
        }
        panic!("the event bus closed")
    })
    .await
    .expect("contract launcher: the host reports the exit");
    assert_eq!(exited, key);
}
