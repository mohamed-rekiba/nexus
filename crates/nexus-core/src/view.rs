//! Building view models from app state.

use std::collections::BTreeMap;

use nexus_api::{
    ActionStyle, Capability, CompatInfo, CompatLevel, GameKey, ImageSource, Platform, PluginId, PluginKind,
    SlotContent, SlotId, SlotItem, Tone,
};
use nexus_ui_api::*;
use serde_json::Value;

use crate::app::{AppState, EmulatorState, Entry, Grid, Route};
use crate::order::sort_by_order;
use crate::torrents::{TorrentJob, TorrentsSnapshot};
use crate::{Core, Job, JobPhase, Kernel, PluginStatus, PreferencesSnapshot};

pub(crate) fn image(plugin: &PluginId, source: &ImageSource) -> ImageRef {
    match source {
        ImageSource::Url(url) => ImageRef::Url { plugin: plugin.clone(), url: url.clone() },
        ImageSource::Asset(path) => ImageRef::PluginAsset { plugin: plugin.clone(), path: path.clone() },
        ImageSource::Folder(path) => ImageRef::Folder { plugin: plugin.clone(), path: path.clone() },
    }
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
            vm.icon = icon.map(|i| image(plugin, &i));
        }
        SlotContent::Action { label, action, args, style, icon } => {
            vm.kind = SlotKind::Action;
            vm.text = label;
            vm.action = Some(action);
            vm.args = args;
            vm.style = style;
            vm.icon = icon.map(|i| image(plugin, &i));
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

pub(crate) fn compat_vm(info: &CompatInfo) -> CompatVm {
    let (level, label) = match info.level {
        CompatLevel::Unknown => ("unknown", "Not tested"),
        CompatLevel::Nothing => ("nothing", "Doesn't start"),
        CompatLevel::Boots => ("boots", "Boots"),
        CompatLevel::Menus => ("menus", "Menus"),
        CompatLevel::Ingame => ("ingame", "In-game"),
        CompatLevel::Playable => ("playable", "Playable"),
    };
    CompatVm {
        level: level.into(),
        label: label.into(),
        notes: info.notes.clone().unwrap_or_default(),
        tested: info.tested.clone().unwrap_or_default(),
    }
}

/// The newest available version, if it is newer than every installed one. Versions compare as
/// semver when they parse, as text otherwise.
pub(crate) fn update_for(emu: &EmulatorState) -> Option<String> {
    let newest = emu.available.first()?;
    let installed = emu.installed.last()?;
    let newer = match (semver::Version::parse(newest), semver::Version::parse(installed)) {
        (Ok(a), Ok(b)) => a > b,
        _ => newest != installed && !emu.installed.contains(newest),
    };
    newer.then(|| newest.clone())
}

/// Settings fields from a JSON Schema object: booleans, numbers, strings and string enums.
/// Other properties are left out. They come in the order the schema's `x-order` list names,
/// then the rest by name: JSON objects don't keep their order.
pub(crate) fn fields(schema: &Value, values: &Value) -> Vec<FieldVm> {
    let order: Vec<&str> = schema
        .get("x-order")
        .and_then(Value::as_array)
        .map(|o| o.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = unordered_fields(schema, values);
    out.sort_by_key(|f| order.iter().position(|k| *k == f.key).unwrap_or(order.len()));
    out
}

/// A choice's values with their labels ("" when it has none): an `enum` list, or `oneOf`/`anyOf`
/// entries. schemars makes a `const` entry, labelled by its doc comment, for each variant with
/// one, and groups the rest into one `enum` entry.
fn choices(schema: &Value) -> Vec<(String, String)> {
    let listed = |o: &Value| -> Vec<(String, String)> {
        o.get("enum")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(|v| (v.to_owned(), String::new())).collect())
            .unwrap_or_default()
    };
    let Some(options) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) else {
        return listed(schema);
    };
    options
        .iter()
        .flat_map(|o| match o.get("const").and_then(Value::as_str) {
            Some(value) => {
                let label = o.get("description").or_else(|| o.get("title")).and_then(Value::as_str);
                vec![(value.to_owned(), label.unwrap_or_default().to_owned())]
            }
            None => listed(o),
        })
        .collect()
}

/// A property's schema with an `Option` taken off: schemars writes `Option<T>` as a type list
/// with "null", or as `anyOf` with a `null` entry.
pub(crate) fn nullable(prop: &Value) -> bool {
    let in_types = prop.get("type").and_then(Value::as_array).is_some_and(|t| t.iter().any(|v| v == "null"));
    let in_any = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|o| o.iter().any(|v| v.get("type") == Some(&Value::from("null"))));
    in_types || in_any
}

/// The property's own type: "string", "integer"…, without "null".
pub(crate) fn type_of(prop: &Value) -> Option<&str> {
    match prop.get("type")? {
        Value::String(t) => Some(t),
        Value::Array(types) => types.iter().filter_map(Value::as_str).find(|t| *t != "null"),
        _ => None,
    }
}

/// What a property is, past an `Option`'s `anyOf` and a `$ref` into the schema's `$defs`.
fn resolved<'a>(schema: &'a Value, prop: &'a Value) -> &'a Value {
    let inner = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .filter(|o| o.len() == 2 && nullable(prop))
        .and_then(|o| o.iter().find(|v| v.get("type") != Some(&Value::from("null"))))
        .unwrap_or(prop);
    inner
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/").or_else(|| r.strip_prefix("#/definitions/")))
        .and_then(|name| schema.get("$defs").or_else(|| schema.get("definitions"))?.get(name))
        .unwrap_or(inner)
}

fn unordered_fields(schema: &Value, values: &Value) -> Vec<FieldVm> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else { return Vec::new() };
    let text = |v: Option<&Value>| match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    properties
        .iter()
        .filter_map(|(key, prop)| {
            // A `$ref` points into the schema's `$defs`: Rust enums come out that way.
            let target = resolved(schema, prop);
            let mut choices = choices(target);
            // An optional choice can also be left to the emulator's own setting.
            if nullable(prop) && !choices.is_empty() {
                choices.insert(0, (String::new(), "As the emulator".into()));
            }
            let kind = match type_of(target).or_else(|| type_of(prop)) {
                _ if !choices.is_empty() => FieldKind::Choice,
                Some("boolean") => FieldKind::Toggle,
                Some("integer" | "number") => FieldKind::Number,
                Some("string") => FieldKind::Text,
                _ => return None,
            };
            Some(FieldVm {
                key: key.clone(),
                label: prop.get("title").and_then(Value::as_str).unwrap_or(key).to_owned(),
                help: prop.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
                kind,
                value: text(values.get(key).or_else(|| prop.get("default"))),
                choice_labels: choices.iter().map(|(_, label)| label.clone()).collect(),
                choices: choices.into_iter().map(|(value, _)| value).collect(),
            })
        })
        .collect()
}

/// The game's square art: the store's tile when a catalog describes it, else its own icon,
/// else the cover of the offer it is.
fn cover_of(state: &AppState, entry: &Entry) -> Option<ImageRef> {
    let store = state.meta.get(&entry.key).and_then(|(plugin, m)| m.icon.as_ref().map(|i| image(plugin, i)));
    store
        .or_else(|| entry.summary.cover.as_ref().map(|c| image(&entry.key.plugin, c)))
        .or_else(|| state.offer_art.get(&entry.key).map(|(plugin, c)| image(plugin, c)))
}

/// An installed game, the offer it is, and the offer's cover with the plugin that has it.
pub(crate) type InstalledOffer = (GameKey, GameKey, Option<(PluginId, ImageSource)>);

/// The offer each installed game is (see [`installed_game`]), by the game, with its cover.
pub(crate) fn installed_offers(state: &AppState, torrents: &TorrentsSnapshot) -> Vec<InstalledOffer> {
    let owned = Owned::new(state, torrents);
    state
        .offers
        .iter()
        .filter_map(|offer| {
            let game = installed_game(state, &owned, offer)?;
            let cover = offer.entry.summary.cover.clone().map(|c| (offer.entry.key.plugin.clone(), c));
            Some((game.key.clone(), offer.entry.key.clone(), cover))
        })
        .collect()
}

