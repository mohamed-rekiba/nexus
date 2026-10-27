//! The core's side of the catalogs: it asks the plugins that give release catalogs for each
//! console's releases and their Catalog buttons, groups the releases into offers
//! ([`crate::releases`]), and hands them to the library subsystem. It reads the stores' installs
//! (Steam's) too, and describes the games on the Catalog's shown pages. These are flows: free
//! functions over the core, since a read crosses areas (the library, the downloads, the
//! notices).

use nexus_api::{Capability, Console, FactKind, GameDetails, GameFact, GameKey, PluginId, PluginKind};

use crate::app::Core;
use crate::changes::Area;
use crate::library::Detail;
use crate::releases::{CatalogCopy, Offer, date_words, days_of, offers};

/// Each turned-on emulator that names a console in its manifest, with that console, in load
/// order. Only the first emulator for a console is in the list: it gets that console's
/// catalog. An emulator that names no console gets no catalog.
fn consoles(kernel: &crate::Kernel) -> Vec<(PluginId, Console)> {
    let mut out: Vec<(PluginId, Console)> = Vec::new();
    for emulator in kernel.of_kind(PluginKind::Emulator) {
        let Some(console) = kernel.manifest(&emulator).ok().and_then(|m| m.console.clone()) else { continue };
        if !out.iter().any(|(_, c)| *c == console) {
            out.push((emulator, console));
        }
    }
    out
}

/// Reads the catalogs again each time a catalog plugin says they changed
/// ([`nexus_api::RELEASES_CHANGED`]), for example after a refresh in the background.
pub(crate) fn watch_catalogs(core: &Core) {
    use futures::StreamExt;
    let weak = std::sync::Arc::downgrade(&core.inner);
    let mut events = core.events();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            let nexus_api::Event::Custom { plugin, topic, .. } = event else { continue };
            if topic != nexus_api::RELEASES_CHANGED {
                continue;
            }
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            let lists = core.kernel().manifest(&plugin).is_ok_and(|m| m.capabilities.contains(&Capability::Releases));
            if lists {
                // Not in this loop: the reads publish events of their own.
                tokio::spawn(async move { refresh_releases(&core).await });
            }
        }
    });
}

/// Asks every catalog plugin for the catalog of each console an emulator plays, and
/// groups the releases. Then asks each for its Catalog buttons (`catalog.action`): after the
/// catalogs, so the plugin knows which consoles they are for.
pub(crate) async fn refresh_releases(core: &Core) {
    // One read at a time. Two at once could end in the wrong order: a read that began
    // before a catalog changed would then put its old copies over the new ones.
    let _reading = core.inner.reading_catalogs.lock().await;
    let kernel = core.kernel();
    let emulators = consoles(kernel);
    let mut copies = Vec::new();
    let mut buttons = Vec::new();
    for (service, provided) in core.providers(Capability::Releases).await {
        let Some(releases) = provided.releases else { continue };
        for (emulator, console) in &emulators {
            // An emulator that lists its own console's releases (Steam's library) gets no
            // other catalog for it.
            let own_list = kernel.manifest(emulator).is_ok_and(|m| m.capabilities.contains(&Capability::Releases));
            if own_list && emulator != &service {
                continue;
            }
            match kernel.guard(&service, "releases.releases", releases.releases(console)).await {
                Ok(Some(list)) => {
                    copies.push(CatalogCopy { service: service.clone(), emulator: emulator.clone(), list })
                }
                Ok(None) => {}
                Err(e) => core.notices().problem(e.to_string()),
            }
        }
        if let Some(slots) = &provided.slots {
            let request = nexus_api::SlotRequest { slot: nexus_api::SlotId::CatalogAction, game: None };
            match kernel.guard(&service, "slots.items", slots.items(&request)).await {
                Ok(items) => buttons.push((service.clone(), items)),
                Err(e) => core.notices().problem(e.to_string()),
            }
        }
    }
    let offers: Vec<Offer> = copies.iter().flat_map(offers).collect();
    let installed = crate::library_reads::installed_offers(core, core.library().snapshot().library(), &offers);
    let added = core.library().set_catalogs(copies, buttons, offers, installed);
    if added > 0 {
        let s = if added == 1 { "" } else { "s" };
        core.notices().opening(
            nexus_ui_api::NoticeKind::Info,
            crate::app::Route::Catalog,
            format!("{added} new release{s} in the Catalog"),
        );
    }
    crate::disks::refresh(core);
    core.changed(Area::Catalogs);
    describe_catalog_page(core);
    // A catalog refresh waits for the VPN: the VPN check reads the catalogs again when it is back.
    if core.torrents().plugin_waits_for_vpn() {
        core.torrents().watch();
    }
}

/// Reads the stores' installs (Steam's) in the background: often while one runs or right
/// after Nexus asked a store to install, rarely otherwise.
pub(crate) fn watch_store_installs(core: &Core) {
    let weak = std::sync::Arc::downgrade(&core.inner);
    let every = core.kernel().host().store_poll;
    let clock = core.ports().clock.clone();
    tokio::spawn(async move {
        loop {
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            refresh_store_installs(&core).await;
            let busy = {
                let state = core.state();
                !state.store_installs.is_empty() || state.store_watch_until.is_some_and(|until| clock.now() < until)
            };
            drop(core);
            clock.sleep(if busy { every } else { every * 10 }).await;
        }
    });
}

