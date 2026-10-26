//! Turning view models, tokens and images into Slint values on the theme's globals.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use nexus_ui_api::*;
use slint::{Model, ModelRc, VecModel};
use slint_interpreter::{Brush, Color, ComponentInstance, Image, SetPropertyError, SharedString, Struct, Value};

/// Images the view refers to. Unknown ones show as empty until loaded.
pub trait ImageSource {
    /// The image at `size`, with `crop` cut off, if it is in, else an empty one; queued for
    /// loading when `want`.
    fn image(&self, image: &ImageRef, size: crate::ImageSize, crop: crate::Crop, want: bool) -> Image;
    /// Whether the image at `size`, with `crop` cut off, couldn't load.
    fn failed(&self, image: &ImageRef, size: crate::ImageSize, crop: crate::Crop) -> bool;
    /// Whether card `index` of the grid named `grid` is on screen or near it.
    fn near(&self, grid: &str, index: usize) -> bool;
}

fn text(s: impl AsRef<str>) -> Value {
    Value::String(SharedString::from(s.as_ref()))
}

fn number(n: impl Into<f64>) -> Value {
    Value::Number(n.into())
}

fn model(values: Vec<Value>) -> Value {
    Value::Model(ModelRc::new(VecModel::from(values)))
}

fn hardware(h: &nexus_ui_api::HardwareVm) -> Value {
    record([
        ("known", Value::Bool(h.known)),
        ("cpu", text(&h.cpu)),
        ("memory", text(&h.memory)),
        ("graphics", strings(&h.graphics)),
        ("verdicts", model(h.verdicts.iter().map(verdict).collect())),
    ])
}

fn verdict(v: &nexus_ui_api::VerdictVm) -> Value {
    record([
        ("plugin", text(&v.plugin)),
        ("emulator", text(&v.emulator)),
        ("level", text(&v.level)),
        ("headline", text(&v.headline)),
        ("why", text(&v.why)),
    ])
}