/// The game Play starts for `key`: for a Catalog offer the player has installed, the installed
/// game it is; else `key` itself.
pub(crate) fn played_game(state: &AppState, torrents: &TorrentsSnapshot, key: &GameKey) -> GameKey {
    let owned = Owned::new(state, torrents);
    let offer = state.offers.iter().find(|o| &o.entry.key == key);
    offer.and_then(|o| installed_game(state, &owned, o)).map(|g| g.key.clone()).unwrap_or_else(|| key.clone())
}

/// A catalog's store data for `key`: an offer's own, or for an installed game, the offer's it
/// is.
fn release_metadata(state: &AppState, key: &GameKey) -> Option<(PluginId, nexus_api::GameMetadata)> {
    let offer_key = state.installed_offers.get(key).unwrap_or(key);
    let offer = state.offers.iter().find(|o| &o.entry.key == offer_key)?;
    crate::releases::metadata_of(offer).map(|m| (offer.entry.key.plugin.clone(), m.clone()))
}

/// Whether `key`'s game can be uninstalled: its folder is known, isn't a whole player's folder,
/// is in one its emulator may write, and is there.
fn can_uninstall(kernel: &Kernel, state: &AppState, key: &GameKey) -> bool {
    let folder = state.details.get(key).and_then(|d| d.details.as_ref()).and_then(|d| d.folder.as_ref());
    folder.is_some_and(|f| {
        !f.path.is_root()
            && kernel.manifest(&key.plugin).is_ok_and(|m| m.permissions.folder(&f.folder).is_some_and(|g| g.write))
            && crate::app::resolve_game_folder(kernel, key, f).is_some()
    })
}

/// The system `platform` (`PS5`), as the plugin whose `[system]` it is describes it: its full
/// name and color. Without one, the code is its name and it has no color.
pub(crate) fn system_of(kernel: &Kernel, platform: &str) -> GameSystemVm {
    let look = kernel
        .order()
        .iter()
        .filter_map(|plugin| kernel.manifest(plugin).ok()?.system.as_ref())
        .find(|look| look.id == platform);
    match look {
        Some(look) => GameSystemVm { name: look.name.clone(), color: look.rgb() },
        None => GameSystemVm { name: platform.to_owned(), color: None },
    }
}

