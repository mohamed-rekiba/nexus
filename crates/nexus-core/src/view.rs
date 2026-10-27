//! Building view models from a [`Snapshot`] of the app. [`build`] is pure: it takes no lock and
//! does no I/O, so a render is a function of the snapshot.

use std::collections::BTreeMap;

use nexus_api::{
    ActionStyle, Capability, CompatInfo, CompatLevel, GameKey, Platform, PluginId, PluginKind, SlotContent, SlotId,
    SlotItem, Tone,
};
use nexus_ui_api::*;
use serde_json::Value;

use crate::app::Route;
use crate::game_installs::InstallsSnapshot;
use crate::grid_query::{Grid, GridData, GridGames, GridQuery};
use crate::library::{Entry, LibrarySnapshot, PluginImage, image_ref};
use crate::order::sort_by_order;
use crate::ownership::Ownership;
use crate::snapshot::{Plugins, Snapshot};
use crate::torrents::TorrentJob;
use crate::{JobPhase, PluginStatus};

/// A plugin's image as the theme host fetches it.
fn plugin_image((plugin, source): PluginImage) -> ImageRef {
    image_ref(&plugin, &source)
}

/// One plugin's slot items as view models, in [`nexus_api::Order`] order.
pub(crate) fn slot_vms(plugin: &PluginId, slot: SlotId, items: Vec<SlotItem>) -> Vec<SlotVm> {
    let keyed = items.into_iter().map(|item| (item.id.clone(), item.order.clone(), item)).collect();
    sort_by_order(keyed).into_iter().map(|item| slot_vm(plugin, slot, item)).collect()
}

fn slot_vm(plugin: &PluginId, slot: SlotId, item: SlotItem) -> SlotVm {
    let mut vm = SlotVm {
        slot,
        plugin: plugin.clone(),
        label: format!("{plugin}:{}", item.id),
        id: item.id,
        kind: SlotKind::Badge,
        text: String::new(),
        short: String::new(),
        tone: Tone::Neutral,
        style: ActionStyle::Secondary,
        icon: None,
        rows: Vec::new(),
        action: None,
        args: Value::Null,
        scope: None,
    };
    match item.content {
        SlotContent::Badge { label, short, tone, icon } => {
            vm.short = if short.is_empty() { label.clone() } else { short };
            vm.text = label;
            vm.tone = tone;
            vm.icon = icon.map(|i| image_ref(plugin, &i));
        }
        SlotContent::Action { label, action, args, style, icon } => {
            vm.kind = SlotKind::Action;
            vm.text = label;
            vm.action = Some(action);
            vm.args = args;
            vm.style = style;
            vm.icon = icon.map(|i| image_ref(plugin, &i));
        }
        SlotContent::Section { title, rows } => {
            vm.kind = SlotKind::Section;
            vm.text = title;
            vm.rows = rows.into_iter().map(|r| RowVm { label: r.label, value: r.value }).collect();
        }
        SlotContent::SettingsPanel { title, scope } => {
            vm.kind = SlotKind::SettingsPanel;
            vm.text = title;
            vm.scope = Some(scope);
        }
    }
    vm
}

/// A compatibility level's id and its words.
fn compat_level(level: CompatLevel) -> (&'static str, &'static str) {
    match level {
        CompatLevel::Unknown => ("unknown", "Not tested"),
        CompatLevel::Nothing => ("nothing", "Doesn't start"),
        CompatLevel::Boots => ("boots", "Boots"),
        CompatLevel::Menus => ("menus", "Menus"),
        CompatLevel::Ingame => ("ingame", "In-game"),
        CompatLevel::Playable => ("playable", "Playable"),
    }
}

fn compat_vm(info: &CompatInfo) -> CompatVm {
    let (level, label) = compat_level(info.level);
    CompatVm {
        level: level.into(),
        label: label.into(),
        notes: info.notes.clone().unwrap_or_default(),
        tested: info.tested.clone().unwrap_or_default(),
    }
}

/// The game's square art (see [`LibrarySnapshot::tile`]).
fn cover_of(library: &LibrarySnapshot, entry: &Entry) -> Option<ImageRef> {
    library.tile(entry).map(plugin_image)
}

/// Whether `key`'s game can be uninstalled: its folder is known, isn't a whole player's folder,
/// is in one its emulator may write, and is there.
fn can_uninstall(s: &Snapshot, key: &GameKey) -> bool {
    let folder = s.library.detail(key).and_then(|d| d.details.folder.as_ref());
    folder.is_some_and(|f| {
        !f.path.is_root()
            && s.plugins.manifest(&key.plugin).is_some_and(|m| m.permissions.folder(&f.folder).is_some_and(|g| g.write))
            && s.files.game_folders.contains(key)
    })
}

/// A game's badges, from its plugin.
fn badges_of(library: &LibrarySnapshot, key: &GameKey) -> Vec<SlotVm> {
    slot_vms(&key.plugin, SlotId::GameBadge, library.badges(key).to_vec())
}

/// Whole days since the game was last played; `None` before it was.
fn last_played_days(s: &Snapshot, key: &GameKey) -> Option<u64> {
    let stats = s.sessions.plays.of(key);
    (stats.count > 0).then(|| crate::sessions::days_between(stats.last, s.now_secs))
}

fn card(s: &Snapshot, entry: &Entry) -> GameCardVm {
    let library = &s.library;
    GameCardVm {
        key: entry.key.to_string(),
        title: entry.summary.title.clone(),
        platform: entry.summary.platform.clone(),
        system: s.plugins.system(&entry.summary.platform),
        cover: cover_of(library, entry),
        selected: s.state.selected.as_ref() == Some(&entry.key),
        badges: badges_of(library, &entry.key),
        release_year: entry.summary.release_year,
        rating: library.meta(&entry.key).and_then(|(_, m)| m.rating).map(|r| r.score),
        size_bytes: library.detail(&entry.key).and_then(|d| d.details.size_bytes),
        last_played_days: last_played_days(s, &entry.key),
        releases: 0,
        availability: String::new(),
        hidden: false,
        // A Library game's; `offer_card` gives an offer's.
        can: game_actions(s, &entry.key, None),
    }
}

/// What can be done with the game `key`: a Library game (`offer` is `None`), or a Catalog
/// offer with the installed game it is, if any. The one place the rules live:
///
/// - Play: it is installed (an offer: the installed game it is), and that game's plugin can
///   start games. A missing emulator is no reason to hide it: Play installs the emulator first.
/// - Get: an offer that isn't installed. A release its store installs asks the store
///   ("Install in Steam"); a torrent's game downloads, from a release the player picks.
/// - Uninstall: the installed game, by its store's link when its plugin gives one and may open
///   it, else by deleting its folder when its plugin may.
/// - The main action: Play, else Get, else none. Uninstall is never the main one.
fn game_actions(
    s: &Snapshot,
    key: &GameKey,
    offer: Option<(&crate::releases::Offer, Option<&Entry>)>,
) -> GameActionsVm {
    let installed: Option<&GameKey> = match offer {
        None => Some(key),
        Some((_, game)) => game.map(|g| &g.key),
    };
    let manifest = |plugin: &PluginId| s.plugins.manifest(plugin);
    let play =
        installed.is_some_and(|k| manifest(&k.plugin).is_some_and(|m| m.capabilities.contains(&Capability::Launcher)));
    let mut actions = GameActionsVm { play, ..GameActionsVm::default() };
    if let Some(game) = installed {
        let details = s.library.detail(game).map(|d| &d.details);
        let link = details.and_then(|d| d.uninstall_link.as_deref());
        let store_removes = link.is_some_and(|l| manifest(&game.plugin).is_some_and(|m| m.permissions.allows_link(l)));
        if store_removes {
            let platform = details.map(|d| d.summary.platform.as_str()).unwrap_or_default();
            actions.uninstall = game.to_string();
            actions.uninstall_store = s.plugins.system(platform).name;
        } else if can_uninstall(s, game) {
            actions.uninstall = game.to_string();
        }
    }
    if let Some((offer, None)) = offer {
        match offer.releases.iter().find(|r| !r.store.is_empty()) {
            Some(release) => {
                actions.get_store = s.plugins.system(&release.platform).name;
                actions.get = format!("Install in {}", actions.get_store);
                actions.get_release = release.id.clone();
            }
            None if !offer.releases.is_empty() => actions.get = "Download".into(),
            None => {}
        }
    }
    actions.primary = if actions.play {
        "play".into()
    } else if !actions.get.is_empty() {
        "get".into()
    } else {
        String::new()
    };
    actions
}