fn system(s: &nexus_ui_api::SystemVm) -> Value {
    record([
        ("whole-screen", Value::Bool(s.whole_screen)),
        ("nexus-os", Value::Bool(s.nexus_os)),
        ("stick", Value::Bool(s.stick)),
        ("network-available", Value::Bool(s.network_available)),
        ("network-reason", text(&s.network_reason)),
        ("connection", text(&s.connection)),
        ("network-name", text(&s.network_name)),
        ("signal-bars", number(s.signal_bars)),
        (
            "wifi",
            model(
                s.wifi
                    .iter()
                    .map(|w| {
                        record([
                            ("ssid", text(&w.ssid)),
                            ("bars", number(w.bars)),
                            ("secured", Value::Bool(w.secured)),
                            ("saved", Value::Bool(w.saved)),
                            ("active", Value::Bool(w.active)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("audio-available", Value::Bool(s.audio_available)),
        ("audio-reason", text(&s.audio_reason)),
        ("volume", number(s.volume)),
        ("muted", Value::Bool(s.muted)),
        (
            "outputs",
            model(
                s.outputs
                    .iter()
                    .map(|o| {
                        record([
                            ("name", text(&o.name)),
                            ("description", text(&o.description)),
                            ("current", Value::Bool(o.current)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("bluetooth-available", Value::Bool(s.bluetooth_available)),
        ("bluetooth-reason", text(&s.bluetooth_reason)),
        ("bluetooth-on", Value::Bool(s.bluetooth_on)),
        ("connected-devices", number(s.connected_devices)),
        (
            "devices",
            model(
                s.devices
                    .iter()
                    .map(|d| {
                        record([
                            ("address", text(&d.address)),
                            ("name", text(&d.name)),
                            ("paired", Value::Bool(d.paired)),
                            ("connected", Value::Bool(d.connected)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("power-available", Value::Bool(s.power_available)),
        ("power-reason", text(&s.power_reason)),
        ("battery-shown", Value::Bool(s.battery_shown)),
        ("battery-percent", number(s.battery_percent)),
        ("battery-charging", Value::Bool(s.battery_charging)),
        ("os-update", text(&s.os_update)),
        ("os-update-text", text(&s.os_update_text)),
        (
            "disks",
            model(
                s.disks
                    .iter()
                    .map(|d| {
                        record([
                            ("path", text(&d.path)),
                            ("size", text(&d.size)),
                            ("windows", Value::Bool(d.windows)),
                            ("usb", Value::Bool(d.usb)),
                            ("model", text(&d.model)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "jobs",
            model(
                s.jobs
                    .iter()
                    .map(|j| {
                        record([
                            ("id", number(j.id)),
                            ("kind", text(&j.kind)),
                            ("target", text(&j.target)),
                            ("state", text(&j.state)),
                            ("percent", number(j.percent)),
                            ("text", text(&j.text)),
                            ("mok", Value::Bool(j.mok)),
                            ("nvidia-later", Value::Bool(j.nvidia_later)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn strings(items: &[String]) -> Value {
    model(items.iter().map(text).collect())
}

fn record<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Struct(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Struct>())
}

/// A background, logo or screenshot.
fn image(images: &dyn ImageSource, r: &Option<ImageRef>) -> Value {
    sized(images, r, crate::ImageSize::Full, crate::Crop::None, true)
}

/// A cover, tile or icon; loaded only when `want`.
fn thumb(images: &dyn ImageSource, r: &Option<ImageRef>, want: bool) -> Value {
    sized(images, r, crate::ImageSize::Thumb, crate::Crop::None, want)
}

/// The crop of a game's cover: a PS4 cover loses the blue "PS4" band of its box art.
fn cover_crop(platform: &str) -> crate::Crop {
    if platform == "PS4" { crate::Crop::Ps4Band } else { crate::Crop::None }
}

/// A game's cover at `size`.
fn cover(images: &dyn ImageSource, r: &Option<ImageRef>, platform: &str, size: crate::ImageSize, want: bool) -> Value {
    sized(images, r, size, cover_crop(platform), want)
}

/// `GameCard.cover-state` for the cover `shown`: "loading", "ready" or "none".
fn cover_state(images: &dyn ImageSource, r: &Option<ImageRef>, platform: &str, shown: &Value) -> Value {
    let state = match r {
        None => "none",
        Some(_) if matches!(shown, Value::Image(i) if i.size().width > 0) => "ready",
        Some(r) if images.failed(r, crate::ImageSize::Thumb, cover_crop(platform)) => "none",
        Some(_) => "loading",
    };
    text(state)
}

fn sized(
    images: &dyn ImageSource,
    r: &Option<ImageRef>,
    size: crate::ImageSize,
    crop: crate::Crop,
    want: bool,
) -> Value {
    Value::Image(r.as_ref().map(|r| images.image(r, size, crop, want)).unwrap_or_default())
}

fn json_text<T: serde::Serialize>(value: &T) -> Value {
    text(serde_json::to_string(value).unwrap_or_default())
}

/// Serde names of an enum value, such as `success` for `Tone::Success`.
fn enum_text<T: serde::Serialize>(value: &T) -> Value {
    text(serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default())
}

fn slot(images: &dyn ImageSource, s: &SlotVm) -> Value {
    record([
        ("slot", text(s.slot.as_str())),
        ("plugin", text(s.plugin.as_str())),
        ("id", text(&s.id)),
        ("label", text(&s.label)),
        ("kind", enum_text(&s.kind)),
        ("text", text(&s.text)),
        ("short", text(&s.short)),
        ("tone", enum_text(&s.tone)),
        ("style", enum_text(&s.style)),
        ("icon", thumb(images, &s.icon, true)),
        (
            "rows",
            model(s.rows.iter().map(|r| record([("label", text(&r.label)), ("value", text(&r.value))])).collect()),
        ),
        ("action", text(s.action.as_deref().unwrap_or_default())),
        ("args", if s.args.is_null() { text("") } else { json_text(&s.args) }),
        ("scope", s.scope.as_ref().map(json_text).unwrap_or_else(|| text(""))),
    ])
}

fn slots(images: &dyn ImageSource, items: &[SlotVm]) -> Value {
    model(items.iter().map(|s| slot(images, s)).collect())
}

/// What can be done with a game, as the theme contract's `GameActions`.
fn game_actions(can: &nexus_ui_api::GameActionsVm) -> Value {
    record([
        ("primary", text(&can.primary)),
        ("play", Value::Bool(can.play)),
        ("get", text(&can.get)),
        ("get-release", text(&can.get_release)),
        ("get-store", text(&can.get_store)),
        ("uninstall", text(&can.uninstall)),
        ("uninstall-store", text(&can.uninstall_store)),
    ])
}

/// A game's card; its cover is loaded when `want`.
fn card(images: &dyn ImageSource, g: &GameCardVm, want: bool) -> Value {
    let shown = cover(images, &g.cover, &g.platform, crate::ImageSize::Thumb, want);
    let state = cover_state(images, &g.cover, &g.platform, &shown);
    card_with(images, g, shown, state)
}

/// A game's card with its cover as it is now (`shown`, `state`).
fn card_with(images: &dyn ImageSource, g: &GameCardVm, shown: Value, state: Value) -> Value {
    record([
        ("key", text(&g.key)),
        ("title", text(&g.title)),
        ("platform", text(&g.platform)),
        ("system-name", text(&g.system.name)),
        ("system-color", system_color(&g.system)),
        ("cover-state", state),
        ("cover", shown),
        ("selected", Value::Bool(g.selected)),
        ("badges", slots(images, &g.badges)),
        ("release-year", number(g.release_year.unwrap_or(0))),
        ("rating", number(g.rating.map(f64::from).unwrap_or(0.0))),
        ("size", text(g.size_bytes.map(size_text).unwrap_or_default())),
        ("last-played", text(g.last_played_days.map(days_text).unwrap_or_default())),
        ("releases", number(g.releases as f64)),
        ("availability", text(&g.availability)),
        ("availability-text", text(availability_text(&g.availability))),
        ("hidden", Value::Bool(g.hidden)),
        ("can", game_actions(&g.can)),
    ])
}

fn filters(filters: &[nexus_ui_api::FilterVm]) -> Value {
    model(
        filters
            .iter()
            .map(|f| {
                record([
                    ("id", text(&f.id)),
                    ("label", text(&f.label)),
                    ("count", number(f.count as f64)),
                    ("on", Value::Bool(f.on)),
                ])
            })
            .collect(),
    )
}

/// The Library's or the Catalog's games, filters and sort. Only the covers of the cards near
/// the screen of the grid named `grid` are loaded.
fn library(images: &dyn ImageSource, cards: &CardModels, l: &nexus_ui_api::LibraryVm, grid: &'static str) -> Value {
    record([
        ("games", cards.list(grid, images, &l.games, |i| images.near(grid, i))),
        ("total", number(l.total as f64)),
        ("all", number(l.all as f64)),
        ("selected", number(l.selected.map(|i| i as f64).unwrap_or(-1.0))),
        ("systems", filters(&l.systems)),
        ("genres", filters(&l.genres)),
        ("sort", text(&l.sort)),
        ("query", text(&l.query)),
    ])
}

/// Cards whose covers all load: Home's groups, which have the installed games.
fn group_cards(images: &dyn ImageSource, games: &[GameCardVm]) -> Value {
    model(games.iter().map(|g| card(images, g, true)).collect())
}

/// The long lists of cards (the Catalog's, the Library's, Home's), each as last shown: the card
/// data and its cover's state, and the list the theme has. A render converts only the cards
/// that changed, and changes only their rows: the rest, often thousands, stay as they are.
/// Each card still asks for its cover, so the covers it shows stay in and load.
#[derive(Default)]
pub struct CardModels(RefCell<HashMap<&'static str, CardList>>);

#[derive(Default)]
struct CardList {
    shown: Vec<(GameCardVm, bool, Value)>,
    model: Rc<VecModel<Value>>,
}

impl CardModels {
    /// A new theme: its lists start empty.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// The list named `name` with `games`; card `i`'s cover loads when `want(i)`.
    fn list(
        &self,
        name: &'static str,
        images: &dyn ImageSource,
        games: &[GameCardVm],
        want: impl Fn(usize) -> bool,
    ) -> Value {
        let mut lists = self.0.borrow_mut();
        let list = lists.entry(name).or_default();
        for (i, g) in games.iter().enumerate() {
            let shown = cover(images, &g.cover, &g.platform, crate::ImageSize::Thumb, want(i));
            let loaded = matches!(&shown, Value::Image(image) if image.size().width > 0);
            let state = cover_state(images, &g.cover, &g.platform, &shown);
            if list.shown.get(i).is_some_and(|(old, was, old_state)| old == g && *was == loaded && *old_state == state)
            {
                continue;
            }
            let row = card_with(images, g, shown, state.clone());
            if i < list.model.row_count() {
                list.model.set_row_data(i, row);
                list.shown[i] = (g.clone(), loaded, state);
            } else {
                list.model.push(row);
                list.shown.push((g.clone(), loaded, state));
            }
        }
        while list.model.row_count() > games.len() {
            list.model.remove(list.model.row_count() - 1);
        }
        list.shown.truncate(games.len());
        Value::Model(ModelRc::from(list.model.clone()))
    }
}

fn detail(images: &dyn ImageSource, d: Option<&GameDetailVm>) -> Value {
    let Some(d) = d else { return Value::Void };
    let compat = d.compat.clone().unwrap_or_default();
    record([
        ("key", text(&d.key)),
        ("title", text(&d.title)),
        ("platform", text(&d.platform)),
        ("system-name", text(&d.system.name)),
        ("system-color", system_color(&d.system)),
        ("description", text(&d.description)),
        ("summary", text(&d.summary)),
        ("developer", text(&d.developer)),
        ("genres", strings(&d.genres)),
        ("cover", cover(images, &d.cover, &d.platform, crate::ImageSize::Full, true)),
        ("background", image(images, &d.background)),
        ("has-compat", Value::Bool(d.compat.is_some())),
        (
            "compat",
            record([
                ("level", text(compat.level)),
                ("label", text(compat.label)),
                ("notes", text(compat.notes)),
                ("tested", text(release_text(&compat.tested))),
            ]),
        ),
        ("badges", slots(images, &d.badges)),
        ("actions", slots(images, &d.actions)),
        ("sections", slots(images, &d.sections)),
        ("can", game_actions(&d.can)),
        ("settings", model(d.settings.iter().map(|p| panel(images, p)).collect())),
        ("launch-hint", text(&d.launch_hint)),
        ("running", Value::Bool(d.running)),
        ("size", text(d.size_bytes.map(size_text).unwrap_or_default())),
        ("has-logo", Value::Bool(d.logo.is_some())),
        ("logo", image(images, &d.logo)),
        ("rating", number(d.rating.map(|r| f64::from(r.score)).unwrap_or(0.0))),
        ("ratings", number(d.rating.map(|r| r.total as f64).unwrap_or(0.0))),
        ("rating-text", text(d.rating.map(rating_text).unwrap_or_default())),
        ("release", text(release_text(&d.release))),
        ("year", text(d.release.get(..4).unwrap_or_default())),
        ("age-rating", text(&d.age_rating)),
        ("screenshots", model(d.screenshots.iter().map(|s| image(images, &Some(s.clone()))).collect())),
        ("store-url", text(&d.store_url)),
        ("protondb", text(&d.protondb)),
        ("protondb-url", text(&d.protondb_url)),
        ("has-this-pc", Value::Bool(d.this_pc.is_some())),
        ("this-pc", verdict(&d.this_pc.clone().unwrap_or_default())),
        ("pc-warning", text(&d.pc_warning)),
        ("your-result", text(&d.your_result)),
        ("has-folder", Value::Bool(d.has_folder)),
        ("has-log", Value::Bool(d.has_log)),
        ("hidden", Value::Bool(d.hidden)),
        ("compat-page", text(&d.compat_page)),
        ("trailer-url", text(&d.trailer_url)),
        ("region", text(fact(d, "region"))),
        ("age", text(fact(d, "age"))),
        ("version", text(fact(d, "version"))),
        (
            "facts",
            model(
                d.facts
                    .iter()
                    .map(|f| record([("id", text(&f.id)), ("label", text(&f.label)), ("value", text(&f.value))]))
                    .collect(),
            ),
        ),
        ("played", text(if d.played_secs > 0 { duration_text(d.played_secs) } else { String::new() })),
        ("last-played", text(d.last_played_days.map(days_text).unwrap_or_default())),
        ("has-shader-cache", Value::Bool(d.shader_cache.is_some())),
        (
            "shader-cache",
            text(d.shader_cache.as_ref().map(|s| shader_text(s.entries, s.size_bytes)).unwrap_or_default()),
        ),
        (
            "releases",
            model(
                d.releases
                    .iter()
                    .map(|r| {
                        record([
                            ("id", text(&r.id)),
                            ("version", text(&r.version)),
                            ("region", text(&r.region)),
                            ("size", text(r.size_bytes.map(size_text).unwrap_or_default())),
                            ("added", text(release_text(&r.added))),
                            ("new", Value::Bool(r.new)),
                            ("description", text(&r.description)),
                            ("system", text(&r.system)),
                            ("not-here", text(&r.not_here)),
                            ("warning", text(&r.warning)),
                            ("no-room", text(&r.no_room)),
                            ("download", text(&r.download)),
                            ("store", text(&r.store)),
                            ("download-progress", number(r.download_progress)),
                            ("version-label", text(&r.version_label)),
                            ("best", Value::Bool(r.best)),
                            ("seeders", text(r.seeders.map(|n| n.to_string()).unwrap_or_default())),
                            ("leechers", text(r.leechers.map(|n| n.to_string()).unwrap_or_default())),
                            ("counted", text(release_text(r.counted_at.get(..10).unwrap_or_default()))),
                            ("availability", text(&r.availability)),
                            ("availability-text", text(availability_text(&r.availability))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("emulator", text(&d.emulator)),
        ("download-folder", text(&d.download_folder)),
        ("availability", text(&d.availability)),
        ("availability-text", text(availability_text(&d.availability))),
        ("download-error", text(&d.download_error)),
    ])
}

/// An availability in words: `good` is "Good", `unknown` is "Unknown"; empty stays empty.
fn availability_text(level: &str) -> &'static str {
    match level {
        "good" => "Good",
        "low" => "Low",
        "none" => "None",
        "unknown" => "Unknown",
        _ => "",
    }
}

/// `4.4 · 3.2K ratings`.
fn rating_text(r: nexus_api::Rating) -> String {
    let noun = if r.total == 1 { "rating" } else { "ratings" };
    format!("{:.1} · {} {noun}", r.score, short_count(r.total))
}

/// `950`, `3.2K`, `10.6K`, `2M`: one decimal, none when it is 0.
fn short_count(n: u64) -> String {
    let (value, unit) = match n {
        0..1_000 => return n.to_string(),
        // Up to where one decimal would round to 1000.0K.
        1_000..999_950 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

/// `2024-03-12` as `Mar 12, 2024`, as the old launcher showed dates; empty stays empty.
fn release_text(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok());
    let month = date.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    match (date.get(..4), month, day) {
        (Some(y), Some(m), Some(d)) => format!("{} {d}, {y}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// The value of the game's fact `id`, or empty.
fn fact(d: &GameDetailVm, id: &str) -> String {
    d.facts.iter().find(|f| f.id == id).map(|f| f.value.clone()).unwrap_or_default()
}

/// `7.6 GB`, `820 MB`: decimal units, as the PS5 shows them.
fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// `4 hr 6 min`, `12 min`, or `Less than a minute`.
fn duration_text(secs: u64) -> String {
    let (hours, minutes) = (secs / 3600, secs % 3600 / 60);
    match (hours, minutes) {
        (0, 0) => "Less than a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// `2 hr 5 min left`, `12 min left`, or `under a minute left`.
fn time_left_text(secs: u64) -> String {
    if secs < 60 { "under a minute left".into() } else { format!("{} left", duration_text(secs)) }
}

/// `Today`, `Yesterday` or `3 days ago`.
fn days_text(days: u64) -> String {
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        n => format!("{n} days ago"),
    }
}

/// `120 shaders, 4.2 MB`.
fn shader_text(entries: u64, size_bytes: u64) -> String {
    let noun = if entries == 1 { "shader" } else { "shaders" };
    format!("{entries} {noun}, {:.1} MB", size_bytes as f64 / 1_000_000.0)
}

/// `12.5 MB of 40.0 MB`, or `12.5 MB` when the size is unknown.
fn progress_text(received: u64, total: Option<u64>) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    match total {
        Some(t) => format!("{} of {}", mb(received), mb(t)),
        None => mb(received),
    }
}

/// `3.1 MB/s`.
fn rate_text(bytes: u64) -> String {
    format!("{:.1} MB/s", bytes as f64 / 1e6)
}

/// One line about a download: its sizes, speed and peers, by its state.
fn torrent_detail(t: &nexus_ui_api::TorrentVm) -> String {
    let peers = if t.peers == 1 { "1 peer".to_owned() } else { format!("{} peers", t.peers) };
    match t.state.as_str() {
        "resolving" => "Reading the torrent's files from its peers; nothing downloads yet".into(),
        "ready" => "Look at its files below: nothing downloads until you press Download".into(),
        "checking" => "Checking the pieces already on disk".into(),
        "downloading" => {
            let left =
                if t.seconds_left > 0 { format!(" · {}", time_left_text(t.seconds_left)) } else { String::new() };
            format!("{} of {} · {} · {peers}{left}", size_text(t.done), size_text(t.total), rate_text(t.download_rate))
        }
        "paused" if t.waiting_vpn && t.file_count == 0 => {
            "It reads the torrent's files once your VPN is connected".into()
        }
        "paused" if t.waiting_vpn => {
            format!("{} of {} · it goes on once your VPN is connected", size_text(t.done), size_text(t.total))
        }
        "paused" | "cancelled" => format!("{} of {}", size_text(t.done), size_text(t.total)),
        "complete" if t.install == "installing" => {
            format!("Installing · {}% · it runs in the background", (t.install_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "unpacking" => {
            format!("Unpacking its archives · {}%", (t.unpack_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "cancelled" => "Unpacking stopped: Install unpacks the archives again".into(),
        "complete" if t.verifying => format!("{} · checking the files before sharing them", size_text(t.total)),
        "complete" if t.seeding => {
            format!("{} · sharing at {} · {peers}", size_text(t.total), rate_text(t.upload_rate))
        }
        "complete" => size_text(t.total),
        _ => String::new(),
    }
}

/// What the file list shows: `120 files · 1.5 GB · first 100 shown`.
fn file_summary(t: &nexus_ui_api::TorrentVm) -> String {
    let files = if t.file_count == 1 { "1 file".to_owned() } else { format!("{} files", t.file_count) };
    let shown = if t.files.len() < t.file_count { format!(" · first {} shown", t.files.len()) } else { String::new() };
    format!("{files} · {}{shown}", size_text(t.total))
}

fn torrent(t: &nexus_ui_api::TorrentVm) -> Value {
    record([
        ("id", text(&t.id)),
        ("title", text(&t.title)),
        ("version", text(&t.version)),
        ("region", text(&t.region)),
        ("state", text(&t.state)),
        ("progress", number(t.progress)),
        ("detail", text(torrent_detail(t))),
        ("seeding", Value::Bool(t.seeding)),
        ("verifying", Value::Bool(t.verifying)),
        ("waiting-vpn", Value::Bool(t.waiting_vpn)),
        ("error", text(&t.error)),
        ("files", strings(&t.files)),
        ("file-summary", text(file_summary(t))),
        ("file-count", number(t.file_count as f64)),
        ("folder", text(&t.folder)),
        ("actions", strings(&t.actions)),
        ("install", text(&t.install)),
        ("install-progress", number(t.install_progress)),
        ("unpack", text(&t.unpack)),
        ("unpack-progress", number(t.unpack_progress)),
        ("game", text(&t.game)),
        ("install-note", text(&t.install_note)),
        ("install-folder", text(&t.install_folder)),
        ("install-error", text(&t.install_error)),
    ])
}

fn vpn(v: &nexus_ui_api::VpnSettingsVm) -> Value {
    record([
        ("problem", text(&v.problem)),
        ("only", Value::Bool(v.only)),
        ("finds", Value::Bool(v.finds)),
        ("can-import", Value::Bool(v.can_import)),
        ("chosen", text(&v.chosen)),
        ("route", text(&v.route)),
        (
            "vpns",
            model(
                v.vpns
                    .iter()
                    .map(|i| {
                        record([
                            ("name", text(&i.name)),
                            ("kind", text(&i.kind)),
                            ("connected", Value::Bool(i.connected)),
                            ("switchable", Value::Bool(i.switchable)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("error", text(&v.error)),
    ])
}

fn field(f: &FieldVm) -> Value {
    record([
        ("key", text(&f.key)),
        ("label", text(&f.label)),
        ("help", text(&f.help)),
        ("kind", enum_text(&f.kind)),
        ("value", text(&f.value)),
        ("choices", strings(&f.choices)),
        ("labels", strings(&f.choice_labels)),
    ])
}

fn emulator(images: &dyn ImageSource, e: &EmulatorVm) -> Value {
    record([
        ("plugin", text(e.plugin.as_str())),
        ("name", text(&e.name)),
        ("status", text(&e.status)),
        ("has-builds", Value::Bool(e.has_builds)),
        ("installed", strings(&e.installed)),
        ("available", strings(&e.available)),
        ("update", text(e.update.as_deref().unwrap_or_default())),
        ("busy", Value::Bool(e.busy)),
        ("panels", slots(images, &e.panels)),
        ("controllers", text(e.controllers.join(", "))),
    ])
}

fn plugin_row(p: &PluginRowVm) -> Value {
    record([
        ("id", text(p.id.as_str())),
        ("name", text(&p.name)),
        ("kind", text(&p.kind)),
        ("version", text(&p.version)),
        ("status", text(&p.status)),
        ("enabled", Value::Bool(p.enabled)),
        ("note", text(&p.note)),
        ("build", text(&p.build)),
        ("build-label", text(&p.build_label)),
        ("has-builds", Value::Bool(p.has_builds)),
    ])
}

fn folder(f: &FolderVm) -> Value {
    record([
        ("name", text(&f.name)),
        ("title", text(&f.title)),
        ("help", text(&f.help)),
        ("text", text(&f.text)),
        ("shown", text(&f.shown)),
        ("default", text(&f.default)),
        ("requested-by", text(f.requested_by.join(", "))),
        ("write", Value::Bool(f.write)),
    ])
}

fn panel(images: &dyn ImageSource, p: &SettingsPanelVm) -> Value {
    record([("slot", slot(images, &p.slot)), ("fields", model(p.fields.iter().map(field).collect()))])
}

/// Every `Vm` property for `view`, by name.
pub fn vm_values(
    view: &AppView,
    images: &dyn ImageSource,
    cards: &CardModels,
    viewport: &Viewport,
) -> Vec<(&'static str, Value)> {
    let running = view.running.as_ref();
    vec![
        ("screen", text(view.screen.as_str())),
        ("game", text(view.game.as_deref().unwrap_or_default())),
        ("can-go-back", Value::Bool(view.can_go_back)),
        (
            "home",
            record([
                ("games", cards.list("home", images, &view.home.games, |_| true)),
                ("selected", number(view.home.selected.map(|i| i as f64).unwrap_or(-1.0))),
                ("loading", Value::Bool(view.home.loading)),
                (
                    "groups",
                    model(
                        view.home
                            .groups
                            .iter()
                            .map(|g| {
                                record([
                                    ("platform", text(&g.platform)),
                                    ("count", number(g.games.len() as f64)),
                                    ("games", group_cards(images, &g.games)),
                                    ("selected", number(g.selected.map(|i| i as f64).unwrap_or(-1.0))),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("library", library(images, cards, &view.home.library, "library")),
            ]),
        ),
        ("has-detail", Value::Bool(view.detail.is_some())),
        ("detail", detail(images, view.detail.as_ref())),
        ("catalog", library(images, cards, &view.catalog.games, "catalog")),
        ("catalog-sources", strings(&view.catalog.sources)),
        ("catalog-releases", number(view.catalog.releases as f64)),
        ("catalog-updated", text(release_text(&view.catalog.updated))),
        ("catalog-loading", Value::Bool(view.catalog.loading)),
        ("catalog-available", Value::Bool(view.catalog.available)),
        ("catalog-actions", slots(images, &view.catalog.actions)),
        ("catalog-status", slots(images, &view.catalog.status)),
        ("torrents", model(view.downloads.torrents.iter().map(torrent).collect())),
        ("seed-downloads", Value::Bool(view.downloads.seeding)),
        ("torrents-downloading", number(view.downloads.downloading as f64)),
        ("torrents-sharing", number(view.downloads.sharing as f64)),
        ("vpn", vpn(&view.settings.vpn)),
        (
            "downloads",
            model(
                view.downloads
                    .jobs
                    .iter()
                    .map(|j| {
                        record([
                            ("id", number(j.id as f64)),
                            ("plugin", text(j.plugin.as_str())),
                            ("label", text(&j.label)),
                            ("progress", number(j.progress)),
                            ("progress-text", text(progress_text(j.received, j.total))),
                            ("state", text(&j.state)),
                            ("error", text(&j.error)),
                            ("can-pause", Value::Bool(j.can_pause)),
                            ("can-resume", Value::Bool(j.can_resume)),
                            ("can-cancel", Value::Bool(j.can_cancel)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("settings", model(view.settings.panels.iter().map(|p| panel(images, p)).collect())),
        ("theme-options", model(view.settings.theme_options.iter().map(field).collect())),
        ("sounds", Value::Bool(view.settings.sounds)),
        ("fullscreen", Value::Bool(view.settings.fullscreen)),
        ("return-on-exit", Value::Bool(view.settings.return_on_exit)),
        ("auto-update", Value::Bool(view.settings.auto_update)),
        ("scale", number(view.settings.scale)),
        ("can-start-at-login", Value::Bool(view.settings.can_start_at_login)),
        ("start-at-login", Value::Bool(view.settings.start_at_login)),
        ("start-at-login-error", text(&view.settings.start_at_login_error)),
        ("hardware", hardware(&view.settings.hardware)),
        (
            "launcher-update",
            record([
                ("state", text(&view.settings.launcher_update.state)),
                ("current", text(&view.settings.launcher_update.current)),
                ("version", text(&view.settings.launcher_update.version)),
                ("text", text(&view.settings.launcher_update.text)),
                ("method", text(&view.settings.launcher_update.method)),
            ]),
        ),
        ("system", system(&view.system)),
        ("has-install-prompt", Value::Bool(view.install_prompt.is_some())),
        (
            "install-prompt",
            match &view.install_prompt {
                Some(p) => record([
                    ("plugin", text(p.plugin.as_str())),
                    ("emulator", text(&p.emulator)),
                    ("version", text(&p.version)),
                    ("game", text(&p.game)),
                    ("size", number(p.size_bytes.unwrap_or(0) as f64)),
                ]),
                None => Value::Void,
            },
        ),
        ("folders", model(view.settings.folders.iter().map(folder).collect())),
        (
            "themes",
            model(
                view.settings
                    .themes
                    .iter()
                    .map(|t| {
                        record([
                            ("id", text(t.id.as_str())),
                            ("name", text(&t.name)),
                            ("current", Value::Bool(t.current)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("emulators", model(view.emulators.emulators.iter().map(|e| emulator(images, e)).collect())),
        ("plugins", model(view.emulators.plugins.iter().map(plugin_row).collect())),
        (
            "emulator-plugins",
            // Emulators, then the services that describe their games.
            model(
                view.emulators
                    .plugins
                    .iter()
                    .filter(|p| p.kind == "emulator")
                    .chain(view.emulators.plugins.iter().filter(|p| p.kind == "service"))
                    .map(plugin_row)
                    .collect(),
            ),
        ),
        (
            "theme-plugins",
            // A theme's status says whether it is the one shown.
            model(
                view.emulators
                    .plugins
                    .iter()
                    .filter(|p| p.kind == "theme")
                    .map(|p| {
                        let current = view.settings.themes.iter().any(|t| t.current && t.id == p.id);
                        plugin_row(&PluginRowVm {
                            status: if current { "in use".into() } else { String::new() },
                            ..p.clone()
                        })
                    })
                    .collect(),
            ),
        ),
        ("has-plugin-page", Value::Bool(view.plugin_page.is_some())),
        (
            "plugin-page",
            match &view.plugin_page {
                Some(page) => record([
                    ("plugin", plugin_row(&page.plugin)),
                    ("is-emulator", Value::Bool(page.emulator.is_some())),
                    (
                        "emulator",
                        // A struct field needs a value: an empty emulator when it isn't one.
                        emulator(
                            images,
                            page.emulator.as_ref().unwrap_or(&EmulatorVm {
                                plugin: page.plugin.id.clone(),
                                name: page.plugin.name.clone(),
                                status: String::new(),
                                has_builds: false,
                                installed: Vec::new(),
                                available: Vec::new(),
                                update: None,
                                busy: false,
                                panels: Vec::new(),
                                controllers: Vec::new(),
                            }),
                        ),
                    ),
                    ("sections", slots(images, &page.sections)),
                    (
                        "actions",
                        slots(
                            images,
                            &page.sections.iter().filter(|s| s.kind == SlotKind::Action).cloned().collect::<Vec<_>>(),
                        ),
                    ),
                    ("settings", model(page.panels.iter().map(|p| panel(images, p)).collect())),
                    ("folders", model(page.folders.iter().map(folder).collect())),
                    ("current", Value::Bool(view.settings.themes.iter().any(|t| t.current && t.id == page.plugin.id))),
                ]),
                None => Value::Void,
            },
        ),
        ("has-running", Value::Bool(running.is_some())),
        (
            "running",
            match running {
                Some(r) => record([
                    ("session", number(r.session as f64)),
                    ("key", text(&r.key)),
                    ("title", text(&r.title)),
                    ("emulator", text(&r.emulator)),
                    ("page", text(&r.page)),
                    ("state", text(&r.state)),
                    ("elapsed", text(duration_text(r.elapsed_secs))),
                    ("in-front", Value::Bool(r.in_front)),
                    (
                        "keymap",
                        model(
                            r.keymap
                                .iter()
                                .map(|k| record([("label", text(&k.button)), ("value", text(&k.keys))]))
                                .collect(),
                        ),
                    ),
                ]),
                None => Value::Void,
            },
        ),
        ("notices", strings(&view.notices)),
        (
            "notice-cards",
            model(
                view.notice_cards
                    .iter()
                    .map(|n| {
                        record([
                            ("id", number(n.id as f64)),
                            ("text", text(&n.text)),
                            ("kind", text(n.kind.as_str())),
                            ("art", thumb(images, &n.art, true)),
                            ("opens", text(&n.opens)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("task", text(view.task.clone().unwrap_or_default())),
        ("notice-art", thumb(images, &view.notice_art, true)),
        ("notice-opens", text(&view.notice_opens)),
        ("has-result-prompt", Value::Bool(view.result_prompt.is_some())),
        ("result-prompt", text(view.result_prompt.clone().unwrap_or_default())),
        (
            "boot",
            record([
                ("ready", Value::Bool(view.boot.ready)),
                ("welcome", Value::Bool(view.boot.welcome)),
                ("progress", number(f64::from(view.boot.progress))),
                ("status", text(&view.boot.status)),
                (
                    "steps",
                    model(
                        view.boot
                            .steps
                            .iter()
                            .map(|s| {
                                record([
                                    ("label", text(&s.label)),
                                    ("detail", text(&s.detail)),
                                    ("state", text(&s.state)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("covers", model(view.boot.covers.iter().map(|c| thumb(images, &Some(c.clone()), true)).collect())),
            ]),
        ),
        (
            "viewport",
            record([
                ("width", number(viewport.width)),
                ("height", number(viewport.height)),
                ("scale", number(viewport.scale)),
                ("size", number(viewport.size)),
                ("aspect", number(if viewport.height > 0.0 { viewport.width / viewport.height } else { 0.0 })),
                ("breakpoint", text(&viewport.breakpoint)),
            ]),
        ),
        ("input-mode", text(&viewport.input_mode)),
    ]
}

/// The window, as the theme sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    /// How big everything is drawn: the player's "Size of everything", or NEXUS_SCALE.
    pub size: f32,
    pub breakpoint: String,
    pub input_mode: String,
}

/// A system's color for the theme: transparent when its plugin gives none, which the theme
/// draws in its neutral look.
fn system_color(system: &nexus_ui_api::GameSystemVm) -> Value {
    let [r, g, b] = system.color.unwrap_or_default();
    let alpha = if system.color.is_some() { 255 } else { 0 };
    Value::Brush(Brush::SolidColor(Color::from_argb_u8(alpha, r, g, b)))
}

pub fn token_value(value: &TokenValue) -> Value {
    match value {
        TokenValue::Color(rgba) => {
            let [r, g, b, a] = rgba.to_be_bytes();
            Value::Brush(Brush::SolidColor(Color::from_argb_u8(a, r, g, b)))
        }
        TokenValue::Length(n) | TokenValue::Duration(n) => number(*n),
        TokenValue::Int(n) => number(*n),
        TokenValue::Text(s) => text(s),
    }
}

/// Sets one global property. A property the theme doesn't have is skipped; a value of the
/// wrong type is an error.
pub fn set(instance: &ComponentInstance, global: &str, name: &str, value: Value) -> Result<(), String> {
    // `Void` means "leave the default", for optional structs.
    if matches!(value, Value::Void) {
        return Ok(());
    }
    let value = match instance.get_global_property(global, name) {
        Ok(old) => {
            let value = reconcile(&old, value);
            if value == old {
                return Ok(());
            }
            value
        }
        Err(_) => value,
    };
    match instance.set_global_property(global, name, value) {
        Ok(()) | Err(SetPropertyError::NoSuchProperty) => Ok(()),
        Err(e) => Err(format!("{global}.{name}: {e}")),
    }
}

/// `new`, with the lists of `old` kept and changed in place. A repeater makes its rows again
/// when it gets another list, so without this every view update would restart the theme's
/// entrance animations and stop its transitions.
fn reconcile(old: &Value, new: Value) -> Value {
    reconcile_at(old, new, Within::Property)
}

/// Where a value is: a property of its own, a field of a struct property, or in a list's row.
#[derive(Clone, Copy, PartialEq)]
enum Within {
    Property,
    Struct,
    Row,
}

/// The `id` or `key` field of a list's row, when it has one.
fn row_key(row: &Value) -> Option<Value> {
    let Value::Struct(fields) = row else { return None };
    fields.get_field("id").or_else(|| fields.get_field("key")).cloned()
}

/// The `id` field of a list's row, when it has one.
fn row_id(row: &Value) -> Option<Value> {
    let Value::Struct(fields) = row else { return None };
    fields.get_field("id").cloned()
}

/// Lists are kept and changed in place, with two exceptions, because a repeater doesn't
/// always see a change made in place to a list that is not a property of its own:
/// - A list inside a list's row (a card's buttons) is kept only while it is the same.
/// - A list inside a struct (a Game Hub's releases) is replaced when its rows are other rows,
///   by their `id` or `key`. A repeater whose struct was set again since it last drew skips
///   a row changed in place, so the Game Hub of one game would show another game's releases.
fn reconcile_at(old: &Value, new: Value, within: Within) -> Value {
    // The same list (a `CardModels` list): it already has its rows.
    if let (Value::Model(a), Value::Model(b)) = (old, &new)
        && a == b
    {
        return new;
    }
    match (old, new) {
        (Value::Model(old_model), Value::Model(new_model)) if within == Within::Row => {
            let same = old_model.row_count() == new_model.row_count()
                && (0..new_model.row_count()).all(|i| old_model.row_data(i) == new_model.row_data(i));
            Value::Model(if same { old_model.clone() } else { new_model })
        }
        (Value::Model(old_model), Value::Model(new_model)) => {
            let Some(rows) = old_model.as_any().downcast_ref::<VecModel<Value>>() else {
                return Value::Model(new_model);
            };
            let count = new_model.row_count();
            let other_rows = (0..count.min(rows.row_count())).any(|i| {
                rows.row_data(i).as_ref().and_then(row_key) != new_model.row_data(i).as_ref().and_then(row_key)
            });
            if within == Within::Struct && other_rows {
                return Value::Model(new_model);
            }
            // In a list of its own, a row whose `id` is gone is removed first. The rows after it
            // then keep their place in the repeater, and their state, such as a notice's timers.
            if within == Within::Property {
                let ids: Vec<Value> =
                    (0..count).filter_map(|i| new_model.row_data(i).as_ref().and_then(row_id)).collect();
                if !ids.is_empty() {
                    for i in (0..rows.row_count()).rev() {
                        if rows.row_data(i).as_ref().and_then(row_id).is_some_and(|id| !ids.contains(&id)) {
                            rows.remove(i);
                        }
                    }
                }
            }
            for i in 0..count {
                let new_row = new_model.row_data(i).unwrap_or_default();
                match rows.row_data(i) {
                    Some(old_row) => {
                        let row = reconcile_at(&old_row, new_row, Within::Row);
                        if row != old_row {
                            rows.set_row_data(i, row);
                        }
                    }
                    None => rows.push(new_row),
                }
            }
            while rows.row_count() > count {
                rows.remove(rows.row_count() - 1);
            }
            Value::Model(old_model.clone())
        }
        (Value::Struct(old_fields), Value::Struct(new_fields)) => {
            let inner = if within == Within::Row { Within::Row } else { Within::Struct };
            Value::Struct(
                new_fields
                    .iter()
                    .map(|(name, value)| {
                        let value = match old_fields.get_field(name) {
                            Some(old) => reconcile_at(old, value.clone(), inner),
                            None => value.clone(),
                        };
                        (name.to_owned(), value)
                    })
                    .collect(),
            )
        }
        (_, new) => new,
    }
}

pub fn set_options(instance: &ComponentInstance, values: &BTreeMap<String, OptionValue>) -> Result<(), String> {
    for (name, value) in values {
        let value = match value {
            OptionValue::Choice(choice) => text(choice),
            OptionValue::Color(rgba) => token_value(&TokenValue::Color(*rgba)),
        };
        set(instance, "Options", name, value)?;
    }
    Ok(())
}

pub fn set_tokens(instance: &ComponentInstance, tokens: &BTreeMap<String, TokenValue>) -> Result<(), String> {
    for (name, value) in tokens {
        set(instance, "Tokens", name, token_value(value))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releases(ids: &[&str], size: &str) -> Value {
        record([("releases", model(ids.iter().map(|id| record([("id", text(id)), ("size", text(size))])).collect()))])
    }

    fn list_of(value: &Value) -> ModelRc<Value> {
        let Value::Struct(fields) = value else { panic!("not a struct") };
        let Some(Value::Model(list)) = fields.get_field("releases") else { panic!("no releases") };
        list.clone()
    }

    #[test]
    fn a_list_in_a_struct_is_replaced_by_other_rows_and_changed_in_place_for_the_same_rows() {
        let old = releases(&["a"], "1 GB");
        let same = reconcile(&old, releases(&["a"], "2 GB"));
        assert_eq!(list_of(&same), list_of(&old), "the same rows: the list is kept");
        assert_eq!(list_of(&old).row_data(0), list_of(&releases(&["a"], "2 GB")).row_data(0), "and changed");
        let more = reconcile(&old, releases(&["a", "b"], "2 GB"));
        assert_eq!(list_of(&more), list_of(&old), "a row more: the list is kept");
        let other = reconcile(&old, releases(&["c"], "2 GB"));
        assert_ne!(list_of(&other), list_of(&old), "other rows: another list");
    }

    #[test]
    fn a_cover_is_loading_until_it_is_in_and_none_without_one_or_when_it_fails() {
        let mut images = crate::images::ImageCache::default();
        let art = |n: u32| {
            Some(ImageRef::Url { plugin: "store".parse().unwrap(), url: format!("https://art.example/{n}.png") })
        };
        let state = |images: &crate::images::ImageCache, r: &Option<ImageRef>| {
            let shown = cover(images, r, "PS5", crate::ImageSize::Thumb, true);
            cover_state(images, r, "PS5", &shown)
        };
        assert_eq!(state(&images, &None), text("none"));
        assert_eq!(state(&images, &art(1)), text("loading"));

        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(64, 96, image::Rgb([200, 60, 40]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        images.insert((art(1).unwrap(), crate::ImageSize::Thumb, crate::Crop::None), Ok(png.into_inner()));
        assert_eq!(state(&images, &art(1)), text("ready"));

        images.insert((art(2).unwrap(), crate::ImageSize::Thumb, crate::Crop::None), Err("HTTP 404".into()));
        assert_eq!(state(&images, &art(2)), text("none"), "it couldn't load");
    }

    #[test]
    fn an_availability_reads_in_words() {
        let words: Vec<&str> = ["good", "low", "none", "unknown", ""].into_iter().map(availability_text).collect();
        assert_eq!(words, ["Good", "Low", "None", "Unknown", ""]);
    }

    #[test]
    fn progress_text_reads_naturally() {
        assert_eq!(progress_text(12_500_000, Some(40_000_000)), "12.5 MB of 40.0 MB");
        assert_eq!(progress_text(0, None), "0.0 MB");
    }

    #[test]
    fn a_download_s_line_says_its_install_s_percent_while_it_installs() {
        let t = nexus_ui_api::TorrentVm {
            state: "complete".into(),
            seeding: true,
            install: "installing".into(),
            install_progress: 0.43,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "Installing · 43% · it runs in the background");
    }

    #[test]
    fn a_download_s_file_list_says_how_many_files_it_shows() {
        let files = vec!["a · 1 MB".to_owned(); 100];
        let t = nexus_ui_api::TorrentVm { files, file_count: 120, total: 1_500_000_000, ..Default::default() };
        assert_eq!(file_summary(&t), "120 files · 1.5 GB · first 100 shown");
        let one = nexus_ui_api::TorrentVm { files: vec!["a".into()], file_count: 1, total: 2_000_000, ..t };
        assert_eq!(file_summary(&one), "1 file · 2 MB");
    }

    #[test]
    fn a_download_s_line_says_its_time_left_only_while_bytes_arrive() {
        let t = nexus_ui_api::TorrentVm {
            state: "downloading".into(),
            done: 1_000_000_000,
            total: 3_000_000_000,
            download_rate: 2_000_000,
            seconds_left: 7_500,
            peers: 3,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "1.0 GB of 3.0 GB · 2.0 MB/s · 3 peers · 2 hr 5 min left");
        assert!(
            torrent_detail(&nexus_ui_api::TorrentVm { seconds_left: 30, ..t.clone() }).ends_with("under a minute left")
        );
        let stalled = nexus_ui_api::TorrentVm { download_rate: 0, seconds_left: 0, ..t };
        assert_eq!(torrent_detail(&stalled), "1.0 GB of 3.0 GB · 0.0 MB/s · 3 peers");
    }
}