fn card(kernel: &Kernel, core_state: &AppState, entry: &Entry) -> GameCardVm {
    let stats = crate::launch::play_stats(kernel, &entry.key);
    GameCardVm {
        key: entry.key.to_string(),
        title: entry.summary.title.clone(),
        platform: entry.summary.platform.clone(),
        system: system_of(kernel, &entry.summary.platform),
        cover: cover_of(core_state, entry),
        selected: core_state.selected.as_ref() == Some(&entry.key),
        badges: core_state.badges.get(&entry.key).cloned().unwrap_or_default(),
        release_year: entry.summary.release_year,
        rating: core_state.meta.get(&entry.key).and_then(|(_, m)| m.rating).map(|r| r.score),
        size_bytes: core_state.details.get(&entry.key).and_then(|d| d.details.as_ref()).and_then(|d| d.size_bytes),
        last_played_days: (stats.count > 0).then(|| crate::launch::days_since(stats.last)),
        releases: 0,
        availability: String::new(),
        hidden: false,
        // A Library game's; `offer_card` gives an offer's.
        can: game_actions(kernel, core_state, &entry.key, None),
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
pub(crate) fn game_actions(
    kernel: &Kernel,
    state: &AppState,
    key: &GameKey,
    offer: Option<(&crate::releases::Offer, Option<&Entry>)>,
) -> GameActionsVm {
    let installed: Option<&GameKey> = match offer {
        None => Some(key),
        Some((_, game)) => game.map(|g| &g.key),
    };
    let manifest = |plugin: &PluginId| kernel.manifest(plugin).ok();
    let play =
        installed.is_some_and(|k| manifest(&k.plugin).is_some_and(|m| m.capabilities.contains(&Capability::Launcher)));
    let mut actions = GameActionsVm { play, ..GameActionsVm::default() };
    if let Some(game) = installed {
        let details = state.details.get(game).and_then(|d| d.details.as_ref());
        let link = details.and_then(|d| d.uninstall_link.as_deref());
        let store_removes = link.is_some_and(|l| manifest(&game.plugin).is_some_and(|m| m.permissions.allows_link(l)));
        if store_removes {
            let platform = details.map(|d| d.summary.platform.as_str()).unwrap_or_default();
            actions.uninstall = game.to_string();
            actions.uninstall_store = system_of(kernel, platform).name;
        } else if can_uninstall(kernel, state, game) {
            actions.uninstall = game.to_string();
        }
    }
    if let Some((offer, None)) = offer {
        match offer.releases.iter().find(|r| !r.store.is_empty()) {
            Some(release) => {
                actions.get_store = system_of(kernel, &release.platform).name;
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

/// The genres a store's catalog gives the game.
fn genres_of(state: &AppState, key: &GameKey) -> Vec<String> {
    state.meta.get(key).map(|(_, m)| m.genres.clone()).unwrap_or_default()
}

/// The fixed genres of a game a catalog offers (see [`crate::releases::GENRES`]), from its
/// releases' genre words, a store's genres and its releases' store data.
fn offer_genres(state: &AppState, offer: &crate::releases::Offer) -> Vec<String> {
    let store = state.meta.get(&offer.entry.key).map(|(_, m)| &m.genres);
    let release = crate::releases::metadata_of(offer).map(|m| &m.genres);
    let words: Vec<&str> = offer
        .releases
        .iter()
        .map(|r| r.fact_text(nexus_api::FactKind::Genre))
        .chain(store.into_iter().chain(release).flatten().map(String::as_str))
        .collect();
    crate::releases::genres_in(&words).into_iter().map(str::to_owned).collect()
}

/// The words of a search or of a field, in lower case: letters and digits. An apostrophe joins
/// its word ("Assassin's" is "assassins").
fn search_words(text: &str) -> Vec<String> {
    let text = text.to_lowercase().replace(['\'', '\u{2019}'], "");
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// Whether each of the search's `words` starts a word of `fields`.
fn has_words<'a>(words: &[String], fields: impl IntoIterator<Item = &'a str>) -> bool {
    let known: Vec<String> = fields.into_iter().flat_map(search_words).collect();
    words.iter().all(|w| known.iter().any(|k| k.starts_with(w.as_str())))
}

/// A Library's or the Catalog's games, searched, filtered and sorted, before paging, with
/// the filters' chips.
struct GridGames<'a> {
    shown: Vec<&'a Entry>,
    /// How many games there are before the search and the filters.
    all: usize,
    systems: Vec<FilterVm>,
    genres: Vec<FilterVm>,
}

/// The offers by their key.
fn offers_by_key(state: &AppState) -> std::collections::HashMap<&GameKey, &crate::releases::Offer> {
    state.offers.iter().map(|o| (&o.entry.key, o)).collect()
}

fn grid<'a>(kernel: &Kernel, state: &'a AppState, torrents: &TorrentsSnapshot, catalog: bool) -> GridGames<'a> {
    let view = if catalog { &state.catalog_view } else { &state.library_view };
    let offers = offers_by_key(state);
    let owned = Owned::new(state, torrents);
    let entries: Vec<&Entry> =
        if catalog { state.offers.iter().map(|o| &o.entry).collect() } else { state.library.iter().collect() };
    // The Catalog's hidden games: out, but for the "Hidden" filter, which shows only them. With
    // the filter turned off in the catalog's settings, its chip goes and it doesn't apply.
    let hidden = if catalog { crate::Core::hidden_games(kernel) } else { Default::default() };
    let hidden_filter = catalog && crate::Core::hidden_filter_on(kernel);
    let only_hidden = hidden_filter && view.hidden;
    // Hidden by the player in Nexus, or in the game's store (Steam's Hidden collection).
    let store_hidden =
        |e: &Entry| offers.get(&e.key).is_some_and(|o| !o.releases.is_empty() && o.releases.iter().all(|r| r.hidden));
    let is_hidden = |e: &Entry| catalog && (hidden.contains(&e.key.to_string()) || store_hidden(e));
    let hidden_count = entries.iter().filter(|e| is_hidden(e)).count();
    let entries: Vec<&Entry> = entries.into_iter().filter(|e| is_hidden(e) == only_hidden).collect();
    // How many games there are before the search and the filters: hidden ones count nowhere.
    let all = entries.len();
    let needle = view.query.trim().to_lowercase();
    let words = search_words(&view.query);
    let found = |e: &Entry| {
        if e.summary.title.to_lowercase().contains(&needle) {
            return true;
        }
        let publisher = state.meta.get(&e.key).map(|(_, m)| m.publisher.as_str()).unwrap_or_default();
        let title_id = e.summary.title_id.as_deref().unwrap_or_default();
        match offers.get(&e.key) {
            // Every word in one release: its title, title ids, publisher, region and version.
            Some(o) => o.releases.iter().any(|r| {
                let ids = std::iter::once(r.title_id.as_str()).chain(r.title_ids.iter().map(String::as_str));
                let own = r.fact_text(nexus_api::FactKind::Publisher);
                let fields = [e.summary.title.as_str(), &r.title, own, publisher, &r.region, &r.version];
                has_words(&words, fields.into_iter().chain(ids))
            }),
            None => has_words(&words, [e.summary.title.as_str(), title_id, publisher]),
        }
    };
    let searched: Vec<&Entry> = entries.into_iter().filter(|e| needle.is_empty() || found(e)).collect();
    // Systems count what the search leaves; genres count what the system leaves too.
    let mut systems =
        vec![FilterVm { id: String::new(), label: "All".into(), count: searched.len(), on: view.system.is_none() }];
    for e in &searched {
        let platform = &e.summary.platform;
        match systems.iter_mut().find(|s| &s.id == platform) {
            Some(s) => s.count += 1,
            None => systems.push(FilterVm {
                id: platform.clone(),
                label: platform.clone(),
                count: 1,
                on: view.system.as_ref() == Some(platform),
            }),
        }
    }
    // How far each game gets: the player's own result first, then the community's. The
    // player's results are read once, not once a game.
    let results = kernel.config().get(&["app", "results"]);
    let level = |e: &Entry| -> u8 {
        let own = results.get(e.key.to_string()).and_then(Value::as_str);
        let compat = state.details.get(&e.key).and_then(|d| d.compat.as_ref()).map(|c| c.level.as_str());
        match own.or(compat) {
            Some("playable") => 5,
            Some("ingame") => 4,
            Some("menus") => 3,
            Some("boots") => 2,
            Some("nothing") => 1,
            _ => 0,
        }
    };
    let ingame = searched.iter().filter(|e| level(e) >= 4).count();
    // The Catalog's only: the Library's games are the player's own. Shown once a game has a
    // result to filter by.
    let only_ingame = catalog && view.ingame;
    if catalog && (ingame > 0 || view.ingame) {
        systems.push(FilterVm {
            id: crate::INGAME_FILTER.into(),
            label: "In-game".into(),
            count: ingame,
            on: view.ingame,
        });
    }
    // The Catalog's offers of the games the player has.
    let installed = |e: &Entry| offers.get(&e.key).is_some_and(|o| installed_game(state, &owned, o).is_some());
    let installed_count = if catalog { searched.iter().filter(|e| installed(e)).count() } else { 0 };
    if installed_count > 0 || (catalog && view.installed) {
        systems.push(FilterVm {
            id: crate::INSTALLED_FILTER.into(),
            label: "Installed".into(),
            count: installed_count,
            on: view.installed,
        });
    }
    if hidden_filter && (hidden_count > 0 || view.hidden) {
        systems.push(FilterVm {
            id: crate::HIDDEN_FILTER.into(),
            label: "Hidden".into(),
            count: hidden_count,
            on: view.hidden,
        });
    }
    let in_system: Vec<&Entry> = searched
        .into_iter()
        .filter(|e| view.system.as_ref().is_none_or(|s| &e.summary.platform == s))
        .filter(|e| !only_ingame || level(e) >= 4)
        .filter(|e| !catalog || !view.installed || installed(e))
        .collect();
    // The Library's chips are the store's genres; the Catalog's are the fixed genres.
    let chips: std::collections::HashMap<&GameKey, Vec<String>> = in_system
        .iter()
        .map(|e| {
            let genres = match offers.get(&e.key) {
                Some(o) if catalog => offer_genres(state, o),
                _ => genres_of(state, &e.key),
            };
            (&e.key, genres)
        })
        .collect();
    let chips_of = |e: &Entry| chips.get(&e.key).map(Vec::as_slice).unwrap_or_default();
    let mut genre_counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &in_system {
        for g in chips_of(e).iter().cloned() {
            *genre_counts.entry(g).or_default() += 1;
        }
    }
    let mut genres: Vec<FilterVm> = genre_counts
        .into_iter()
        .map(|(g, count)| FilterVm { on: view.genre.as_ref() == Some(&g), label: g.clone(), id: g, count })
        .collect();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
    genres.insert(
        0,
        FilterVm { id: String::new(), label: "All genres".into(), count: in_system.len(), on: view.genre.is_none() },
    );
    let mut shown: Vec<&Entry> =
        in_system.into_iter().filter(|e| view.genre.as_ref().is_none_or(|g| chips_of(e).contains(g))).collect();
    let size = |e: &Entry| -> Option<u64> {
        match offers.get(&e.key) {
            Some(o) => best_release(state, &owned, o).size_bytes,
            None => state.details.get(&e.key).and_then(|d| d.details.as_ref()).and_then(|d| d.size_bytes),
        }
    };
    let added = |e: &Entry| offers.get(&e.key).map(|o| o.releases[0].added.clone()).unwrap_or_default();
    // The catalog's own date of its best release first, then the store's, then the year.
    let released = |e: &Entry| {
        let catalog = offers.get(&e.key).and_then(|o| crate::releases::release_day(best_release(state, &owned, o)));
        let store = || state.meta.get(&e.key).and_then(|(_, m)| crate::releases::date_key(&m.release));
        catalog.or_else(store).or_else(|| e.summary.release_year.map(|y| y.to_string())).unwrap_or_default()
    };
    match view.sort.as_str() {
        "played" => shown.sort_by_key(|e| std::cmp::Reverse(crate::launch::play_stats(kernel, &e.key).last)),
        "size" => shown.sort_by_key(|e| std::cmp::Reverse(size(e))),
        // Smallest first; a game whose size isn't known goes last.
        "smallest" => shown.sort_by_key(|e| size(e).unwrap_or(u64::MAX)),
        // Top rated, newest, and best running first; a game without one goes last.
        "rating" => shown.sort_by(|a, b| {
            let score = |e: &Entry| state.meta.get(&e.key).and_then(|(_, m)| m.rating).map_or(-1.0, |r| r.score);
            score(b).total_cmp(&score(a))
        }),
        "release" => shown.sort_by_cached_key(|e| std::cmp::Reverse(released(e))),
        "compat" => shown.sort_by_key(|e| std::cmp::Reverse(level(e))),
        "added" => shown.sort_by_cached_key(|e| std::cmp::Reverse(added(e))),
        "title" => shown.sort_by_cached_key(|e| e.summary.title.to_lowercase()),
        _ => {}
    }
    GridGames { shown, all, systems, genres }
}

/// The games beside `game`, at most `most`: in Home's row, one each side; in the Library's or
/// the Catalog's grid, one each side and the ones above and below, when the grid said how many
/// columns it has (`columns`).
pub(crate) fn neighbours(
    kernel: &Kernel,
    state: &AppState,
    torrents: &TorrentsSnapshot,
    game: &GameKey,
    columns: &std::collections::HashMap<Grid, usize>,
    most: usize,
) -> Vec<GameKey> {
    let mut found: Vec<GameKey> = Vec::new();
    let mut around = |list: Vec<&GameKey>, steps: &[isize]| {
        let Some(at) = list.iter().position(|k| *k == game) else { return };
        for step in steps {
            if let Some(key) = at.checked_add_signed(*step).and_then(|i| list.get(i))
                && *key != game
                && !found.contains(key)
            {
                found.push((*key).clone());
            }
        }
    };
    // An offer is in the Catalog's grid; a game of the Library is in Home's row too.
    let (which, view) = if state.offers.iter().any(|o| &o.entry.key == game) {
        (Grid::Catalog, &state.catalog_view)
    } else {
        around(state.library.iter().map(|e| &e.key).collect(), &[1, -1]);
        (Grid::Library, &state.library_view)
    };
    let games = grid(kernel, state, torrents, which == Grid::Catalog);
    let keys: Vec<&GameKey> = games.shown.iter().take(view.shown).map(|e| &e.key).collect();
    match columns.get(&which).copied().filter(|c| *c > 1) {
        Some(c) => around(keys, &[1, -1, c as isize, -(c as isize)]),
        None => around(keys, &[1, -1]),
    }
    found.truncate(most);
    found
}

/// The games on the Catalog's shown pages, in order.
pub(crate) fn catalog_page(kernel: &Kernel, state: &AppState, torrents: &TorrentsSnapshot) -> Vec<GameKey> {
    grid(kernel, state, torrents, true)
        .shown
        .into_iter()
        .take(state.catalog_view.shown)
        .map(|e| e.key.clone())
        .collect()
}

fn grid_vm(kernel: &Kernel, state: &AppState, torrents: &TorrentsSnapshot, catalog: bool) -> LibraryVm {
    let view = if catalog { &state.catalog_view } else { &state.library_view };
    let GridGames { shown, all, systems, genres } = grid(kernel, state, torrents, catalog);
    let offers = offers_by_key(state);
    let owned = Owned::new(state, torrents);
    let today = crate::releases::today();
    let total = shown.len();
    let hidden = if catalog { crate::Core::hidden_games(kernel) } else { Default::default() };
    let games: Vec<GameCardVm> = shown
        .into_iter()
        .take(view.shown)
        .map(|e| match offers.get(&e.key) {
            Some(offer) if catalog => GameCardVm {
                hidden: hidden.contains(&e.key.to_string()),
                ..offer_card(kernel, state, &owned, offer, today)
            },
            _ => card(kernel, state, e),
        })
        .collect();
    LibraryVm {
        selected: games.iter().position(|g| g.selected),
        games,
        total,
        all,
        systems,
        genres,
        sort: view.sort.clone(),
        query: view.query.clone(),
    }
}

/// What clicking `notice` opens, for example "Open Downloads"; empty for nothing.
fn notice_opens(kernel: &Kernel, notice: &crate::app::Notice) -> String {
    match &notice.route {
        Some(Route::Downloads) => "Open Downloads".into(),
        Some(Route::Catalog) => "Open the Catalog".into(),
        Some(Route::GameDetail(_)) => "Open the Game Hub".into(),
        Some(Route::Plugin(plugin)) => {
            format!("Open {}", kernel.manifest(plugin).map(|m| m.name.clone()).unwrap_or_default())
        }
        Some(Route::Settings) => "Open Settings".into(),
        _ => String::new(),
    }
}

/// The cover of the game `notice` is about.
fn notice_art(state: &AppState, notice: &crate::app::Notice) -> Option<ImageRef> {
    let game = notice.game.as_ref()?;
    state.library.iter().find(|e| &e.key == game).and_then(|e| cover_of(state, e))
}

fn notice_vm(kernel: &Kernel, state: &AppState, notice: &crate::app::Notice) -> NoticeVm {
    NoticeVm {
        id: notice.id,
        text: notice.message.clone(),
        kind: notice.kind,
        art: notice_art(state, notice),
        opens: notice_opens(kernel, notice),
    }
}

pub(crate) fn build(core: &Core) -> AppView {
    let kernel = core.kernel();
    let jobs = core.inner.downloads.jobs();
    let statuses = kernel.statuses();
    let torrents = core.torrents().snapshot();
    let prefs = core.prefs();
    let mut state = core.state();

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
    // The detail page of the game the player last opened stays available under overlays.
    // With no detail page open, the selected game's details, once loaded.
    let detail_game = game
        .clone()
        .or_else(|| {
            state.nav.iter().rev().find_map(|r| if let Route::GameDetail(g) = r { Some(g.clone()) } else { None })
        })
        .or_else(|| if route == Route::Catalog { state.catalog_selected.clone() } else { None })
        .or_else(|| state.selected.clone());

    let catalog = catalog_vm(kernel, &state, &torrents);
    if state.catalog_grid.is_none() || state.nav.last() == Some(&Route::Catalog) {
        state.catalog_grid = Some(catalog.games.clone());
    }

    AppView {
        screen,
        game: game.map(|g| g.to_string()),
        can_go_back: state.nav.len() > 1,
        home: home_vm(kernel, &state, &torrents),
        detail: detail_game.and_then(|key| detail_vm(core, kernel, &state, &torrents, &key)),
        catalog,
        downloads: downloads_vm(kernel, jobs, &state, &torrents),
        settings: settings_vm(core, &prefs, &state),
        emulators: emulators_vm(kernel, &state, &statuses),
        plugin_page: match &route {
            Route::Plugin(plugin) => plugin_page_vm(kernel, &state, &statuses, plugin),
            _ => None,
        },
        running: running_vm(kernel, &state, &route),
        notices: state.notices.iter().map(|n| n.message.clone()).collect(),
        notice_cards: state.notices.iter().map(|n| notice_vm(kernel, &state, n)).collect(),
        install_prompt: state.install_prompt.as_ref().map(|p| InstallPromptVm {
            plugin: p.game.plugin.clone(),
            emulator: p.emulator.clone(),
            version: p.version.clone(),
            game: p.title.clone(),
            size_bytes: p.size_bytes,
        }),
        boot: boot_vm(kernel, &prefs, &state),
        task: state.task.clone(),
        notice_opens: state.notices.last().map(|n| notice_opens(kernel, n)).unwrap_or_default(),
        notice_art: state.notices.last().and_then(|n| notice_art(&state, n)),
        system: system_vm(kernel, &state),
        result_prompt: state.result_prompt.as_ref().map(|(key, _)| {
            state
                .library
                .iter()
                .find(|e| &e.key == key)
                .map(|e| e.summary.title.clone())
                .unwrap_or_else(|| key.local.clone())
        }),
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

/// The game of the emulator's library that the download `j` installed: the one with the title
/// its install reported. A PC game has no title id, so this is how its download finds it.
fn installed_from<'a>(state: &'a AppState, j: &TorrentJob) -> Option<&'a Entry> {
    let install = state.game_installs.values().rev().find(|g| g.plugin == j.emulator && g.source == j.source)?;
    let done = install.phase == crate::game_installs::GamePhase::Done;
    let title = &install.installed.as_ref().filter(|_| done)?.title;
    state.library.iter().find(|e| e.key.plugin == j.emulator && &e.summary.title == title)
}

/// The emulators' installed games by title id and by name, to find the game an offer is. A
/// view builds it once: the Catalog asks it about every offer.
pub(crate) struct Owned<'a> {
    /// The downloads, whose installs say which game an offer is.
    jobs: &'a BTreeMap<String, TorrentJob>,
    by_id: std::collections::HashMap<(&'a PluginId, &'a str), &'a Entry>,
    /// By the emulator, and the console and the title as letters and digits (`ps5/astrobot`).
    by_name: std::collections::HashMap<(&'a PluginId, String), &'a Entry>,
}

impl<'a> Owned<'a> {
    pub(crate) fn new(state: &'a AppState, torrents: &'a TorrentsSnapshot) -> Self {
        let mut owned = Owned { jobs: &torrents.jobs, by_id: Default::default(), by_name: Default::default() };
        for e in &state.library {
            if let Some(id) = e.summary.title_id.as_deref().filter(|t| !t.is_empty()) {
                owned.by_id.entry((&e.key.plugin, id)).or_insert(e);
            }
            if let Some(name) = Self::name(&e.summary.platform, &e.summary.title) {
                owned.by_name.entry((&e.key.plugin, name)).or_insert(e);
            }
        }
        owned
    }

    /// A game's name to find it by: its console and its title, as letters and digits.
    fn name(console: &str, title: &str) -> Option<String> {
        let title = crate::releases::plain(title);
        (!title.is_empty()).then(|| format!("{}/{title}", console.to_ascii_lowercase()))
    }
}

/// The installed game an offer is, from the emulator's library: the game a download of one of
/// its releases installed, else the game with one of its title ids, else the game of its name
/// on the same console. A PS4 and a PS5 game of one name are two games.
pub(crate) fn installed_game<'a>(
    state: &'a AppState,
    owned: &Owned<'a>,
    offer: &crate::releases::Offer,
) -> Option<&'a Entry> {
    let downloaded = owned.jobs.values().filter(|j| j.offer == offer.entry.key).find_map(|j| installed_from(state, j));
    let emulator = &offer.emulator;
    let by_id = || {
        let mut ids = offer.releases.iter().flat_map(|r| std::iter::once(&r.title_id).chain(&r.title_ids));
        ids.find_map(|t| owned.by_id.get(&(emulator, t.as_str())).copied())
    };
    let by_name = || {
        let console = &offer.entry.summary.platform;
        let mut names = offer.releases.iter().filter_map(|r| Owned::name(console, &r.title));
        names.find_map(|name| owned.by_name.get(&(emulator, name)).copied())
    };
    downloaded.or_else(by_id).or_else(by_name)
}

/// The release an offer's card shows: the version the player has, then a release whose
/// download installed the game, then one with a magnet link, then one with art, then the
/// highest version, then the one most people share. Of equals, the newest upload.
pub(crate) fn best_release<'a>(
    state: &AppState,
    owned: &Owned,
    offer: &'a crate::releases::Offer,
) -> &'a nexus_api::Release {
    use crate::releases::version_key;
    let game = installed_game(state, owned, offer);
    let has = game
        .and_then(|e| state.details.get(&e.key))
        .and_then(|d| d.details.as_ref())
        .and_then(|d| d.facts.iter().find(|f| f.id == "version"))
        .map(|f| version_key(&f.value))
        .filter(|v| !v.is_empty());
    let rank = |r: &nexus_api::Release| {
        let downloaded = owned.jobs.get(&r.id).and_then(|j| installed_from(state, j)).is_some();
        (
            has.as_ref().is_some_and(|v| *v == version_key(&r.version)),
            downloaded,
            !r.torrent.is_empty(),
            r.metadata.is_some() || !r.image_urls.is_empty(),
            version_key(&r.version),
            r.seeders.unwrap_or(0),
        )
    };
    // The releases are newest upload first, and `max_by_key` keeps the last of equals.
    offer.releases.iter().rev().max_by_key(|r| rank(r)).unwrap_or(&offer.releases[0])
}