fn grid_vm(s: &Snapshot, data: &GridData, query: &GridQuery) -> LibraryVm {
    let GridGames { shown, all, systems, genres } = query.games(data);
    let library = data.library;
    let offers: std::collections::HashMap<&GameKey, &crate::releases::Offer> =
        library.offers.iter().map(|o| (&o.entry.key, o)).collect();
    let total = shown.len();
    let games: Vec<GameCardVm> = shown
        .into_iter()
        .take(query.shown)
        .map(|e| match offers.get(&e.key) {
            Some(offer) if query.grid == Grid::Catalog => {
                GameCardVm { hidden: library.hidden(&e.key), ..offer_card(s, data.ownership, offer) }
            }
            _ => card(s, e),
        })
        .collect();
    LibraryVm {
        selected: games.iter().position(|g| g.selected),
        games,
        total,
        all,
        systems,
        genres,
        sort: query.sort.as_str().into(),
        query: query.query.clone(),
    }
}

/// What clicking `notice` opens, for example "Open Downloads"; empty for nothing.
fn notice_opens(plugins: &Plugins, notice: &crate::notices::Notice) -> String {
    match &notice.route {
        Some(Route::Downloads) => "Open Downloads".into(),
        Some(Route::Catalog) => "Open the Catalog".into(),
        Some(Route::GameDetail(_)) => "Open the Game Hub".into(),
        Some(Route::Plugin(plugin)) => {
            format!("Open {}", plugins.manifest(plugin).map(|m| m.name.clone()).unwrap_or_default())
        }
        Some(Route::Settings) => "Open Settings".into(),
        _ => String::new(),
    }
}

/// The cover of the game `notice` is about.
fn notice_art(library: &LibrarySnapshot, notice: &crate::notices::Notice) -> Option<ImageRef> {
    let game = notice.game.as_ref()?;
    library.listed(game).and_then(|e| cover_of(library, e))
}

fn notice_vm(s: &Snapshot, notice: &crate::notices::Notice) -> NoticeVm {
    NoticeVm {
        id: notice.id,
        text: notice.message.clone(),
        kind: notice.kind,
        art: notice_art(&s.library, notice),
        opens: notice_opens(&s.plugins, notice),
    }
}

/// The view models for the snapshot `s`.
pub(crate) fn build(s: &Snapshot) -> AppView {
    let state = &s.state;
    let route = state.nav.last().cloned().unwrap_or(Route::Home);
    let (screen, game) = match &route {
        Route::Home => (ScreenId::Home, None),
        Route::GameDetail(g) => (ScreenId::GameDetail, Some(g.clone())),
        Route::Catalog => (ScreenId::Catalog, None),
        Route::Downloads => (ScreenId::Downloads, None),
        Route::Settings => (ScreenId::Settings, None),
        Route::Emulators => (ScreenId::Emulators, None),
        Route::Themes => (ScreenId::Themes, None),
        Route::Plugin(_) => (ScreenId::Plugin, None),
        Route::GameSettings(g) => (ScreenId::GameSettings, Some(g.clone())),
        Route::Running(_) => (ScreenId::Running, None),
    };
    let owned = s.ownership();
    let data = s.grid_data(&owned);
    let notices = &s.notices;
    AppView {
        screen,
        game: game.map(|g| g.to_string()),
        can_go_back: state.nav.len() > 1,
        home: home_vm(s, &data),
        // The detail page of the game the player last opened stays available under overlays.
        // With no detail page open, the selected game's details, once loaded.
        detail: s.detail_game.as_ref().and_then(|key| detail_vm(s, &owned, key)),
        catalog: catalog_vm(s, &data, route == Route::Catalog),
        downloads: downloads_vm(s, &owned),
        settings: settings_vm(s),
        emulators: emulators_vm(s),
        plugin_page: match &route {
            Route::Plugin(plugin) => plugin_page_vm(s, plugin),
            _ => None,
        },
        running: running_vm(s, &route),
        notices: notices.iter().map(|n| n.message.clone()).collect(),
        notice_cards: notices.iter().map(|n| notice_vm(s, n)).collect(),
        install_prompt: state.install_prompt.as_ref().map(|p| InstallPromptVm {
            plugin: p.game.plugin.clone(),
            emulator: p.emulator.clone(),
            version: p.version.clone(),
            game: p.title.clone(),
            size_bytes: p.size_bytes,
        }),
        boot: boot_vm(s),
        task: s.library.task.clone(),
        notice_opens: notices.last().map(|n| notice_opens(&s.plugins, n)).unwrap_or_default(),
        notice_art: notices.last().and_then(|n| notice_art(&s.library, n)),
        system: system_vm(s),
        result_prompt: state.result_prompt.as_ref().map(|(key, _)| s.library.title(key)),
    }
}

/// A badge the core gives a card itself.
fn badge(plugin: &PluginId, id: &str, text: &str, tone: Tone) -> SlotVm {
    slot_vm(
        plugin,
        SlotId::GameBadge,
        SlotItem {
            id: id.into(),
            order: Default::default(),
            content: SlotContent::Badge { label: text.into(), short: String::new(), tone, icon: None },
        },
    )
}

fn offer_card(s: &Snapshot, owned: &Ownership, offer: &crate::releases::Offer) -> GameCardVm {
    let mut card = card(s, &offer.entry);
    card.selected = s.state.catalog_selected.as_ref() == Some(&offer.entry.key);
    let best = owned.best_release(offer);
    card.size_bytes = best.size_bytes;
    card.availability = crate::releases::availability(best.seeders).into();
    card.releases = offer.releases.len();
    let installed = owned.installed_game(offer);
    card.can = game_actions(s, &offer.entry.key, Some((offer, installed)));
    // A game its store installs has no torrent to be available.
    if offer.releases.iter().any(|r| !r.store.is_empty()) {
        card.availability = String::new();
    }
    let plugin = &offer.entry.key.plugin;
    if offer.releases.iter().any(|r| r.favorite) {
        card.badges.push(badge(plugin, "favorite", "Favorite", Tone::Info));
    }
    if installed.is_some() {
        card.badges.push(badge(plugin, "installed", "Installed", Tone::Success));
    } else if offer.releases.iter().any(|r| crate::releases::is_new(&r.added, s.today())) {
        card.badges.push(badge(plugin, "new", "New", Tone::Info));
    }
    // Every release is for another system.
    let here = s.plugins.host().platform;
    if offer.releases.iter().all(|r| !crate::releases::not_here(&r.system, here).is_empty()) {
        card.badges.push(badge(plugin, "not-here", "Can't run here", Tone::Warning));
    }
    card
}