/// Asks each catalog plugin for its store's installs. One that is gone has ended: the
/// Library and the catalogs are read again, and an install that left its game installed
/// says so.
async fn refresh_store_installs(core: &Core) {
    let kernel = core.kernel();
    let mut now = Vec::new();
    for (plugin, provided) in core.providers(Capability::Releases).await {
        let Some(releases) = provided.releases else { continue };
        match kernel.guard(&plugin, "releases.store_installs", releases.store_installs()).await {
            Ok(installs) => now.extend(installs.into_iter().map(|i| (plugin.clone(), i))),
            Err(e) => tracing::warn!(error = %e, "could not read a store's installs"),
        }
    }
    let ended: Vec<(PluginId, nexus_api::StoreInstall)> = {
        let state = core.state();
        if state.store_installs == now {
            return;
        }
        let still =
            |p: &PluginId, i: &nexus_api::StoreInstall| now.iter().any(|(q, j)| q == p && j.release == i.release);
        state.store_installs.iter().filter(|(p, i)| !still(p, i)).cloned().collect()
    };
    // The game is in the Library before its card leaves Downloads.
    if !ended.is_empty() {
        if let Err(e) = Box::pin(crate::library_reads::refresh_library(core)).await {
            tracing::warn!(error = %e, "could not read the Library again");
        }
        refresh_releases(core).await;
    }
    core.state().store_installs = now;
    if !ended.is_empty() {
        for (plugin, install) in &ended {
            let installed = {
                let library = core.library().snapshot();
                let (torrents, installs) = (core.torrents().snapshot(), core.installs().snapshot());
                let owned = crate::ownership::Ownership::new(&library, &torrents.jobs, &installs);
                library.offers.iter().any(|o| {
                    &o.entry.key.plugin == plugin
                        && o.releases.iter().any(|r| r.id == install.release)
                        && owned.installed_game(o).is_some()
                })
            };
            if installed {
                core.notices().info(format!("{} is installed", install.title));
            }
        }
    }
    core.changed(Area::Downloads);
}

/// An offer's Game Hub: the words and facts of its best release (see
/// [`crate::ownership::Ownership::best_release`]), with how many share it and when the catalog counted them.
pub(crate) fn offer_detail(core: &Core, offer: &Offer) -> Detail {
    let best = {
        let library = core.library().snapshot();
        let (torrents, installs) = (core.torrents().snapshot(), core.installs().snapshot());
        crate::ownership::Ownership::new(&library, &torrents.jobs, &installs).best_release(offer).clone()
    };
    let mut facts = Vec::new();
    let mut fact = |id: &str, label: &str, value: &str| {
        if !value.is_empty() {
            facts.push(GameFact::new(id, label, value));
        }
    };
    fact("region", "Region", &best.region);
    fact("version", "Version", &best.version);
    if let Some(t) = &offer.entry.summary.title_id {
        fact("title-id", "Title ID", t);
    }
    // The catalog's facts, in its order. The genre words and the compatibility tier show
    // elsewhere.
    for f in best.facts.iter().filter(|f| !matches!(f.kind, FactKind::Genre | FactKind::Compatibility)) {
        let value = if f.kind == FactKind::ReleaseDate { date_words(&f.value) } else { f.value.clone() };
        fact(f.kind.id(), &f.label, &value);
    }
    // The catalog's counts, not live ones.
    let count = |n: Option<u64>| n.map_or_else(|| "Unknown".to_owned(), |n| n.to_string());
    fact("seeders", "Sharing it (seeders)", &count(best.seeders));
    fact("leechers", "Getting it (leechers)", &count(best.leechers));
    let counted = match best.counted_at.get(..10).filter(|d| days_of(d).is_some()) {
        Some(day) => format!("{} · not live", date_words(day)),
        None => "Date unknown".to_owned(),
    };
    fact("counted", "Counted", &counted);
    Detail::of(GameDetails {
        summary: offer.entry.summary.clone(),
        description: best.description.clone(),
        developer: Some(best.fact_text(FactKind::Developer).to_owned()).filter(|d| !d.is_empty()),
        genres: Vec::new(),
        size_bytes: best.size_bytes,
        background: None,
        facts,
        folder: None,
        also: Vec::new(),
        uninstall_link: None,
    })
}

/// Asks the stores about the games on the Catalog's shown pages that they haven't
/// described yet, by title id and the newest release's region. In the background.
pub(crate) fn describe_catalog_page(core: &Core) {
    let snapshot = core.snapshot();
    let page = snapshot.catalog_page();
    let library = snapshot.library;
    let refs: Vec<(GameKey, nexus_api::GameRef)> = page
        .iter()
        .filter_map(|key| library.offer(key))
        .filter_map(|o| {
            Some((
                o.entry.key.clone(),
                nexus_api::GameRef {
                    platform: o.entry.summary.platform.clone(),
                    title_id: o.entry.summary.title_id.clone()?,
                    content_id: String::new(),
                    title: o.entry.summary.title.clone(),
                    region: o.releases[0].region.clone(),
                },
            ))
        })
        .collect();
    let keys: Vec<GameKey> = refs.iter().map(|(key, _)| key.clone()).collect();
    let asking = core.library().to_ask(&keys);
    let todo: Vec<_> = refs.into_iter().filter(|(key, _)| asking.contains(key)).collect();
    if todo.is_empty() {
        return;
    }
    let core = core.clone();
    tokio::spawn(async move { crate::library_reads::describe(&core, todo).await });
}