fn offer_card(
    kernel: &Kernel,
    state: &AppState,
    owned: &Owned,
    offer: &crate::releases::Offer,
    today: i64,
) -> GameCardVm {
    let mut card = card(kernel, state, &offer.entry);
    card.selected = state.catalog_selected.as_ref() == Some(&offer.entry.key);
    let best = best_release(state, owned, offer);
    card.size_bytes = best.size_bytes;
    card.availability = crate::releases::availability(best.seeders).into();
    card.releases = offer.releases.len();
    let installed = installed_game(state, owned, offer);
    card.can = game_actions(kernel, state, &offer.entry.key, Some((offer, installed)));
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
    } else if offer.releases.iter().any(|r| crate::releases::is_new(&r.added, today)) {
        card.badges.push(badge(plugin, "new", "New", Tone::Info));
    }
    // Every release is for another system.
    let here = kernel.host().platform;
    if offer.releases.iter().all(|r| !crate::releases::not_here(&r.system, here).is_empty()) {
        card.badges.push(badge(plugin, "not-here", "Can't run here", Tone::Warning));
    }
    card
}

fn catalog_vm(kernel: &Kernel, state: &AppState, torrents: &TorrentsSnapshot) -> CatalogVm {
    let name =
        |plugin: &PluginId| kernel.manifest(plugin).map(|m| m.name.clone()).unwrap_or_else(|_| plugin.to_string());
    let sources = state
        .catalogs
        .iter()
        .map(|c| {
            let l = &c.list;
            let copy = match l.source {
                nexus_api::ReleaseSource::Fetched => format!("fetched from {}", l.host),
                nexus_api::ReleaseSource::Cached => format!("the copy fetched before from {}", l.host),
                nexus_api::ReleaseSource::Bundled => "the bundled catalog".into(),
            };
            let mut parts = vec![format!("{}: {copy}", name(&c.emulator))];
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
    let releases = state.catalogs.iter().flat_map(|c| &c.list.releases);
    // The releases of the games the Catalog has, not of the ones the player hid.
    let hidden = crate::Core::hidden_games(kernel);
    let shown_releases: usize =
        state.offers.iter().filter(|o| !hidden.contains(&o.entry.key.to_string())).map(|o| o.releases.len()).sum();
    // The newest count of seeders, else the newest date a catalog gives.
    let counted =
        releases.clone().filter_map(|r| crate::releases::days_of(&r.counted_at).map(|_| &r.counted_at[..10])).max();
    let dated = || {
        state
            .catalogs
            .iter()
            .filter_map(|c| crate::releases::days_of(&c.list.updated).map(|_| &c.list.updated[..10]))
            .max()
    };
    // Only the Catalog shows its grid: on another screen, the last one stands.
    let games = match &state.catalog_grid {
        Some(games) if state.nav.last() != Some(&Route::Catalog) => games.clone(),
        _ => grid_vm(kernel, state, torrents, true),
    };
    CatalogVm {
        available: crate::Core::catalog_available(kernel),
        games,
        sources,
        releases: shown_releases,
        updated: counted.or_else(dated).unwrap_or_default().to_owned(),
        loading: !state.catalogs_read,
        actions: state.catalog_buttons.iter().filter(|s| s.kind == SlotKind::Action).cloned().collect(),
        status: state.catalog_buttons.iter().filter(|s| s.kind == SlotKind::Badge).cloned().collect(),
    }
}

fn home_vm(kernel: &Kernel, state: &AppState, torrents: &TorrentsSnapshot) -> HomeVm {
    let games: Vec<GameCardVm> = state.library.iter().map(|e| card(kernel, state, e)).collect();
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
    HomeVm { games, selected, loading: state.loading, groups, library: grid_vm(kernel, state, torrents, false) }
}

/// The art a game's Game Hub and Home's hero show beside its cover.
pub(crate) struct HubArt {
    pub background: Option<ImageRef>,
    pub logo: Option<ImageRef>,
    pub screenshots: Vec<ImageRef>,
}

/// A game's background, logo and screenshots: from the store's catalog, then a catalog's
/// release's store data, then (the background) the game's own details, when they are in.
pub(crate) fn hub_art(state: &AppState, key: &GameKey) -> HubArt {
    let release = release_metadata(state, key);
    let metas: Vec<&(PluginId, nexus_api::GameMetadata)> =
        state.meta.get(key).into_iter().chain(release.as_ref()).collect();
    let art = |field: fn(&nexus_api::GameMetadata) -> Option<&nexus_api::ImageSource>| {
        metas.iter().find_map(|(plugin, m)| field(m).map(|c| image(plugin, c)))
    };
    let own = state.details.get(key).and_then(|d| d.details.as_ref()).and_then(|d| d.background.as_ref());
    HubArt {
        background: art(|m| m.background.as_ref()).or_else(|| own.map(|c| image(&key.plugin, c))),
        logo: art(|m| m.logo.as_ref()),
        screenshots: metas
            .iter()
            .map(|(plugin, m)| m.screenshots.iter().map(|s| image(plugin, s)).collect::<Vec<_>>())
            .find(|shots| !shots.is_empty())
            .unwrap_or_default(),
    }
}

fn detail_vm(
    core: &Core,
    kernel: &Kernel,
    state: &AppState,
    torrents: &TorrentsSnapshot,
    key: &GameKey,
) -> Option<GameDetailVm> {
    let d = state.details.get(key)?;
    let details = d.details.as_ref()?;
    let summary = &details.summary;
    let manifest = kernel.manifest(&key.plugin).ok();
    let offer = state.offers.iter().find(|o| &o.entry.key == key);
    let owned = Owned::new(state, torrents);
    let has_installer = manifest.is_some_and(|m| m.capabilities.contains(&Capability::Installer));
    let installed = state.emulators.get(&key.plugin).is_some_and(|e| !e.installed.is_empty());
    // A Catalog offer plays as the installed game it is, and not at all before it is installed.
    let installed_as = offer.and_then(|o| installed_game(state, &owned, o));
    let played = installed_as.map(|g| &g.key).unwrap_or(key);
    let can = game_actions(kernel, state, key, offer.map(|o| (o, installed_as)));
    let needs_emulator = has_installer && !installed;
    // The program, not the console: "Launches with the KytyPS5 emulator".
    let name = manifest.map(|m| m.emulator_name().to_owned()).unwrap_or_default();
    let stats = crate::launch::play_stats(kernel, key);
    let meta = state.meta.get(key);
    // A release's store data fills in, field by field, what the store's catalog leaves out.
    let release = release_metadata(state, key);
    let metas: Vec<&(PluginId, nexus_api::GameMetadata)> = meta.into_iter().chain(release.as_ref()).collect();
    let text = |field: fn(&nexus_api::GameMetadata) -> &String| {
        metas.iter().map(|(_, m)| field(m)).find(|t| !t.is_empty()).cloned().unwrap_or_default()
    };
    let art = |field: fn(&nexus_api::GameMetadata) -> Option<&nexus_api::ImageSource>| {
        metas.iter().find_map(|(plugin, m)| field(m).map(|c| image(plugin, c)))
    };
    let shown = hub_art(state, key);
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
            Some(o) => format!(
                "In the catalog for {}.",
                kernel.manifest(&o.emulator).map(|m| m.name.clone()).unwrap_or_else(|_| o.emulator.to_string())
            ),
            None => format!("Installed locally. Launches with the {name} emulator."),
        },
    };
    let compatibility = offer.map(crate::releases::compatibility).unwrap_or_default();
    // Where the offer's downloads go, and the free space there. The console's folder may not
    // be there yet: the disk that holds it is the one the nearest folder above is on.
    let download_folder = offer
        .and_then(|o| core.torrent_folder(&o.emulator, &o.releases[0].platform, "x", None).ok())
        .and_then(|(_, folder)| folder.parent().map(std::path::Path::to_path_buf));
    let free = download_folder
        .as_ref()
        .and_then(|folder| folder.ancestors().find(|p| p.is_dir()).map(std::path::Path::to_path_buf))
        .and_then(|folder| kernel.hardware().free_bytes(&folder));
    let no_room = |r: &nexus_api::Release| match (r.size_bytes, free) {
        (Some(size), Some(free)) if !torrents.jobs.contains_key(&r.id) => {
            crate::torrents::no_room(size, free, "download folder").unwrap_or_default()
        }
        _ => String::new(),
    };
    let releases = offer.map_or(&[][..], |o| &o.releases[..]);
    let (this_pc, pc_warning) = on_this_pc(kernel, state, offer.map_or(&key.plugin, |o| &o.emulator), releases);
    // A game that starts here but doesn't fit says so instead, from its best release.
    let this_pc = match (this_pc, offer.map(|o| no_room(best_release(state, &owned, o))), free) {
        (Some(v), Some(short), Some(free)) if v.level != "no" && !short.is_empty() => Some(VerdictVm {
            level: "no".into(),
            headline: "Not enough space".into(),
            why: format!("{} free", crate::torrents::gigabytes(free)),
            ..v
        }),
        (this_pc, _, _) => this_pc,
    };
    Some(GameDetailVm {
        key: key.to_string(),
        title: summary.title.clone(),
        platform: summary.platform.clone(),
        system: system_of(kernel, &summary.platform),
        summary: summary_of(&line),
        description,
        developer,
        genres,
        cover: art(|m| m.cover.as_ref().or(m.icon.as_ref()))
            .or_else(|| summary.cover.as_ref().map(|c| image(&key.plugin, c)))
            .or_else(|| state.offer_art.get(key).map(|(plugin, c)| image(plugin, c))),
        background: shown.background,
        logo: shown.logo,
        rating: meta.and_then(|(_, m)| m.rating),
        release: text(|m| &m.release),
        age_rating: meta.map(|(_, m)| m.age_rating.clone()).unwrap_or_default(),
        screenshots: shown.screenshots,
        store_url: text(|m| &m.store_url),
        protondb: compatibility.0,
        protondb_url: compatibility.1,
        this_pc,
        pc_warning,
        your_result: core.your_result(key).unwrap_or_default(),
        has_folder: details.folder.as_ref().and_then(|f| crate::app::resolve_game_folder(kernel, key, f)).is_some(),
        has_log: crate::launch::log_path(kernel, key).is_file(),
        hidden: crate::Core::hidden_games(kernel).contains(&key.to_string()),
        compat_page: d.compat_page.clone(),
        trailer_url: meta.map(|(_, m)| m.trailer.clone()).unwrap_or_default(),
        compat: d.compat.clone(),
        badges: state.badges.get(key).cloned().unwrap_or_default(),
        actions: d.actions.clone(),
        sections: d.sections.clone(),
        can: can.clone(),
        settings: d.settings.iter().cloned().collect(),
        launch_hint: if can.play && needs_emulator { format!("Play installs {name} first") } else { String::new() },
        running: state.sessions.values().any(|s| &s.game == played),
        size_bytes: details.size_bytes,
        facts: details.facts.clone(),
        played_secs: stats.secs,
        last_played_days: (stats.count > 0).then(|| crate::launch::days_since(stats.last)),
        // No plugin reports a shader cache: the plugin API has no seam for one.
        shader_cache: None,
        releases: offer
            .map(|o| {
                let today = crate::releases::today();
                let best = &best_release(state, &owned, o).id;
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
                        store: if r.store.is_empty() { String::new() } else { system_of(kernel, &r.platform).name },
                        version: r.version.clone(),
                        region: r.region.clone(),
                        size_bytes: r.size_bytes,
                        added: r.added.clone(),
                        new: crate::releases::is_new(&r.added, today),
                        description: r.description.clone(),
                        system: crate::releases::system_label(&r.system).into(),
                        not_here: crate::releases::not_here(&r.system, kernel.host().platform),
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
                kernel
                    .manifest(&o.emulator)
                    .map(|m| m.emulator_name().to_owned())
                    .unwrap_or_else(|_| o.emulator.to_string())
            })
            .unwrap_or_default(),
        download_folder: download_folder
            .map(|p| short_home(&p.display().to_string(), kernel.host().home.as_ref().and_then(|h| h.to_str())))
            .unwrap_or_default(),
        availability: offer
            .map(|o| crate::releases::availability(best_release(state, &owned, o).seeders).to_owned())
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
fn on_this_pc(
    kernel: &Kernel,
    state: &AppState,
    emulator: &PluginId,
    releases: &[nexus_api::Release],
) -> (Option<VerdictVm>, String) {
    let Some(manifest) = kernel.manifest(emulator).ok().filter(|m| m.kind == PluginKind::Emulator) else {
        return (None, String::new());
    };
    let here = kernel.host().platform;
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
    let Some(specs) = &state.hardware else { return (None, String::new()) };
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

fn downloads_vm(kernel: &Kernel, jobs: Vec<Job>, state: &AppState, torrents: &TorrentsSnapshot) -> DownloadsVm {
    // Game installs, newest first, then the downloads. A catalog download's install shows on
    // the download's own card (`torrent_vm`), so it has no card of its own.
    let on_a_card = |j: &crate::game_installs::GameJob| {
        torrents.jobs.values().any(|t| t.emulator == j.plugin && t.source == j.source)
    };
    let installs = state.game_installs.iter().rev().filter(|(_, j)| !on_a_card(j)).map(|(id, j)| DownloadVm {
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
    let downloads = jobs.into_iter().rev().map(|j| DownloadVm {
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
    let jobs = installs.chain(downloads).collect();
    let home = kernel.host().home.as_ref().and_then(|h| h.to_str()).map(str::to_owned);
    let owned = Owned::new(state, torrents);
    let mut cards: Vec<TorrentVm> = torrents
        .jobs
        .values()
        .map(|j| TorrentVm {
            folder: short_home(&j.folder.display().to_string(), home.as_deref()),
            install_note: install_note(kernel, &j.emulator),
            install_folder: crate::release_downloads::install_folder(kernel, &j.emulator, None)
                .map(|(_, path)| short_home(&path.display().to_string(), home.as_deref()))
                .unwrap_or_default(),
            ..torrent_vm(state, &owned, j)
        })
        .collect();
    cards.reverse();
    // The installs stores do (Steam's), after the downloads: the same cards, with the store's
    // own page as their one action.
    cards.extend(state.store_installs.iter().map(|(plugin, install)| store_install_vm(kernel, plugin, install)));
    let downloading = torrents.jobs.values().filter(|j| j.state.active()).count();
    let sharing = torrents.jobs.values().filter(|j| j.seeding).count();
    DownloadsVm { jobs, torrents: cards, seeding: torrents.sharing, downloading, sharing }
}

/// A store's install as a Downloads card: its phase and bytes, and the store's page.
fn store_install_vm(kernel: &Kernel, plugin: &PluginId, install: &nexus_api::StoreInstall) -> TorrentVm {
    use nexus_api::StorePhase as P;
    let store = kernel
        .manifest(plugin)
        .ok()
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
fn install_note(kernel: &Kernel, emulator: &PluginId) -> String {
    let own = kernel.manifest(emulator).map(|m| m.install_note.clone()).unwrap_or_default();
    let archives =
        "Nexus first unpacks the RAR, ZIP, 7z and TAR archives in the download, but not one with a password.";
    if own.is_empty() { archives.to_owned() } else { format!("{own} {archives}") }
}

/// Sizes for file lists, as the theme's other sizes: `1.5 GB`, `340 MB`.
fn bytes_label(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

fn torrent_vm(state: &AppState, owned: &Owned, j: &TorrentJob) -> TorrentVm {
    use crate::torrents::TorrentState as S;
    let install = state.game_installs.values().rev().find(|g| g.plugin == j.emulator && g.source == j.source);
    let installing = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Installing);
    let installed = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Done);
    // The installed game: the one this download's install reported, else the offer's, as its
    // Catalog card finds it (by title id, or by name: a PC game has no title id). The install's
    // record is gone after a restart.
    let offer = state.offers.iter().find(|o| o.entry.key == j.offer);
    let game = installed_from(state, j)
        .or_else(|| offer.and_then(|o| installed_game(state, owned, o)))
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

fn system_vm(kernel: &crate::Kernel, state: &AppState) -> nexus_ui_api::SystemVm {
    use nexus_api::{Capability, Connection, OsUpdate};
    use nexus_ui_api::{AudioOutputVm, BluetoothDeviceVm, InstallDiskVm, SystemJobVm, WifiVm};
    let system = &state.system;
    let place = kernel.host().place;
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

fn launcher_update_vm(core: &Core, update: &crate::launcher_update::UpdateState) -> nexus_ui_api::LauncherUpdateVm {
    use crate::launcher_update::{UpdateState, repo};
    let current = env!("CARGO_PKG_VERSION").to_owned();
    let off = repo(core).is_none();
    let (state, version, text) = match update {
        _ if off => ("off", String::new(), "Updates are off: no release repo is set".to_owned()),
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
    let method = core.kernel().host().install.method().to_owned();
    nexus_ui_api::LauncherUpdateVm { state: state.into(), current, version, text, method }
}

fn settings_vm(core: &Core, prefs: &PreferencesSnapshot, state: &AppState) -> SettingsVm {
    let kernel = core.kernel();
    let manifests: Vec<_> = kernel
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter_map(|id| {
            kernel.manifest(&id).ok().filter(|m| m.capabilities.contains(&Capability::Launcher)).map(|m| (id, m))
        })
        .collect();
    let hardware = crate::hardware::hardware_vm(
        state.hardware.as_ref(),
        manifests.iter().map(|(id, m)| (id.to_string(), m.name.clone(), &m.requires)),
    );
    let themes = kernel
        .of_kind(PluginKind::Theme)
        .into_iter()
        .map(|id| ThemeChoiceVm {
            name: kernel.manifest(&id).map(|m| m.name.clone()).unwrap_or_default(),
            current: state.theme.as_ref() == Some(&id),
            id,
        })
        .collect();
    let theme_options = match &state.theme {
        Some(theme) => {
            let chain = theme_chain(kernel, theme);
            let refs: Vec<&ThemeManifest> = chain.iter().collect();
            let schema = options_schema(&merge_options(&refs));
            fields(&schema, &core.theme_options(theme))
        }
        None => Vec::new(),
    };
    SettingsVm {
        launcher_update: launcher_update_vm(core, &state.launcher_update),
        hardware,
        panels: state.settings.clone(),
        themes,
        theme_options,
        // The game folders. The other folders are on the pages of the plugins that ask for them.
        folders: folders_vm(kernel).into_iter().filter(|f| nexus_api::shared_folder(&f.name).is_some()).collect(),
        sounds: prefs.sounds,
        fullscreen: prefs.fullscreen,
        return_on_exit: prefs.return_on_exit,
        auto_update: prefs.auto_update,
        scale: prefs.scale,
        can_start_at_login: kernel.desktop().can_start_at_login(),
        start_at_login: prefs.start_at_login,
        start_at_login_error: prefs.start_at_login_error.clone().unwrap_or_default(),
        vpn: vpn_vm(core, prefs, state),
    }
}

fn vpn_vm(core: &Core, prefs: &PreferencesSnapshot, state: &AppState) -> nexus_ui_api::VpnSettingsVm {
    let network = core.kernel().network();
    let only = prefs.vpn_only && network.problem().is_none();
    let chosen = prefs.vpn.clone();
    let finds = network.finds_vpns();
    nexus_ui_api::VpnSettingsVm {
        problem: network.problem().unwrap_or_default(),
        only,
        finds,
        can_import: network.can_import(),
        route: if only { crate::vpn::pick(&state.vpns, &chosen, finds).unwrap_or_default() } else { String::new() },
        chosen,
        vpns: state
            .vpns
            .iter()
            .map(|v| nexus_ui_api::VpnVm {
                name: v.name.clone(),
                kind: v.kind.clone(),
                connected: v.connected,
                switchable: v.switchable,
            })
            .collect(),
        error: state.vpn_error.clone().unwrap_or_default(),
    }
}

fn folders_vm(kernel: &Kernel) -> Vec<FolderVm> {
    let mut by_name: BTreeMap<String, FolderVm> = BTreeMap::new();
    for plugin in kernel.order() {
        let Ok(manifest) = kernel.manifest(plugin) else { continue };
        for grant in &manifest.permissions.filesystem {
            let home = kernel.host().home.as_deref();
            let folder = by_name.entry(grant.folder.clone()).or_insert_with(|| {
                let paths: Vec<String> = crate::folders::roots(kernel.config(), home, kernel.host().platform, grant)
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
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
                && let Some(default) =
                    grant.default_path(kernel.host().platform).and_then(|d| crate::folders::expand(d, home))
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
    let home = kernel.host().home.as_deref().map(|h| h.display().to_string());
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

fn emulators_vm(kernel: &Kernel, state: &AppState, statuses: &BTreeMap<PluginId, PluginStatus>) -> EmulatorsVm {
    let manages_an_emulator = |id: &PluginId| {
        kernel.manifest(id).is_ok_and(|m| {
            m.capabilities.contains(&Capability::Installer) || m.capabilities.contains(&Capability::Launcher)
        })
    };
    let emulators = kernel
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter(manages_an_emulator)
        .map(|id| {
            let emu = state.emulators.get(&id).cloned().unwrap_or_default();
            EmulatorVm {
                name: kernel.manifest(&id).map(|m| m.emulator_name().to_owned()).unwrap_or_default(),
                status: status_text(statuses.get(&id)),
                has_builds: kernel.manifest(&id).is_ok_and(|m| m.capabilities.contains(&Capability::Installer)),
                update: update_for(&emu),
                installed: emu.installed,
                available: emu.available,
                busy: emu.busy,
                panels: emu.panels,
                controllers: controllers(kernel.manifest(&id).ok()),
                plugin: id,
            }
        })
        .collect();
    let plugins = kernel
        .order()
        .iter()
        .filter_map(|id| kernel.manifest(id).ok().map(|m| (id, m)))
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
            status: status_text(statuses.get(id)),
            enabled: kernel.enabled(id),
            build: state.emulators.get(id).and_then(|e| e.installed.last().cloned()).unwrap_or_default(),
            build_label: build_label(
                m.emulator_name(),
                state.emulators.get(id).and_then(|e| e.installed.last()).map_or("", String::as_str),
            ),
            has_builds: m.capabilities.contains(&Capability::Installer),
            note: if kernel.elsewhere(id) {
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
    EmulatorsVm { emulators, plugins }
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

fn plugin_page_vm(
    kernel: &Kernel,
    state: &AppState,
    statuses: &BTreeMap<PluginId, PluginStatus>,
    plugin: &PluginId,
) -> Option<PluginPageVm> {
    let all = emulators_vm(kernel, state, statuses);
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
    let asks: Vec<&str> = kernel
        .manifest(plugin)
        .map(|m| m.permissions.filesystem.iter().map(|g| g.folder.as_str()).collect())
        .unwrap_or_default();
    Some(PluginPageVm {
        emulator: all.emulators.into_iter().find(|e| &e.plugin == plugin),
        panels: state.settings.iter().filter(|p| &p.slot.plugin == plugin).cloned().collect(),
        sections: state.emulators.get(plugin).map(|e| e.panels.clone()).unwrap_or_default(),
        folders: folders_vm(kernel)
            .into_iter()
            .filter(|f| nexus_api::shared_folder(&f.name).is_none() && asks.contains(&f.name.as_str()))
            .collect(),
        plugin: row,
    })
}

/// The most covers the boot screen's wall shows.
const BOOT_COVERS: usize = 30;

fn boot_vm(kernel: &Kernel, prefs: &PreferencesSnapshot, state: &AppState) -> BootVm {
    let ready = state.booted;
    let games = state.library.len();
    let step = |label: String, detail: String| BootStepVm {
        label,
        detail,
        state: if ready { "done".into() } else { "working".into() },
    };
    let mut steps = vec![step("Game folders".into(), if ready { Look::games(games) } else { "Looking…".into() })];
    for (plugin, emu) in &state.emulators {
        let Ok(manifest) = kernel.manifest(plugin) else { continue };
        if !manifest.capabilities.contains(&Capability::Installer) {
            continue;
        }
        let detail = if emu.own {
            "Your own build"
        } else if emu.installed.is_empty() {
            "Installs on first play"
        } else if update_for(emu).is_some() {
            "Update available"
        } else {
            "Latest build"
        };
        steps.push(step(format!("{} emulator", manifest.emulator_name()), detail.into()));
    }
    if let Some((done, total)) = state.describing {
        steps.push(BootStepVm {
            label: "Official artwork & details".into(),
            detail: if state.described {
                "Done".into()
            } else if total > 0 {
                format!("{done}/{total}")
            } else {
                "Looking…".into()
            },
            state: if state.described { "done".into() } else { "working".into() },
        });
    }
    steps.push(step("Covers & Home screen".into(), if ready { "Ready".into() } else { "Loading…".into() }));
    BootVm {
        ready,
        welcome: !prefs.welcomed,
        progress: if ready {
            1.0
        } else if state.loading {
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
        covers: state.library.iter().filter_map(|e| cover_of(state, e)).take(BOOT_COVERS).collect(),
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
fn running_vm(kernel: &Kernel, state: &AppState, route: &Route) -> Option<RunningVm> {
    let session = match route {
        Route::Running(session) => *session,
        _ => *state.sessions.keys().next_back()?,
    };
    let s = state.sessions.get(&session)?;
    let manifest = kernel.manifest(&s.game.plugin).ok();
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
        elapsed_secs: kernel.clock().now().saturating_duration_since(s.started).as_secs(),
        in_front: state.game_in_front,
        keymap: manifest.map(|m| m.keymap.clone()).unwrap_or_default(),
    })
}

/// A theme's manifest and its parents' (the theme first). Stops at a missing or bad manifest.
fn theme_chain(kernel: &crate::Kernel, theme: &PluginId) -> Vec<ThemeManifest> {
    let mut chain: Vec<ThemeManifest> = Vec::new();
    let mut next = Some(theme.clone());
    while let Some(id) = next.take() {
        if chain.iter().any(|m| m.id() == &id) {
            break;
        }
        let Some(manifest) = kernel.manifest(&id).ok().and_then(|m| ThemeManifest::from_plugin(m).ok()) else { break };
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

    #[test]
    fn settings_fields_follow_the_schema_s_order_when_it_names_one() {
        let schema = serde_json::json!({
            "x-order": ["fullscreen", "width", "amd_cpu"],
            "properties": {
                "amd_cpu": { "type": "boolean" },
                "extra": { "type": "string" },
                "fullscreen": { "type": "boolean" },
                "width": { "type": "integer" },
            },
        });
        let keys: Vec<_> = fields(&schema, &Value::Null).into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["fullscreen", "width", "amd_cpu", "extra"], "unnamed fields come last");

        let unordered = serde_json::json!({ "properties": { "b": { "type": "string" }, "a": { "type": "string" } } });
        let keys: Vec<_> = fields(&unordered, &Value::Null).into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["a", "b"]);
    }

    #[test]
    fn a_choice_can_be_a_reference_to_a_list_of_constants() {
        // What schemars makes of a Rust enum whose variants have doc comments.
        let schema = serde_json::json!({
            "properties": { "mode": { "title": "Present mode", "$ref": "#/$defs/Mode", "default": "Mailbox" } },
            "$defs": { "Mode": { "oneOf": [
                { "type": "string", "const": "Mailbox", "description": "Low latency." },
                { "type": "string", "const": "Fifo" },
            ] } },
        });
        let field = fields(&schema, &Value::Null).remove(0);
        assert_eq!(
            (field.label.as_str(), field.kind, field.value.as_str()),
            ("Present mode", FieldKind::Choice, "Mailbox")
        );
        assert_eq!(field.choices, ["Mailbox", "Fifo"]);
        assert_eq!(field.choice_labels, ["Low latency.", ""], "a doc comment names its choice");
    }

    #[test]
    fn variants_without_doc_comments_are_choices_too() {
        // schemars groups the variants with no doc comment into one `enum` entry.
        let schema = serde_json::json!({
            "properties": { "out": { "title": "Video out", "$ref": "#/$defs/Out" } },
            "$defs": { "Out": { "oneOf": [
                { "type": "string", "enum": ["FullHd", "Uhd"] },
                { "type": "string", "const": "Title", "description": "Game default" },
            ] } },
        });
        let field = fields(&schema, &Value::Null).remove(0);
        assert_eq!(field.choices, ["FullHd", "Uhd", "Title"]);
        assert_eq!(field.choice_labels, ["", "", "Game default"]);
    }
}