/// The Catalog. Only the Catalog shows its grid: `on_screen` off, the last one stands.
fn catalog_vm(s: &Snapshot, data: &GridData, on_screen: bool) -> CatalogVm {
    let library = &s.library;
    let sources = library
        .catalogs
        .iter()
        .map(|c| {
            let l = &c.list;
            let copy = match l.source {
                nexus_api::ReleaseSource::Fetched => format!("fetched from {}", l.host),
                nexus_api::ReleaseSource::Cached => format!("the copy fetched before from {}", l.host),
                nexus_api::ReleaseSource::Bundled => "the bundled catalog".into(),
            };
            let mut parts = vec![format!("{}: {copy}", s.plugins.name(&c.emulator))];
            if !l.version.is_empty() {
                parts.push(format!("version {}", l.version));
            }
            if !l.updated.is_empty() {
                parts.push(format!("updated {}", l.updated.get(..10).unwrap_or(&l.updated)));
            }
            parts.push(if l.releases.len() == 1 {
                "1 release".into()
            } else {
                format!("{} releases", l.releases.len())
            });
            if let Some(problem) = &l.problem {
                parts.push(format!("the fetch failed: {problem}"));
            }
            parts.join(" · ")
        })
        .collect();
    let releases = library.catalogs.iter().flat_map(|c| &c.list.releases);
    // The releases of the games the Catalog has, not of the ones the player hid.
    let shown_releases: usize =
        library.offers.iter().filter(|o| !library.hidden(&o.entry.key)).map(|o| o.releases.len()).sum();
    // The newest count of seeders, else the newest date a catalog gives.
    let counted =
        releases.clone().filter_map(|r| crate::releases::days_of(&r.counted_at).map(|_| &r.counted_at[..10])).max();
    let dated = || {
        library
            .catalogs
            .iter()
            .filter_map(|c| crate::releases::days_of(&c.list.updated).map(|_| &c.list.updated[..10]))
            .max()
    };
    let games = s.state.catalog_grid.get(on_screen).unwrap_or_else(|| grid_vm(s, data, &s.state.catalog_view));
    let buttons = |kind: SlotKind| -> Vec<SlotVm> {
        let all = library.catalog_buttons.iter();
        all.flat_map(|(plugin, items)| slot_vms(plugin, SlotId::CatalogAction, items.clone()))
            .filter(|s| s.kind == kind)
            .collect()
    };
    CatalogVm {
        available: s.plugins.catalog_available(),
        games,
        sources,
        releases: shown_releases,
        updated: counted.or_else(dated).unwrap_or_default().to_owned(),
        loading: !library.catalogs_read,
        actions: buttons(SlotKind::Action),
        status: buttons(SlotKind::Badge),
    }
}

fn home_vm(s: &Snapshot, data: &GridData) -> HomeVm {
    let games: Vec<GameCardVm> = s.library.library().map(|e| card(s, e)).collect();
    let selected = games.iter().position(|g| g.selected);
    let mut groups: Vec<GroupVm> = Vec::new();
    for game in &games {
        match groups.iter_mut().find(|g| g.platform == game.platform) {
            Some(group) => group.games.push(game.clone()),
            None => groups.push(GroupVm { platform: game.platform.clone(), games: vec![game.clone()], selected: None }),
        }
    }
    for group in &mut groups {
        group.selected = group.games.iter().position(|g| g.selected);
    }
    let grid = grid_vm(s, data, &s.state.library_view);
    HomeVm { games, selected, loading: s.library.loading, groups, library: grid }
}

/// A plugin's settings panel, with its fields.
fn settings_panel_vm(panel: &crate::app::SettingsPanel) -> SettingsPanelVm {
    let slot = slot_vm(&panel.plugin, SlotId::SettingsPanel, panel.item.clone());
    SettingsPanelVm { fields: crate::settings_schema::fields(&panel.schema, &panel.values), slot }
}

/// A game's own settings, as its Game Hub shows them.
fn game_settings_vm(plugins: &Plugins, key: &GameKey, settings: &crate::library::GameSettings) -> SettingsPanelVm {
    let plugin = &key.plugin;
    let name = plugins.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_default();
    let slot = SlotVm {
        slot: SlotId::SettingsPanel,
        plugin: plugin.clone(),
        id: "game".into(),
        label: format!("{plugin}:game"),
        kind: SlotKind::SettingsPanel,
        text: format!("{name} settings for this game"),
        short: String::new(),
        tone: Tone::Neutral,
        style: ActionStyle::Secondary,
        icon: None,
        rows: Vec::new(),
        action: None,
        args: Value::Null,
        scope: Some(nexus_api::ConfigScope::Game(key.local.clone())),
    };
    SettingsPanelVm { fields: crate::settings_schema::fields(&settings.schema, &settings.values), slot }
}

fn detail_vm(s: &Snapshot, owned: &Ownership, key: &GameKey) -> Option<GameDetailVm> {
    let (library, torrents) = (&s.library, &s.torrents);
    let d = library.detail(key)?;
    let details = &d.details;
    let summary = &details.summary;
    let manifest = s.plugins.manifest(&key.plugin);
    let offer = library.offer(key);
    let has_installer = manifest.is_some_and(|m| m.capabilities.contains(&Capability::Installer));
    let installed = s.state.emulators.get(&key.plugin).is_some_and(|e| !e.installed.is_empty());
    // A Catalog offer plays as the installed game it is, and not at all before it is installed.
    let installed_as = offer.and_then(|o| owned.installed_game(o));
    let played = installed_as.map(|g| &g.key).unwrap_or(key);
    let can = game_actions(s, key, offer.map(|o| (o, installed_as)));
    let needs_emulator = has_installer && !installed;
    // The program, not the console: "Launches with the KytyPS5 emulator".
    let name = manifest.map(|m| m.emulator_name().to_owned()).unwrap_or_default();
    let stats = s.sessions.plays.of(key);
    let meta = library.meta(key);
    // A release's store data fills in, field by field, what the store's catalog leaves out.
    let metas = library.metadata(key);
    let text = |field: fn(&nexus_api::GameMetadata) -> &String| {
        metas.iter().map(|(_, m)| field(m)).find(|t| !t.is_empty()).cloned().unwrap_or_default()
    };
    let shown = library.art(key);
    // The game's own words first; a store's catalog fills in what they leave out.
    let description =
        if details.description.is_empty() { text(|m| &m.description) } else { details.description.clone() };
    let developer = details
        .developer
        .clone()
        .or_else(|| Some(text(|m| &m.publisher)).filter(|p| !p.is_empty()))
        .unwrap_or_default();
    let genres = if details.genres.is_empty() {
        metas.iter().map(|(_, m)| &m.genres).find(|g| !g.is_empty()).cloned().unwrap_or_default()
    } else {
        details.genres.clone()
    };
    // Home's line, as the old launcher: the store's tagline, else the game's own words, else its
    // facts on one line, else where it runs.
    let tagline = meta.map(|(_, m)| m.tagline.clone()).filter(|t| !t.is_empty());
    let version = details.facts.iter().find(|f| f.id == "version").map(|f| f.value.clone()).unwrap_or_default();
    let facts_line = [
        summary.title.clone(),
        genres.join(", "),
        Some(developer.clone()).filter(|d| !d.is_empty()).map(|d| format!("Developer: {d}")).unwrap_or_default(),
        Some(version).filter(|v| !v.is_empty()).map(|v| format!("Version: {v}")).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>();
    let line = match tagline {
        Some(tagline) => tagline,
        None if !details.description.is_empty() => details.description.clone(),
        None if facts_line.len() > 1 => facts_line.join(" · "),
        None => match offer {
            Some(o) => format!("In the catalog for {}.", s.plugins.name(&o.emulator)),
            None => format!("Installed locally. Launches with the {name} emulator."),
        },
    };
    let compatibility = offer.map(crate::releases::compatibility).unwrap_or_default();
    // Where the offer's downloads go, and the free space there.
    let download_folder = offer.and_then(|o| s.state.disks.download_folder(&o.emulator, &o.releases[0].platform));
    let free = download_folder.and_then(|f| f.free);
    let no_room = |r: &nexus_api::Release| match (r.size_bytes, free) {
        (Some(size), Some(free)) if !torrents.jobs.contains_key(&r.id) => {
            crate::torrents::no_room(size, free, "download folder").unwrap_or_default()
        }
        _ => String::new(),
    };
    let releases = offer.map_or(&[][..], |o| &o.releases[..]);
    let (this_pc, pc_warning) = on_this_pc(s, offer.map_or(&key.plugin, |o| &o.emulator), releases);
    // A game that starts here but doesn't fit says so instead, from its best release.
    let this_pc = match (this_pc, offer.map(|o| no_room(owned.best_release(o))), free) {
        (Some(v), Some(short), Some(free)) if v.level != "no" && !short.is_empty() => Some(VerdictVm {
            level: "no".into(),
            headline: "Not enough space".into(),
            why: format!("{} free", crate::torrents::gigabytes(free)),
            ..v
        }),
        (this_pc, _, _) => this_pc,
    };
    let home = s.plugins.host().home.as_ref().and_then(|h| h.to_str());
    Some(GameDetailVm {
        key: key.to_string(),
        title: summary.title.clone(),
        platform: summary.platform.clone(),
        system: s.plugins.system(&summary.platform),
        summary: summary_of(&line),
        description,
        developer,
        genres,
        cover: shown.cover.map(plugin_image),
        background: shown.background.map(plugin_image),
        logo: shown.logo.map(plugin_image),
        rating: meta.and_then(|(_, m)| m.rating),
        release: text(|m| &m.release),
        age_rating: meta.map(|(_, m)| m.age_rating.clone()).unwrap_or_default(),
        screenshots: shown.screenshots.into_iter().map(plugin_image).collect(),
        store_url: text(|m| &m.store_url),
        protondb: compatibility.0,
        protondb_url: compatibility.1,
        this_pc,
        pc_warning,
        your_result: library.result(key).unwrap_or_default().to_owned(),
        has_folder: s.files.game_folders.contains(key),
        has_log: s.files.has_log,
        hidden: library.hidden(key),
        compat_page: d.compat.as_ref().and_then(|c| c.page.clone()).unwrap_or_default(),
        trailer_url: meta.map(|(_, m)| m.trailer.clone()).unwrap_or_default(),
        compat: d.compat.as_ref().map(compat_vm),
        badges: badges_of(library, key),
        actions: slot_vms(&key.plugin, SlotId::GameAction, d.actions.clone()),
        sections: slot_vms(&key.plugin, SlotId::GameDetailSection, d.sections.clone()),
        can: can.clone(),
        settings: d.settings.iter().map(|settings| game_settings_vm(&s.plugins, key, settings)).collect(),
        launch_hint: if can.play && needs_emulator { format!("Play installs {name} first") } else { String::new() },
        running: s.sessions.runs(played),
        size_bytes: details.size_bytes,
        facts: details.facts.clone(),
        played_secs: stats.secs,
        last_played_days: last_played_days(s, key),
        // No plugin reports a shader cache: the plugin API has no seam for one.
        shader_cache: None,
        releases: offer
            .map(|o| {
                let today = s.today();
                let best = &owned.best_release(o).id;
                // By version, highest first; of one version, the newest upload first. A release
                // its store installs is no use once the game is installed.
                let mut ordered: Vec<&nexus_api::Release> =
                    o.releases.iter().filter(|r| r.store.is_empty() || installed_as.is_none()).collect();
                ordered.sort_by_cached_key(|r| std::cmp::Reverse(crate::releases::version_key(&r.version)));
                let count = ordered.len();
                ordered
                    .into_iter()
                    .enumerate()
                    .map(|(at, r)| ReleaseVm {
                        version_label: format!(
                            "Version {} of {count}{}",
                            count - at,
                            if at == 0 { " (newest)" } else { "" }
                        ),
                        best: &r.id == best,
                        seeders: r.seeders,
                        leechers: r.leechers,
                        counted_at: r.counted_at.clone(),
                        availability: crate::releases::availability(r.seeders).into(),
                        id: r.id.clone(),
                        store: if r.store.is_empty() { String::new() } else { s.plugins.system(&r.platform).name },
                        version: r.version.clone(),
                        region: r.region.clone(),
                        size_bytes: r.size_bytes,
                        added: r.added.clone(),
                        new: crate::releases::is_new(&r.added, today),
                        description: r.description.clone(),
                        system: crate::releases::system_label(&r.system).into(),
                        not_here: crate::releases::not_here(&r.system, s.plugins.host().platform),
                        warning: r.install_warning.clone().unwrap_or_default(),
                        no_room: no_room(r),
                        download: torrents.jobs.get(&r.id).map(|j| j.state.as_str().to_owned()).unwrap_or_default(),
                        download_progress: torrents
                            .jobs
                            .get(&r.id)
                            .map(|j| if j.total > 0 { (j.done as f64 / j.total as f64).min(1.0) as f32 } else { 0.0 })
                            .unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        emulator: offer
            .map(|o| {
                s.plugins.manifest(&o.emulator).map_or_else(|| o.emulator.to_string(), |m| m.emulator_name().to_owned())
            })
            .unwrap_or_default(),
        download_folder: download_folder.map(|f| short_home(&f.path.display().to_string(), home)).unwrap_or_default(),
        availability: offer
            .map(|o| crate::releases::availability(owned.best_release(o).seeders).to_owned())
            .unwrap_or_default(),
        download_error: torrents
            .download_error
            .as_ref()
            .filter(|(game, _)| game == key)
            .map(|(_, why)| why.clone())
            .unwrap_or_default(),
    })
}

/// How `emulator`'s games run on this PC, and why they won't start, if they won't. An offer
/// whose `releases` are all for another system won't start, whatever the emulator needs.
/// `None` before the check, for a plugin that isn't an emulator, and when the system can't say.
fn on_this_pc(s: &Snapshot, emulator: &PluginId, releases: &[nexus_api::Release]) -> (Option<VerdictVm>, String) {
    let Some(manifest) = s.plugins.manifest(emulator).filter(|m| m.kind == PluginKind::Emulator) else {
        return (None, String::new());
    };
    let here = s.plugins.host().platform;
    if !releases.is_empty() && releases.iter().all(|r| !crate::releases::not_here(&r.system, here).is_empty()) {
        let mut systems: Vec<&str> = Vec::new();
        for label in releases.iter().map(|r| crate::releases::system_label(&r.system)) {
            if !systems.contains(&label) {
                systems.push(label);
            }
        }
        let why = format!("{} release", systems.join(" or "));
        // The release rows say why, so there is no warning for the whole emulator.
        let verdict = crate::hardware::Verdict::No(why);
        return (
            Some(crate::hardware::verdict_vm(emulator.to_string(), manifest.name.clone(), verdict)),
            String::new(),
        );
    }
    let Some(specs) = &s.state.hardware else { return (None, String::new()) };
    let verdict = crate::hardware::verdict(specs, &manifest.requires);
    let warning = match &verdict {
        crate::hardware::Verdict::Unknown(_) => return (None, String::new()),
        crate::hardware::Verdict::No(why) => crate::hardware::blocked_notice(&[(manifest.name.clone(), why.clone())]),
        _ => None,
    };
    (
        Some(crate::hardware::verdict_vm(emulator.to_string(), manifest.name.clone(), verdict)),
        warning.unwrap_or_default(),
    )
}

fn downloads_vm(s: &Snapshot, owned: &Ownership) -> DownloadsVm {
    let (torrents, installs) = (&s.torrents, &s.installs);
    // Game installs, newest first, then the downloads. A catalog download's install shows on
    // the download's own card (`torrent_vm`), so it has no card of its own.
    let on_a_card = |j: &crate::game_installs::GameJob| {
        torrents.jobs.values().any(|t| t.emulator == j.plugin && t.source == j.source)
    };
    let install_jobs = installs.jobs.iter().rev().filter(|(_, j)| !on_a_card(j)).map(|(id, j)| DownloadVm {
        id: id.0,
        plugin: j.plugin.clone(),
        label: j.label.clone(),
        progress: if j.phase == crate::game_installs::GamePhase::Done {
            1.0
        } else if j.total > 0 {
            (j.done as f32 / j.total as f32).min(1.0)
        } else {
            0.0
        },
        received: j.done,
        total: (j.total > 0).then_some(j.total),
        state: j.phase.as_str().into(),
        error: j.error.clone().unwrap_or_default(),
        can_pause: false,
        can_resume: false,
        can_cancel: j.phase == crate::game_installs::GamePhase::Installing,
    });
    let downloads = s.downloads.iter().rev().cloned().map(|j| DownloadVm {
        id: j.id.0,
        progress: match j.total {
            Some(t) if t > 0 => (j.received as f32 / t as f32).min(1.0),
            _ => 0.0,
        },
        plugin: j.plugin,
        label: j.label,
        received: j.received,
        total: j.total,
        state: j.phase.as_str().into(),
        error: j.error.unwrap_or_default(),
        can_pause: matches!(j.phase, JobPhase::Queued | JobPhase::Downloading),
        can_resume: j.phase == JobPhase::Paused,
        can_cancel: !j.phase.is_over(),
    });
    let jobs = install_jobs.chain(downloads).collect();
    let home = s.plugins.host().home.as_ref().and_then(|h| h.to_str()).map(str::to_owned);
    let mut cards: Vec<TorrentVm> = torrents
        .jobs
        .values()
        .map(|j| TorrentVm {
            folder: short_home(&j.folder.display().to_string(), home.as_deref()),
            install_note: install_note(&s.plugins, &j.emulator),
            install_folder: s
                .files
                .install_folders
                .get(&j.emulator)
                .map(|path| short_home(&path.display().to_string(), home.as_deref()))
                .unwrap_or_default(),
            ..torrent_vm(&s.library, installs, owned, j)
        })
        .collect();
    cards.reverse();
    // The installs stores do (Steam's), after the downloads: the same cards, with the store's
    // own page as their one action.
    cards.extend(s.state.store_installs.iter().map(|(plugin, install)| store_install_vm(&s.plugins, plugin, install)));
    let downloading = torrents.jobs.values().filter(|j| j.state.active()).count();
    let sharing = torrents.jobs.values().filter(|j| j.seeding).count();
    DownloadsVm { jobs, torrents: cards, seeding: torrents.sharing, downloading, sharing }
}

/// A store's install as a Downloads card: its phase and bytes, and the store's page.
fn store_install_vm(plugins: &Plugins, plugin: &PluginId, install: &nexus_api::StoreInstall) -> TorrentVm {
    use nexus_api::StorePhase as P;
    let store = plugins
        .manifest(plugin)
        .and_then(|m| m.system.as_ref().map(|s| s.name.clone()))
        .unwrap_or_else(|| plugin.to_string());
    let part =
        if install.total > 0 { (install.done as f64 / install.total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let installing = install.phase == P::Installing;
    TorrentVm {
        id: format!("store:{plugin}:{}", install.release),
        title: install.title.clone(),
        state: match install.phase {
            P::Queued => "queued",
            P::Downloading | P::Installing => "downloading",
            P::Paused => "paused",
        }
        .into(),
        progress: if installing { 1.0 } else { part },
        done: if installing { install.total } else { install.done },
        total: install.total,
        install: if installing { "installing".into() } else { String::new() },
        install_progress: if installing { part } else { 0.0 },
        actions: if install.open.is_empty() { Vec::new() } else { vec!["open-store".into()] },
        store,
        ..TorrentVm::default()
    }
}

/// What `emulator` installs from, as its manifest says, then what the host does first.
fn install_note(plugins: &Plugins, emulator: &PluginId) -> String {
    let own = plugins.manifest(emulator).map(|m| m.install_note.clone()).unwrap_or_default();
    let archives =
        "Nexus first unpacks the RAR, ZIP, 7z and TAR archives in the download, but not one with a password.";
    if own.is_empty() { archives.to_owned() } else { format!("{own} {archives}") }
}

/// Sizes for file lists, as the theme's other sizes: `1.5 GB`, `340 MB`.
fn bytes_label(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

fn torrent_vm(library: &LibrarySnapshot, installs: &InstallsSnapshot, owned: &Ownership, j: &TorrentJob) -> TorrentVm {
    use crate::torrents::TorrentState as S;
    let install = installs.latest(&j.emulator, &j.source);
    let installing = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Installing);
    let installed = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Done);
    // The installed game: the one this download's install reported, else the offer's, as its
    // Catalog card finds it (by title id, or by name: a PC game has no title id). The install's
    // record is gone after a restart.
    let offer = library.offer(&j.offer);
    let game = owned
        .installed_from(j)
        .or_else(|| offer.and_then(|o| owned.installed_game(o)))
        .map(|e| e.key.to_string())
        .unwrap_or_default();
    use crate::archives::UnpackPhase as U;
    let unpacking = j.hand_off.unpack.phase == U::Unpacking;
    let mut actions: Vec<&str> = Vec::new();
    match j.state {
        S::Ready | S::Paused | S::Failed | S::Cancelled => actions.push("start"),
        S::Checking | S::Downloading => actions.push("pause"),
        _ => {}
    }
    if j.state == S::Complete && !installing && !unpacking {
        actions.push(if !game.is_empty() { "play" } else { "install" });
    }
    if j.seeding || j.verifying {
        actions.push("stop-seeding");
    }
    // While its archives unpack, Cancel stops the unpacking.
    if !matches!(j.state, S::Complete | S::Cancelled) || unpacking {
        actions.push("cancel");
    } else if !j.seeding && !j.verifying && !installing {
        actions.push("remove");
    }
    if j.state == S::Complete && j.hand_off.unpack.phase == U::Unpacked && !installing {
        actions.push("remove-unpacked");
    }
    if installing {
        actions.push("cancel-install");
    }
    let unpack_error =
        (j.hand_off.unpack.phase == U::Failed).then(|| j.hand_off.unpack.error.clone()).filter(|e| !e.is_empty());
    TorrentVm {
        store: String::new(),
        id: j.id.clone(),
        title: j.title.clone(),
        version: j.version.clone(),
        region: j.region.clone(),
        state: j.state.as_str().into(),
        progress: if j.state == S::Complete {
            1.0
        } else if j.total > 0 {
            (j.done as f64 / j.total as f64).clamp(0.0, 1.0) as f32
        } else {
            0.0
        },
        done: j.done,
        total: j.total,
        download_rate: j.download_rate,
        upload_rate: j.upload_rate,
        seconds_left: match j.state {
            S::Downloading if j.download_rate > 0 => j.total.saturating_sub(j.done).div_ceil(j.download_rate),
            _ => 0,
        },
        peers: j.peers,
        seeding: j.seeding,
        verifying: j.verifying,
        waiting_vpn: j.waiting_vpn,
        error: install.and_then(|g| g.error.clone()).or(unpack_error).unwrap_or_else(|| j.error.clone()),
        files: j.files.iter().map(|f| format!("{} · {}", f.path, bytes_label(f.size))).collect(),
        file_count: j.file_count,
        folder: j.folder.display().to_string(),
        actions: actions.into_iter().map(str::to_owned).collect(),
        install: install.map(|g| g.phase.as_str().to_owned()).unwrap_or_default(),
        unpack: j.hand_off.unpack.phase.as_str().into(),
        unpack_progress: j.hand_off.unpack.progress,
        install_progress: install
            .map(|g| if g.total > 0 { (g.done as f64 / g.total as f64).min(1.0) as f32 } else { 0.0 })
            .unwrap_or(0.0),
        game: if installed || install.is_none() { game } else { String::new() },
        install_note: String::new(),
        install_folder: String::new(),
        install_error: j.hand_off.install_error.clone(),
    }
}

/// Wi-Fi signal (0 to 100) in bars, 0 to 4.
fn bars(signal: u8) -> i32 {
    match signal {
        0..=19 => 1,
        20..=44 => 2,
        45..=69 => 3,
        _ => 4,
    }
}

/// A job's text for the player, and a finished install's flags: its plugin reports how the
/// install ended as [`nexus_api::Installed`] JSON.
fn job_text(job: &crate::system::JobState) -> nexus_ui_api::SystemJobVm {
    let mut vm = nexus_ui_api::SystemJobVm {
        id: 0,
        kind: String::new(),
        target: String::new(),
        state: String::new(),
        percent: -1,
        text: job.text.clone(),
        mok: false,
        nvidia_later: false,
    };
    if job.kind == crate::system::JobKind::InstallOs
        && job.phase == crate::system::JobPhase::Done
        && let Ok(installed) = serde_json::from_str::<nexus_api::Installed>(&job.text)
    {
        vm.text = "Nexus OS is installed".into();
        vm.mok = installed.mok;
        vm.nvidia_later = installed.nvidia_later;
    }
    vm
}

fn system_vm(s: &Snapshot) -> nexus_ui_api::SystemVm {
    use nexus_api::{Capability, Connection, OsUpdate};
    use nexus_ui_api::{AudioOutputVm, BluetoothDeviceVm, InstallDiskVm, SystemJobVm, WifiVm};
    let system = &s.state.system;
    let place = s.plugins.host().place;
    let reason = |c: Capability| system.unavailable(c);
    let network = system.network.clone().unwrap_or_default();
    let (os_update, os_update_text) = match &system.os_update {
        None => (String::new(), String::new()),
        Some(OsUpdate::Current) => ("current".into(), "Nexus OS is up to date".into()),
        Some(OsUpdate::Checking) => ("checking".into(), "Looking for an update…".into()),
        Some(OsUpdate::Staged { version }) => ("staged".into(), format!("Restart to update to {version}")),
        Some(OsUpdate::Failed { reason }) => ("failed".into(), format!("Couldn't look for an update: {reason}")),
    };
    let level = system.level.unwrap_or_default();
    nexus_ui_api::SystemVm {
        whole_screen: place.whole_screen,
        nexus_os: place.nexus_os,
        stick: place.stick,
        network_available: reason(Capability::Network).is_none(),
        network_reason: reason(Capability::Network).unwrap_or_default(),
        connection: match network.connection {
            Connection::None => "none",
            Connection::Wired => "wired",
            Connection::Wifi => "wifi",
        }
        .into(),
        network_name: network.name,
        signal_bars: if network.connection == Connection::Wifi { network.signal.map_or(0, bars) } else { 0 },
        wifi: system
            .wifi
            .iter()
            .map(|w| WifiVm {
                ssid: w.ssid.clone(),
                bars: bars(w.signal),
                secured: w.secured,
                saved: w.saved,
                active: w.active,
            })
            .collect(),
        audio_available: reason(Capability::Audio).is_none(),
        audio_reason: reason(Capability::Audio).unwrap_or_default(),
        volume: level.volume as i32,
        muted: level.muted,
        outputs: system
            .outputs
            .iter()
            .map(|o| AudioOutputVm { name: o.name.clone(), description: o.description.clone(), current: o.current })
            .collect(),
        bluetooth_available: reason(Capability::Bluetooth).is_none(),
        bluetooth_reason: reason(Capability::Bluetooth).unwrap_or_default(),
        bluetooth_on: system.bluetooth_on,
        connected_devices: system.devices.iter().filter(|d| d.connected).count() as i32,
        devices: system
            .devices
            .iter()
            .map(|d| BluetoothDeviceVm {
                address: d.address.clone(),
                name: d.name.clone(),
                paired: d.paired,
                connected: d.connected,
            })
            .collect(),
        power_available: reason(Capability::Power).is_none(),
        power_reason: reason(Capability::Power).unwrap_or_default(),
        battery_shown: system.battery.is_some(),
        battery_percent: system.battery.map_or(0, |b| b.percent as i32),
        battery_charging: system.battery.is_some_and(|b| b.charging),
        os_update,
        os_update_text,
        disks: system
            .disks
            .iter()
            .map(|d| InstallDiskVm {
                path: d.path.display().to_string(),
                size: format!("{} GB", d.bytes / 1_000_000_000),
                windows: d.windows,
                usb: d.usb,
                model: d.model.clone(),
            })
            .collect(),
        jobs: system
            .jobs
            .iter()
            .map(|j| SystemJobVm {
                id: j.id as i32,
                kind: j.kind.name().into(),
                target: j.target.clone(),
                state: match j.phase {
                    crate::system::JobPhase::Running => "running",
                    crate::system::JobPhase::Done => "done",
                    crate::system::JobPhase::Failed => "failed",
                }
                .into(),
                percent: j.percent.map_or(-1, i32::from),
                ..job_text(j)
            })
            .collect(),
    }
}

fn launcher_update_vm(update: &crate::launcher_update::LauncherUpdateSnapshot) -> nexus_ui_api::LauncherUpdateVm {
    use crate::launcher_update::UpdateState;
    let current = update.current.to_owned();
    let (state, version, text) = match &update.state {
        _ if update.off => ("off", String::new(), "Updates are off: no release repo is set".to_owned()),
        UpdateState::Off => ("off", String::new(), "Updates are off: no release repo is set".to_owned()),
        UpdateState::Idle => ("idle", String::new(), "Check for a newer Nexus".to_owned()),
        UpdateState::Checking => ("checking", String::new(), "Checking…".to_owned()),
        UpdateState::UpToDate => ("current", String::new(), format!("Nexus {current} is the newest")),
        UpdateState::Available { version, .. } => {
            ("available", version.clone(), format!("Nexus {version} is out · download it"))
        }
        UpdateState::Downloading { version } => {
            ("downloading", version.clone(), format!("Downloading Nexus {version}…"))
        }
        UpdateState::Ready { version } => {
            ("ready", version.clone(), format!("Nexus {version} is ready · restart to update"))
        }
        UpdateState::Installed { version } => {
            ("installed", version.clone(), format!("Nexus {version} is installed · restart Nexus to start it"))
        }
        UpdateState::WithSystem { version } => {
            ("system", version.clone(), format!("Nexus {version} is out · it comes with the next system update"))
        }
        UpdateState::Failed(why) => ("failed", String::new(), format!("Couldn't update: {why}")),
    };
    nexus_ui_api::LauncherUpdateVm { state: state.into(), current, version, text, method: update.method.to_owned() }
}

fn settings_vm(s: &Snapshot) -> SettingsVm {
    let (plugins, state, prefs) = (&s.plugins, &s.state, &s.prefs);
    let manifests: Vec<_> = plugins
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter_map(|id| {
            plugins.manifest(&id).filter(|m| m.capabilities.contains(&Capability::Launcher)).map(|m| (id, m))
        })
        .collect();
    let hardware = crate::hardware::hardware_vm(
        state.hardware.as_ref(),
        manifests.iter().map(|(id, m)| (id.to_string(), m.name.clone(), &m.requires)),
    );
    let themes = plugins
        .of_kind(PluginKind::Theme)
        .into_iter()
        .map(|id| ThemeChoiceVm {
            name: plugins.manifest(&id).map(|m| m.name.clone()).unwrap_or_default(),
            current: state.theme.as_ref() == Some(&id),
            id,
        })
        .collect();
    let theme_options = match &state.theme {
        Some(theme) => {
            let chain = theme_chain(plugins, theme);
            let refs: Vec<&ThemeManifest> = chain.iter().collect();
            let schema = options_schema(&merge_options(&refs));
            crate::settings_schema::fields(&schema, &s.theme_options)
        }
        None => Vec::new(),
    };
    SettingsVm {
        launcher_update: launcher_update_vm(&s.launcher_update),
        hardware,
        panels: state.settings.iter().map(settings_panel_vm).collect(),
        themes,
        theme_options,
        // The game folders. The other folders are on the pages of the plugins that ask for them.
        folders: folders_vm(s).into_iter().filter(|f| nexus_api::shared_folder(&f.name).is_some()).collect(),
        sounds: prefs.sounds,
        fullscreen: prefs.fullscreen,
        return_on_exit: prefs.return_on_exit,
        auto_update: prefs.auto_update,
        scale: prefs.scale,
        can_start_at_login: s.can_start_at_login,
        start_at_login: prefs.start_at_login,
        start_at_login_error: prefs.start_at_login_error.clone().unwrap_or_default(),
        vpn: vpn_vm(&s.vpn),
    }
}

fn vpn_vm(vpn: &crate::vpn::VpnSnapshot) -> nexus_ui_api::VpnSettingsVm {
    nexus_ui_api::VpnSettingsVm {
        problem: vpn.problem.clone().unwrap_or_default(),
        only: vpn.only,
        finds: vpn.finds,
        can_import: vpn.can_import,
        route: vpn.route.clone().unwrap_or_default(),
        chosen: vpn.chosen.clone(),
        vpns: vpn
            .vpns
            .iter()
            .map(|v| nexus_ui_api::VpnVm {
                name: v.name.clone(),
                kind: v.kind.clone(),
                connected: v.connected,
                switchable: v.switchable,
            })
            .collect(),
        error: vpn.error.clone().unwrap_or_default(),
    }
}

fn folders_vm(s: &Snapshot) -> Vec<FolderVm> {
    let (plugins, host) = (&s.plugins, s.plugins.host());
    let mut by_name: BTreeMap<String, FolderVm> = BTreeMap::new();
    for plugin in plugins.order() {
        let Some(manifest) = plugins.manifest(plugin) else { continue };
        for grant in &manifest.permissions.filesystem {
            let home = host.home.as_deref();
            let folder = by_name.entry(grant.folder.clone()).or_insert_with(|| {
                let roots = s.files.roots.get(&grant.folder).map(Vec::as_slice).unwrap_or_default();
                let paths: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
                FolderVm {
                    name: grant.folder.clone(),
                    title: String::new(),
                    help: String::new(),
                    text: paths.join("; "),
                    shown: String::new(),
                    paths,
                    default: String::new(),
                    requested_by: Vec::new(),
                    write: false,
                }
            });
            if folder.default.is_empty()
                && let Some(default) = grant.default_path(host.platform).and_then(|d| crate::folders::expand(d, home))
            {
                folder.default = default.display().to_string();
            }
            if let Some(shared) = nexus_api::shared_folder(&grant.folder) {
                folder.title = shared.title.into();
                folder.help = shared.help.into();
            }
            if folder.title.is_empty() {
                folder.title = grant.title.clone().unwrap_or_default();
            }
            if folder.help.is_empty() {
                folder.help = grant.help.clone().unwrap_or_default();
            }
            folder.requested_by.push(manifest.name.clone());
            folder.write |= grant.write;
        }
    }
    let home = host.home.as_deref().map(|h| h.display().to_string());
    by_name
        .into_values()
        .map(|mut f| {
            let paths: Vec<String> = if f.paths.is_empty() { vec![f.default.clone()] } else { f.paths.clone() };
            f.shown = paths
                .iter()
                .filter(|p| !p.is_empty())
                .map(|p| short_home(p, home.as_deref()))
                .collect::<Vec<_>>()
                .join("; ");
            f
        })
        .collect()
}

/// `path` with the home folder at its start as `~`.
fn short_home(path: &str, home: Option<&str>) -> String {
    match home.and_then(|h| path.strip_prefix(h)) {
        Some(rest) if rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR) => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

fn emulators_vm(s: &Snapshot) -> EmulatorsVm {
    let (plugins, state) = (&s.plugins, &s.state);
    let manages_an_emulator = |id: &PluginId| {
        plugins.manifest(id).is_some_and(|m| {
            m.capabilities.contains(&Capability::Installer) || m.capabilities.contains(&Capability::Launcher)
        })
    };
    let emulators = plugins
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter(manages_an_emulator)
        .map(|id| {
            let emu = state.emulators.get(&id).cloned().unwrap_or_default();
            EmulatorVm {
                name: plugins.manifest(&id).map(|m| m.emulator_name().to_owned()).unwrap_or_default(),
                status: status_text(plugins.status(&id)),
                has_builds: plugins.manifest(&id).is_some_and(|m| m.capabilities.contains(&Capability::Installer)),
                update: emu.update(),
                installed: emu.installed,
                available: emu.available,
                busy: emu.busy,
                panels: slot_vms(&id, SlotId::EmulatorPanel, emu.panels),
                controllers: controllers(plugins.manifest(&id)),
                plugin: id,
            }
        })
        .collect();
    let rows = plugins
        .order()
        .iter()
        .filter_map(|id| plugins.manifest(id).map(|m| (id, m)))
        .map(|(id, m)| PluginRowVm {
            id: id.clone(),
            name: m.name.clone(),
            kind: match m.kind {
                PluginKind::Emulator => "emulator".into(),
                PluginKind::Service => "service".into(),
                PluginKind::System => "system".into(),
                PluginKind::Theme => "theme".into(),
            },
            version: m.version.to_string(),
            status: status_text(plugins.status(id)),
            enabled: plugins.enabled(id),
            build: state.emulators.get(id).and_then(|e| e.installed.last().cloned()).unwrap_or_default(),
            build_label: build_label(
                m.emulator_name(),
                state.emulators.get(id).and_then(|e| e.installed.last()).map_or("", String::as_str),
            ),
            has_builds: m.capabilities.contains(&Capability::Installer),
            note: if plugins.elsewhere(id) {
                let names: Vec<&str> = m
                    .platforms
                    .iter()
                    .map(|p| match p {
                        Platform::Linux => "Linux",
                        Platform::Macos => "macOS",
                        Platform::Windows => "Windows",
                    })
                    .collect();
                format!("Made for {}, not this system: turn it on to use your own build", names.join(" and "))
            } else {
                String::new()
            },
        })
        .collect();
    EmulatorsVm { emulators, plugins: rows }
}

/// `build` with the program's name `emulator` once: "shadPS4 v.0.20.0", and
/// "KytyPS5-2026-10-21-e2dd48a" as it is, as it starts with the name. A build in words ("Your
/// own build") comes after a dot. Empty for no build.
fn build_label(emulator: &str, build: &str) -> String {
    let named = build.get(..emulator.len()).is_some_and(|start| start.eq_ignore_ascii_case(emulator));
    let version = build.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit() || c == '.');
    if build.is_empty() || named {
        build.to_owned()
    } else if version {
        format!("{emulator} {build}")
    } else {
        format!("{emulator} · {build}")
    }
}

fn plugin_page_vm(s: &Snapshot, plugin: &PluginId) -> Option<PluginPageVm> {
    let state = &s.state;
    let all = emulators_vm(s);
    let row = all.plugins.into_iter().find(|p| &p.id == plugin)?;
    if !row.enabled {
        return Some(PluginPageVm {
            plugin: row,
            emulator: None,
            panels: Vec::new(),
            sections: Vec::new(),
            folders: Vec::new(),
        });
    }
    let asks: Vec<&str> = s
        .plugins
        .manifest(plugin)
        .map(|m| m.permissions.filesystem.iter().map(|g| g.folder.as_str()).collect())
        .unwrap_or_default();
    Some(PluginPageVm {
        emulator: all.emulators.into_iter().find(|e| &e.plugin == plugin),
        panels: state.settings.iter().filter(|p| &p.plugin == plugin).map(settings_panel_vm).collect(),
        sections: state
            .emulators
            .get(plugin)
            .map(|e| slot_vms(plugin, SlotId::EmulatorPanel, e.panels.clone()))
            .unwrap_or_default(),
        folders: folders_vm(s)
            .into_iter()
            .filter(|f| nexus_api::shared_folder(&f.name).is_none() && asks.contains(&f.name.as_str()))
            .collect(),
        plugin: row,
    })
}

/// The most covers the boot screen's wall shows.
const BOOT_COVERS: usize = 30;

fn boot_vm(s: &Snapshot) -> BootVm {
    let library = &s.library;
    let ready = library.booted;
    let games = library.library().count();
    let step = |label: String, detail: String| BootStepVm {
        label,
        detail,
        state: if ready { "done".into() } else { "working".into() },
    };
    let mut steps = vec![step("Game folders".into(), if ready { Look::games(games) } else { "Looking…".into() })];
    for (plugin, emu) in &s.state.emulators {
        let Some(manifest) = s.plugins.manifest(plugin) else { continue };
        if !manifest.capabilities.contains(&Capability::Installer) {
            continue;
        }
        let detail = if emu.own {
            "Your own build"
        } else if emu.installed.is_empty() {
            "Installs on first play"
        } else if emu.update().is_some() {
            "Update available"
        } else {
            "Latest build"
        };
        steps.push(step(format!("{} emulator", manifest.emulator_name()), detail.into()));
    }
    if let Some((done, total)) = library.describing {
        steps.push(BootStepVm {
            label: "Official artwork & details".into(),
            detail: if library.described {
                "Done".into()
            } else if total > 0 {
                format!("{done}/{total}")
            } else {
                "Looking…".into()
            },
            state: if library.described { "done".into() } else { "working".into() },
        });
    }
    steps.push(step("Covers & Home screen".into(), if ready { "Ready".into() } else { "Loading…".into() }));
    BootVm {
        ready,
        welcome: !s.prefs.welcomed,
        progress: if ready {
            1.0
        } else if library.loading {
            0.6
        } else {
            0.2
        },
        status: if ready {
            format!("All set · {games} installed game{} found", if games == 1 { "" } else { "s" })
        } else {
            "Loading...".into()
        },
        steps,
        covers: library.library().filter_map(|e| cover_of(library, e)).take(BOOT_COVERS).collect(),
    }
}

/// Words for counts.
struct Look;

impl Look {
    /// "1 game", "5 games".
    fn games(n: usize) -> String {
        if n == 1 { "1 game".into() } else { format!("{n} games") }
    }
}

/// The game on the Running screen, or else the newest one still running.
fn running_vm(snapshot: &Snapshot, route: &Route) -> Option<RunningVm> {
    let sessions = &snapshot.sessions;
    let session = match route {
        Route::Running(session) => *session,
        _ => *sessions.sessions.keys().next_back()?,
    };
    let s = sessions.sessions.get(&session)?;
    let manifest = snapshot.plugins.manifest(&s.game.plugin);
    Some(RunningVm {
        session: session.0,
        key: s.game.to_string(),
        title: s.title.clone(),
        emulator: manifest.as_ref().map(|m| m.emulator_name().to_owned()).unwrap_or_default(),
        page: manifest.as_ref().map(|m| m.name.clone()).unwrap_or_default(),
        state: if s.stopping {
            "stopping".into()
        } else if s.shown {
            "running".into()
        } else {
            "starting".into()
        },
        elapsed_secs: snapshot.now.saturating_duration_since(s.started).as_secs(),
        in_front: sessions.game_in_front,
        keymap: manifest.map(|m| m.keymap.clone()).unwrap_or_default(),
    })
}

/// A theme's manifest and its parents' (the theme first). Stops at a missing or bad manifest.
fn theme_chain(plugins: &Plugins, theme: &PluginId) -> Vec<ThemeManifest> {
    let mut chain: Vec<ThemeManifest> = Vec::new();
    let mut next = Some(theme.clone());
    while let Some(id) = next.take() {
        if chain.iter().any(|m| m.id() == &id) {
            break;
        }
        let Some(manifest) = plugins.manifest(&id).and_then(|m| ThemeManifest::from_plugin(m).ok()) else { break };
        next = manifest.theme.extends.clone();
        chain.push(manifest);
    }
    chain
}

/// What an emulator's games play with: a controller, and the keyboard when the manifest lists
/// its keys (`[[keymap]]`). None for an emulator that lists no keys.
fn controllers(manifest: Option<&nexus_api::PluginManifest>) -> Vec<String> {
    match manifest {
        Some(m) if !m.keymap.is_empty() => {
            vec!["Controller (DualSense, DualShock 4, Xbox and most others)".into(), "Keyboard".into()]
        }
        _ => Vec::new(),
    }
}

fn status_text(status: Option<&PluginStatus>) -> String {
    match status {
        Some(PluginStatus::Active) => "active".into(),
        Some(PluginStatus::Failed(_)) => "failed".into(),
        _ => "loaded".into(),
    }
}

/// `text` on one line, cut after a word to at most `SUMMARY_CHARS` characters, with "…".
fn summary_of(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= nexus_ui_api::SUMMARY_CHARS {
        return line;
    }
    let cut: String = line.chars().take(nexus_ui_api::SUMMARY_CHARS).collect();
    format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_names_its_program_once() {
        assert_eq!(build_label("OrbitEmu", "OrbitEmu-2026-10-21-e2dd48a"), "OrbitEmu-2026-10-21-e2dd48a");
        assert_eq!(
            build_label("OrbitEmu", "orbitemu-2026-10-21-e2dd48a"),
            "orbitemu-2026-10-21-e2dd48a",
            "in any case"
        );
        assert_eq!(build_label("shadPS4", "v.0.20.0"), "shadPS4 v.0.20.0");
        assert_eq!(build_label("Emu", "1.0"), "Emu 1.0");
        assert_eq!(
            build_label("shadPS4", "Your own build · 0.3"),
            "shadPS4 · Your own build · 0.3",
            "words, not a version"
        );
        assert_eq!(build_label("OrbitEmu", ""), "", "none installed");
    }
}
