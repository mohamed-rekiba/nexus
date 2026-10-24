//! The games each console's catalog offers to download. One catalog file per console, named
//! by the console an emulator's manifest names, in lowercase: `ps5.json`, `ps4.json`,
//! `pc.json`. The core gives a console's catalog to the first emulator that plays it.
//!
//! Which copy is used:
//! - with a catalog address set, the file fetched from `<address>/<console>.json`, kept in
//!   the plugin's folder in `fetched/`; it is fetched again after [`FRESH`]. A copy that
//!   changes gets a new name (`ps5.<time>.json`), and is written before
//!   `fetched/<console>.meta.json`, which names it, as in `refreshed/` below;
//! - when that fetch fails or its answer doesn't read, the copy fetched before, if any;
//! - else, and with no address set, the copy bundled from the project's `assets/catalog/`.
//!   The build bundles only the files in [`BUNDLED_NAMES`].
//!
//! The core's call never waits for the catalog address: it gets the best copy kept now, and
//! the plugin's task (see below) fetches. With no copy fetched yet, the call gets the bundled
//! copy. When the fetch brings a new copy, or the reason it failed changes, the task tells the
//! core, and the core asks again. A fetch that failed tries again after the same back-off as a
//! failed refresh.
//!
//! With **Refresh from the forums** on, the copy above is the base of a refresh (see
//! [`refresh`]): each of the console's forums in the catalog sources gives its feed and its
//! forum list. A forum that fails leaves its topics as they were; the refresh fails only when
//! no forum answers. The result is kept in `refreshed/`, under a new name each time
//! (`ps5.<time>.json`), with `refreshed/<console>.meta.json` beside it: the time, the base
//! copy, the counts of new, removed and borrowed releases, each forum that failed, and the
//! name of the copy. The copy is written first, then the meta file, so a reader never gets one
//! refresh's copy with another's meta file. The copy before stays for a reader that read the
//! meta file before; older ones are removed. A refresh that can't write its copy or its meta
//! file failed: it changes no catalog, and it backs off like any failed refresh.
//!
//! The fetch and the refresh run in the plugin's own task, `refresh_task`, one at a time, so
//! the core's call never waits for the forums: the core gets the copy refreshed before, else the
//! base, at once. The task runs when the core asks for a catalog with no refreshed copy of its
//! base copy or one over [`FRESH`] old, and with no ask when a refreshed copy gets over
//! [`FRESH`] old, so a long session stays fresh. When the refresh ends, the core asks again and
//! gets the new copy. While it fails or waits, the copy refreshed before is used, also when it is from
//! another base copy, else the base. A refresh that failed tries again after [`RETRY`], then
//! after twice as long each time, at most [`FRESH`]; an ask doesn't make it try sooner, but
//! **Refresh now** and a new base copy do. Its reason is logged once, when it changes.
//!
//! The refresh can't build a catalog from nothing: the feed has only the last weeks' titles,
//! and the forum list has no titles. It doesn't rename titles: a release the catalog has keeps
//! its title, and a new release has the feed's title, untranslated. The catalog collector
//! (`scripts/catalog`) makes the full files.
//!
//! With "Download only through a VPN" on, the refresh asks the host's [`VpnGate`] first. While
//! the VPN isn't connected, the refresh waits: it doesn't run, each ask asks the gate again,
//! and with no ask the task asks it after the same back-off as a failed refresh.
//! The core asks again when its VPN check sees the VPN connect. When the computer has no VPN,
//! the refresh is skipped. Either way the copy above is used, and the page says why. Where this
//! computer can't bind traffic to a VPN, the refresh runs, and the page says why it ran
//! without the VPN. The gate only says whether the VPN is connected: the refresh's requests
//! don't go through the VPN.
//!
//! The plugin's page shows the blocks of the `emulator.panel` slot: "Catalog sources", and
//! "Catalog refresh" with the last refresh of each console (time, new, removed) or why it
//! didn't run. With the refresh on, the **Refresh now** action starts the refresh of each
//! console the core asked for, fresh or not.
//!
//! Each console's "Show <console> games" setting (see [`Settings::shown`]) turned off: the
//! core gets no catalog for it, and it isn't fetched or refreshed. Its installed games are the
//! emulator's, so they stay.
//!
//! The files are in the catalog collector's format: see [`format`].
//!
//! The catalog sources (see [`sources`]) name the forums each catalog comes from. Which copy
//! is used follows the same rules, with the **Sources address** setting in place of the
//! catalog address: `sources/catalog-sources.json` (a folder of its own, so no console takes
//! its place), else the copy bundled from the project's `assets/catalog-sources.json`. The
//! refresh takes only each forum's `f=` number: its addresses are fixed in [`refresh::API`]
//! and [`refresh::FEED`]. A forum whose address has no `f=` number is left out. The "Catalog
//! sources" section names the copy in use, each catalog's forums and each forum left out.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use nexus_api::schemars::{self, JsonSchema};
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");
/// The catalog files in the project's `assets/catalog/`, by file name: the ones in
/// [`BUNDLED_NAMES`].
const BUNDLED: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// How long a fetched copy is used before the address is asked again, in seconds.
pub const FRESH: u64 = 6 * 3600;

/// How long a refresh that failed or waits for the VPN waits before it tries again the first
/// time, in seconds. Each next try waits twice as long, at most [`FRESH`].
pub const RETRY: u64 = 5 * 60;

/// The Game Catalog plugin. [`ReleaseCatalog::default`] keeps a copy for [`FRESH`] and tries
/// again after [`RETRY`].
#[derive(Debug)]
pub struct ReleaseCatalog {
    /// How long a copy is used before it is fetched or refreshed again.
    fresh: Duration,
    /// How long a refresh that failed or waits waits before its first new try.
    retry: Duration,
    /// The refresh task of this activation: `shutdown` stops it.
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Default for ReleaseCatalog {
    fn default() -> Self {
        Self { fresh: Duration::from_secs(FRESH), retry: Duration::from_secs(RETRY), task: Mutex::new(None) }
    }
}

impl ReleaseCatalog {
    /// The plugin with a copy kept for `fresh`, in whole seconds, in place of [`FRESH`], for
    /// tests.
    pub fn fresh_for(fresh: Duration) -> Self {
        Self { fresh, ..Self::default() }
    }

    /// This plugin with a first new try after `retry` in place of [`RETRY`], for tests.
    pub fn retry_after(self, retry: Duration) -> Self {
        Self { retry, ..self }
    }
}

#[async_trait]
impl Plugin for ReleaseCatalog {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }

    /// Stops the refresh task, also while a reference to the catalogs outlives the plugin.
    async fn shutdown(&self) -> PluginResult<()> {
        if let Some(task) = self.task.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task.abort();
        }
        Ok(())
    }
}

#[async_trait]
impl ServicePlugin for ReleaseCatalog {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let wake = Arc::new(Notify::new());
        let catalogs = Arc::new(Catalogs {
            ctx,
            parsed: Mutex::new(HashMap::new()),
            asked: Mutex::new(BTreeSet::new()),
            played: Mutex::new(BTreeSet::new()),
            asking: Mutex::new(BTreeSet::new()),
            not_run: Mutex::new(HashMap::new()),
            forced: Mutex::new(BTreeSet::new()),
            refreshing: std::sync::atomic::AtomicBool::new(false),
            held: Mutex::new(HashMap::new()),
            fetch_held: Mutex::new(HashMap::new()),
            fetch_problem: Mutex::new(HashMap::new()),
            wake: wake.clone(),
            fresh: self.fresh.as_secs(),
            retry: self.retry,
        });
        let task = tokio::spawn(refresh_task(Arc::downgrade(&catalogs), wake));
        // A task from an activation before, if its shutdown didn't run.
        if let Some(before) = self.task.lock().unwrap_or_else(|p| p.into_inner()).replace(task) {
            before.abort();
        }
        Ok(Provided {
            releases: Some(catalogs.clone()),
            configurator: Some(catalogs.clone()),
            slots: Some(catalogs),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::service(|| Box::new(ReleaseCatalog::default())) }

/// The fetch of the catalog address and the catalog refresh, in a task of their own: one for
/// the plugin, so two never run at once, and no call of the core waits for one. It runs each
/// time it is woken, and also when a kept copy gets too old or a held try is due (see
/// [`Catalogs::next_wake`]): in a long session the catalogs stay fresh with no ask. When a
/// fetch or a refresh changes a catalog or the reason it failed, it publishes
/// [`RELEASES_CHANGED`], and the core asks for the catalogs again. It ends when the plugin
/// shuts down or its catalogs are dropped. Each pass runs [`guarded`]: a pass that panics is
/// logged, and the task waits [`RETRY`] before its next pass.
async fn refresh_task(catalogs: Weak<Catalogs>, wake: Arc<Notify>) {
    let mut wait = None;
    loop {
        match wait {
            Some(wait) => {
                // Woken or not, it looks at what is due.
                let _ = tokio::time::timeout(wait, wake.notified()).await;
            }
            None => wake.notified().await,
        }
        let Some(catalogs) = catalogs.upgrade() else { return };
        let pass = guarded({
            let catalogs = catalogs.clone();
            async move {
                // The fetch first: the refresh is of the copy it brings, and the core gets that
                // copy with no wait for the forums.
                if catalogs.fetch_due().await {
                    catalogs.changed();
                }
                if catalogs.refresh_due().await {
                    catalogs.changed();
                }
                catalogs.next_wake().await
            }
        });
        wait = match pass.await {
            Ok(wait) => wait,
            Err(e) => {
                catalogs.ctx.log().error(&format!("the catalog fetch and refresh stopped with a panic: {e}"));
                Some(catalogs.retry)
            }
        };
    }
}

/// Runs `pass` in a task of its own and waits for it: a panic in it ends only that pass, as
/// `Err` with the panic's words.
async fn guarded<T: Send + 'static>(pass: impl Future<Output = T> + Send + 'static) -> Result<T, String> {
    /// Stops the pass when the caller stops waiting: the task stopped, or shut down.
    struct StopOnDrop<T>(tokio::task::JoinHandle<T>);
    impl<T> Drop for StopOnDrop<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut pass = StopOnDrop(tokio::spawn(pass));
    match (&mut pass.0).await {
        Ok(out) => Ok(out),
        Err(e) if e.is_panic() => {
            let panic = e.into_panic();
            let words = panic
                .downcast_ref::<&str>()
                .map(|w| (*w).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned());
            Err(words.unwrap_or_else(|| "a panic with no words".to_owned()))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The catalog's settings.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub struct Settings {
    /// Where the catalog files are: `https://xyz.dev/catalog` gives
    /// `https://xyz.dev/catalog/ps5.json`. Empty for the catalog that ships with Nexus.
    #[serde(default)]
    #[schemars(title = "Catalog address")]
    pub catalog_url: String,
    /// Where the list of forums the catalogs come from is: an `https://` address of a
    /// `catalog-sources.json` file. Empty for the list that ships with Nexus.
    #[serde(default)]
    #[schemars(title = "Sources address")]
    pub sources_url: String,
    /// Bring each console's catalog up to date from its forums: new releases from each
    /// forum's feed, and torrents, sizes and peers from its forum list. With "Download only
    /// through a VPN" on, it waits for the VPN.
    #[serde(default)]
    #[schemars(title = "Refresh from the forums")]
    pub refresh_from_forums: bool,
    /// A "Hidden" chip in the Catalog's filters, once you hide a game: it shows the games you
    /// hid. Off, hidden games stay hidden, and only turning this on again brings them back.
    #[serde(default = "on")]
    #[schemars(title = "Show the Hidden filter")]
    pub show_hidden_filter: bool,
    /// Each console's "Show <console> games" toggle, by [`shown_key`]: off, the console gets no
    /// catalog, so none of its games can be downloaded. Its games already installed are the
    /// emulator's: they stay in the library and play. On when not set. [`Catalogs::schema`]
    /// names a toggle for each console an emulator turned on plays.
    #[serde(flatten)]
    pub shown: BTreeMap<String, bool>,
}

impl Settings {
    /// Whether the catalog of `console` is given: its toggle isn't off.
    fn shows(&self, console: &Console) -> bool {
        self.shown.get(&shown_key(console)) != Some(&false)
    }
}

/// The key of a console's "Show <console> games" setting: `show_ps5`.
fn shown_key(console: &Console) -> String {
    format!("show_{}", console.file_name())
}

fn on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            catalog_url: String::new(),
            sources_url: String::new(),
            refresh_from_forums: false,
            show_hidden_filter: on(),
            shown: BTreeMap::new(),
        }
    }
}

/// Where and when a kept copy came from, kept beside it in a `.meta.json` file, so the copy
/// isn't read to know that: `fetched/<console>.meta.json` for a console's catalog,
/// `sources/catalog-sources.meta.json` for the catalog sources.
#[derive(Clone, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct Kept {
    /// When it was fetched, in seconds since 1970.
    fetched: u64,
    /// Where from.
    url: String,
    /// For a console's catalog, the copy's file in `fetched/`: `<console>.<time in
    /// nanoseconds>.json`. A copy that changes is written under a new name before its meta
    /// file, so a reader of the meta file never gets another fetch's copy. Empty for the
    /// catalog sources, kept in `sources/catalog-sources.json`.
    #[serde(default)]
    body: String,
}

/// When and from which base a refreshed copy was made, what the refresh changed, and which
/// file the copy is in, kept in `refreshed/<console>.meta.json`. The copy is written before its
/// meta file, under a new name each time, so a reader of the meta file never gets another
/// refresh's copy.
#[derive(Clone, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct Refreshed {
    /// When it was refreshed, in seconds since 1970.
    refreshed: u64,
    /// The base copy it was made from: see [`Catalogs::base`].
    base: String,
    /// The copy's file in `refreshed/`: `<console>.<time in nanoseconds>.json`.
    #[serde(default)]
    body: String,
    new: usize,
    removed: usize,
    borrowed: usize,
    /// Each forum that didn't answer, with why: `wine failed: HTTP 404 from …`. Its topics
    /// stay as they were.
    #[serde(default)]
    failed: Vec<String>,
    /// Why the refresh ran without the VPN while "Download only through a VPN" is on:
    /// `Ran without the VPN: <the downloads' words>`.
    #[serde(default)]
    vpn: Option<String>,
}

/// A refresh that failed or waits for the VPN, and when it tries again: after [`RETRY`] the
/// first time, then twice as long each time, at most [`FRESH`]. A new base copy is not held.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    /// The base copy the refresh was of: see [`Catalogs::base`].
    base: String,
    /// `true` while it waits for the VPN: then each wake asks the VPN gate again, which asks
    /// no forum. A failed refresh doesn't run before `at`.
    waiting: bool,
    /// When it tries again.
    at: Instant,
    /// How long it waited this time.
    delay: Duration,
}

impl Held {
    /// The hold after a try of `base` at `now` that failed or waits, with the hold before:
    /// `first` the first time, else twice the delay before, at most `most`.
    fn after(before: Option<&Held>, base: &str, waiting: bool, first: Duration, most: Duration, now: Instant) -> Self {
        let delay = match before {
            Some(b) if b.base == base => b.delay.saturating_mul(2).min(most),
            _ => first.min(most),
        };
        Held { base: base.to_owned(), waiting, at: now + delay, delay }
    }
}

/// Now, in nanoseconds since 1970: a new name for each kept copy.
fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default()
}

/// The file `body` names in `folder` (`refreshed`, `fetched`), when it is a copy of the catalog
/// named `console`: `<console>.<digits>.json`.
fn copy_path(folder: &str, console: &str, body: &str) -> Result<RelPath, String> {
    let plain = body.strip_prefix(&format!("{console}.")).and_then(|rest| rest.strip_suffix(".json"));
    if !plain.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
        return Err(format!("the {folder} {console} catalog's meta file names no copy of it"));
    }
    RelPath::new(format!("{folder}/{body}")).map_err(|e| e.to_string())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The host of an `https://` address, lowercase. `None` for any other address.
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The info hash in a magnet link, as 40 lowercase hex digits. Takes the hex form and the
/// 32-letter base32 form.
pub fn info_hash(magnet: &str) -> Option<String> {
    let query = magnet.strip_prefix("magnet:?")?;
    let hash = query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        let value = value.strip_prefix("urn:btih:").or_else(|| value.strip_prefix("urn%3Abtih%3A"))?;
        (key == "xt").then_some(value)
    })?;
    if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(hash.to_ascii_lowercase());
    }
    if hash.len() == 32 {
        let mut bits: u64 = 0;
        let mut count = 0;
        let mut out = String::with_capacity(40);
        for c in hash.bytes() {
            let v = match c.to_ascii_uppercase() {
                c @ b'A'..=b'Z' => c - b'A',
                c @ b'2'..=b'7' => c - b'2' + 26,
                _ => return None,
            };
            bits = (bits << 5) | u64::from(v);
            count += 5;
            if count >= 8 {
                count -= 8;
                out.push_str(&format!("{:02x}", (bits >> count) & 0xff));
            }
        }
        return Some(out);
    }
    None
}

pub mod format;
pub mod refresh;
pub mod sources;

/// The catalog sources the build bundles: the project's `assets/catalog-sources.json`.
const BUNDLED_SOURCES: &[u8] = include!(concat!(env!("OUT_DIR"), "/sources.rs"));

/// The catalog files the build bundles, from the project's `assets/catalog/`: one for each
/// console an emulator plugin plays. `build.rs` reads the same list.
pub const BUNDLED_NAMES: [&str; 3] = include!("../bundled-names.rs");

/// Where the catalog sources are kept, without `.json`: in a folder of their own, so no
/// console's catalog file can take their place.
const SOURCES_KEPT: &str = "sources/catalog-sources";

/// The address of the catalog named `console` with the catalog address set, else `None`.
fn catalog_url(settings: &Settings, console: &str) -> Option<String> {
    let base = settings.catalog_url.trim().trim_end_matches('/');
    (!base.is_empty()).then(|| format!("{base}/{console}.json"))
}

/// Where the copy fetched of the catalog named `console` is kept, without `.json`.
fn fetched(console: &str) -> String {
    format!("fetched/{console}")
}

/// The catalog sources in use, with only the forums that have an `f=` number.
struct SourcesInUse {
    sources: sources::Sources,
    /// Which copy: fetched now, fetched before, or bundled.
    copy: ReleaseSource,
    /// The host of the sources address. Empty with no address set.
    host: String,
    /// Why the fetch didn't give the list, when an address is set.
    problem: Option<String>,
    /// Each forum left out because its address has no `f=` number, as `<key>: <url>`.
    unusable: Vec<String>,
}

/// Reads a catalog file. A release keeps only the pictures on the hosts in the manifest: the
/// core fetches a picture with this plugin's permissions, and refuses any other host.
fn read(bytes: &[u8]) -> Result<ReleaseList, String> {
    let mut list = format::read(bytes)?;
    let permissions = PluginManifest::from_toml(MANIFEST).map_err(|e| e.to_string())?.permissions;
    for release in &mut list.releases {
        release.image_urls.retain(|url| https_host(url).is_some_and(|host| permissions.allows_host(&host)));
    }
    Ok(list)
}

struct Catalogs {
    ctx: PluginContext,
    /// Each console's catalog as read last, by file name, with the copy it came from: a
    /// catalog file can be megabytes, and is read once per copy.
    parsed: Mutex<HashMap<String, (String, ReleaseList)>>,
    /// The consoles the core asked for: the ones **Refresh now** refreshes.
    asked: Mutex<BTreeSet<Console>>,
    /// The consoles the core asked for in its last pass over the emulators: the ones with a
    /// "Show <console> games" setting. The core asks for the Catalog's buttons after the
    /// catalogs, which ends a pass: a console whose emulator was turned off goes.
    played: Mutex<BTreeSet<Console>>,
    /// The consoles the core asked for in the pass it is in, until it ends.
    asking: Mutex<BTreeSet<Console>>,
    /// Why the last refresh of each console didn't run or failed, by file name. A refresh that
    /// works removes its entry.
    not_run: Mutex<HashMap<String, String>>,
    /// The consoles **Refresh now** asked for: the next refresh runs for them, fresh or not.
    forced: Mutex<BTreeSet<Console>>,
    /// **Refresh now** asked, and its refresh hasn't ended yet: the Catalog says "Refreshing…".
    refreshing: std::sync::atomic::AtomicBool,
    /// Each refresh that failed or waits, by file name, and when it tries again. A refresh that
    /// works removes its entry.
    held: Mutex<HashMap<String, Held>>,
    /// Each fetch of a catalog address that failed, by file name, and when it tries again. Its
    /// `base` is the address. A fetch that works removes its entry.
    fetch_held: Mutex<HashMap<String, Held>>,
    /// Why the last fetch of each console's catalog failed, by file name, with the address.
    fetch_problem: Mutex<HashMap<String, (String, String)>>,
    /// Wakes the refresh task.
    wake: Arc<Notify>,
    /// How long a copy is used before it is fetched or refreshed again, in seconds: [`FRESH`]
    /// but in tests.
    fresh: u64,
    /// The first wait of a refresh that failed or waits: [`RETRY`] but in tests.
    retry: Duration,
}

impl Drop for Catalogs {
    /// Ends the refresh task.
    fn drop(&mut self) {
        self.wake.notify_one();
    }
}

impl Catalogs {
    /// Tells the core the catalogs changed: it asks for them again.
    fn changed(&self) {
        if let Err(e) = self.ctx.events().publish(RELEASES_CHANGED, Value::Null) {
            self.ctx.log().warn(&format!("could not tell the core the catalogs changed: {e}"));
        }
    }

    async fn settings(&self) -> Settings {
        let stored = self.ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        serde_json::from_value(stored).unwrap_or_default()
    }

    /// The consoles the core asked for whose catalog `settings` shows: the ones fetched,
    /// refreshed and shown on the page.
    fn shown(&self, settings: &Settings) -> Vec<Console> {
        let asked = self.asked.lock().unwrap_or_else(|p| p.into_inner());
        asked.iter().filter(|c| settings.shows(c)).cloned().collect()
    }

    /// The file of the copy kept at `stem` (see [`fetched`] and [`SOURCES_KEPT`]): `part` is
    /// empty for the copy, `.meta` for its meta file.
    fn kept_path(stem: &str, part: &str) -> PluginResult<RelPath> {
        Ok(RelPath::new(format!("{stem}{part}.json"))?)
    }

    async fn kept(&self, stem: &str) -> Option<Kept> {
        let bytes = self.ctx.storage().read(&Self::kept_path(stem, ".meta").ok()?).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The catalog in `copy`, read from `bytes` unless it was read before.
    fn read_once(
        &self,
        console: &str,
        copy: &str,
        bytes: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Result<ReleaseList, String> {
        if let Some((read_from, list)) = self.parsed.lock().unwrap_or_else(|p| p.into_inner()).get(console)
            && read_from == copy
        {
            return Ok(list.clone());
        }
        let list = read(&bytes().ok_or("the copy is gone")?)?;
        self.parsed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(console.to_owned(), (copy.to_owned(), list.clone()));
        Ok(list)
    }

    /// Which kept copy of a console's catalog this is, for [`Catalogs::read_once`] and as a
    /// base copy: [`Catalogs::base_bytes`] reads the file it names.
    fn kept_copy(kept: &Kept) -> String {
        format!("kept:{}:{}", kept.url, kept.body)
    }

    /// What `fetched/<console>.meta.json` says, when it names a copy of the catalog named
    /// `console`.
    async fn kept_catalog(&self, console: &str) -> Option<Kept> {
        self.kept(&fetched(console)).await.filter(|k| copy_path("fetched", console, &k.body).is_ok())
    }

    async fn read_kept(&self, console: &str, kept: &Kept) -> Result<ReleaseList, String> {
        let copy = Self::kept_copy(kept);
        let cached =
            self.parsed.lock().unwrap_or_else(|p| p.into_inner()).get(console).is_some_and(|(c, _)| *c == copy);
        let bytes = if cached {
            None
        } else {
            let path = copy_path("fetched", console, &kept.body)?;
            Some(self.ctx.storage().read(&path).await.map_err(|e| e.to_string())?)
        };
        self.read_once(console, &copy, || bytes)
    }

    /// Fetches `url`, the address of the catalog named `console`, and keeps the copy when it
    /// reads: a new file, then the meta file that names it. An answer the same as the copy
    /// before keeps that copy's name, and only its time changes. `true` for a new copy. It
    /// fails when the copy or its meta file can't be written.
    async fn fetch(&self, url: &str, console: &str) -> Result<bool, String> {
        let response = self.ctx.http().send(HttpRequest::get(url)).await.map_err(|e| e.to_string())?;
        if response.status != 200 {
            return Err(format!("HTTP {} from {url}", response.status));
        }
        let body = response.body;
        let list = read(&body)?;
        let before = self.kept_catalog(console).await.filter(|k| k.url == url);
        let same = match &before {
            Some(before) => match copy_path("fetched", console, &before.body) {
                Ok(path) => self.ctx.storage().read(&path).await.is_ok_and(|b| b == body),
                Err(_) => false,
            },
            None => false,
        };
        let name = match &before {
            Some(before) if same => before.body.clone(),
            _ => format!("{console}.{}.json", nanos()),
        };
        let kept = Kept { fetched: now(), url: url.to_owned(), body: name };
        let saved = async {
            if !same {
                let path = copy_path("fetched", console, &kept.body)?;
                self.ctx.storage().write(&path, &body).await.map_err(|e| e.to_string())?;
            }
            let meta = serde_json::to_vec(&kept).unwrap_or_default();
            let path = Self::kept_path(&fetched(console), ".meta").map_err(|e| e.to_string())?;
            self.ctx.storage().write(&path, &meta).await.map_err(|e| e.to_string())
        };
        // A copy with no meta file is no change: the next read would not find it.
        saved.await.map_err(|e| format!("could not keep the {console} catalog: {e}"))?;
        if !same {
            // Always this answer's own reading: an earlier copy may have the same name.
            self.parsed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(console.to_owned(), (Self::kept_copy(&kept), list));
            let keep = [kept.body.as_str(), before.as_ref().map_or("", |b| b.body.as_str())];
            self.remove_old_copies("fetched", console, &keep).await;
        }
        Ok(!same)
    }

    /// The catalog sources, from the copy the rules in this module's notes pick.
    async fn sources(&self) -> SourcesInUse {
        let url = self.settings().await.sources_url.trim().to_owned();
        let host = https_host(&url).unwrap_or_default();
        let (sources, copy, problem) = if url.is_empty() {
            (self.bundled_sources(), ReleaseSource::Bundled, None)
        } else {
            self.address_sources(&url).await
        };
        let (sources, unusable) = sources.usable();
        for forum in &unusable {
            self.ctx.log().warn(&format!("catalog sources: skipped, no f= number: {forum}"));
        }
        SourcesInUse { sources, copy, host, problem, unusable }
    }

    fn bundled_sources(&self) -> sources::Sources {
        sources::read(BUNDLED_SOURCES).unwrap_or_else(|e| {
            // A test checks the bundled file, so this is a broken build.
            self.ctx.log().error(&format!("the bundled catalog sources don't read: {e}"));
            debug_assert!(false, "bundled catalog sources: {e}");
            sources::Sources::default()
        })
    }

    /// The catalog sources from `url`: the kept copy while it is fresh, else a new fetch, else
    /// the kept copy, else the bundled one.
    async fn address_sources(&self, url: &str) -> (sources::Sources, ReleaseSource, Option<String>) {
        let kept = self.kept(SOURCES_KEPT).await.filter(|k| k.url == url);
        let read_kept = || async {
            let path = Self::kept_path(SOURCES_KEPT, "").map_err(|e| e.to_string())?;
            sources::read(&self.ctx.storage().read(&path).await.map_err(|e| e.to_string())?)
        };
        if kept.as_ref().is_some_and(|k| now().saturating_sub(k.fetched) < self.fresh)
            && let Ok(list) = read_kept().await
        {
            return (list, ReleaseSource::Fetched, None);
        }
        let problem = match self.fetch_sources(url).await {
            Ok(list) => return (list, ReleaseSource::Fetched, None),
            Err(e) => e,
        };
        self.ctx.log().warn(&format!("{url}: {problem}"));
        if kept.is_some()
            && let Ok(list) = read_kept().await
        {
            return (list, ReleaseSource::Cached, Some(problem));
        }
        (self.bundled_sources(), ReleaseSource::Bundled, Some(problem))
    }

    /// Fetches the catalog sources from `url` and reads them. The copy is kept only when it
    /// reads.
    async fn fetch_sources(&self, url: &str) -> Result<sources::Sources, String> {
        let response = self.ctx.http().send(HttpRequest::get(url)).await.map_err(|e| e.to_string())?;
        if response.status != 200 {
            return Err(format!("HTTP {} from {url}", response.status));
        }
        let list = sources::read(&response.body)?;
        let kept = Kept { fetched: now(), url: url.to_owned(), body: String::new() };
        let saved = async {
            self.ctx.storage().write(&Self::kept_path(SOURCES_KEPT, "")?, &response.body).await?;
            let meta = serde_json::to_vec(&kept).unwrap_or_default();
            self.ctx.storage().write(&Self::kept_path(SOURCES_KEPT, ".meta")?, &meta).await
        };
        if let Err(e) = saved.await {
            self.ctx.log().warn(&format!("could not keep the catalog sources: {e}"));
        }
        Ok(list)
    }

    /// The "Catalog sources" section: the copy in use, each catalog's forums and each forum left
    /// out.
    async fn sources_section(&self) -> SlotItem {
        let used = self.sources().await;
        let row = |label: &str, value: String| SectionRow { label: label.into(), value };
        let copy = match used.copy {
            ReleaseSource::Fetched => format!("fetched from {}", used.host),
            ReleaseSource::Cached => format!("the copy fetched before from {}", used.host),
            ReleaseSource::Bundled => "the bundled list".into(),
        };
        let mut rows = vec![row("List", copy)];
        rows.extend(used.problem.map(|p| row("Fetch failed", p)));
        for catalog in used.sources.catalogs.values() {
            let keys: Vec<&str> = catalog.forums.iter().map(|f| f.key.as_str()).collect();
            rows.push(row(catalog.console.as_str(), keys.join(", ")));
        }
        rows.extend(used.unusable.into_iter().map(|f| row("Skipped", format!("No f= number: {f}"))));
        SlotItem {
            id: "sources".into(),
            order: Order::default(),
            content: SlotContent::Section { title: "Catalog sources".into(), rows },
        }
    }

    fn bundled(&self, console: &str) -> Option<ReleaseList> {
        let name = format!("{console}.json");
        let (_, bytes) = BUNDLED.iter().find(|(n, _)| *n == name)?;
        match self.read_once(console, "bundled", || Some(bytes.to_vec())) {
            Ok(list) => Some(list),
            Err(e) => {
                // A test checks every bundled file, so this is a broken build.
                debug_assert!(false, "bundled {name}: {e}");
                None
            }
        }
    }

    /// The catalog named `console` (`ps5`), from the copy the rules in this module's notes pick.
    /// It doesn't wait for a refresh: with the refresh on, it is the copy refreshed before, if
    /// any, also from another base copy. The refresh task makes one from this base copy.
    async fn list(&self, console: &str, settings: &Settings) -> Option<ReleaseList> {
        let (base, _) = self.base(console, settings).await?;
        if !settings.refresh_from_forums {
            return Some(base);
        }
        let kept = self.refreshed_meta(console).await;
        let refreshed = match &kept {
            Some(meta) => self.read_refreshed(console, meta).await.ok(),
            None => None,
        };
        let list = refreshed.unwrap_or_else(|| base.clone());
        Some(ReleaseList { source: base.source, host: base.host, problem: base.problem, ..list })
    }

    /// The catalog named `console` with no refresh, and the name of its copy: `bundled:<time>`
    /// for the bundled copy, else the name [`Catalogs::kept_copy`] gives. It doesn't fetch: with
    /// an address set, it is the copy fetched before, if any, else the bundled one. The refresh
    /// task fetches (see [`Catalogs::fetch_due`]).
    async fn base(&self, console: &str, settings: &Settings) -> Option<(ReleaseList, String)> {
        let bundled = |list: ReleaseList| {
            let copy = format!("bundled:{}", list.updated);
            (list, copy)
        };
        let Some(url) = catalog_url(settings, console) else {
            return self.bundled(console).map(bundled);
        };
        let host = https_host(&url).unwrap_or_default();
        let problem = self
            .fetch_problem
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(console)
            .filter(|(at, _)| *at == url)
            .map(|(_, why)| why.clone());
        if let Some(kept) = self.kept_catalog(console).await.filter(|k| k.url == url)
            && let Ok(list) = self.read_kept(console, &kept).await
        {
            let source = if problem.is_some() { ReleaseSource::Cached } else { ReleaseSource::Fetched };
            return Some((ReleaseList { source, host, problem, ..list }, Self::kept_copy(&kept)));
        }
        let (list, copy) = bundled(self.bundled(console)?);
        Some((ReleaseList { source: ReleaseSource::Bundled, host, problem, ..list }, copy))
    }

    /// The fetch of each catalog address the core asked for that is due: with no copy fetched
    /// from it, or one over [`FRESH`] old. A fetch that failed doesn't run again before its
    /// [`Held`] time. `true` when a copy or the reason a fetch failed changed. A reason is
    /// logged once, when it changes.
    async fn fetch_due(&self) -> bool {
        let settings = self.settings().await;
        let asked = self.shown(&settings);
        let mut changed = false;
        for console in asked {
            let name = console.file_name();
            let Some(url) = catalog_url(&settings, &name) else { continue };
            let kept = self.kept_catalog(&name).await.filter(|k| k.url == url);
            if kept.is_some_and(|k| now().saturating_sub(k.fetched) < self.fresh) {
                continue;
            }
            let before = self.fetch_held.lock().unwrap_or_else(|p| p.into_inner()).get(&name).cloned();
            if before.as_ref().is_some_and(|h| h.base == url && Instant::now() < h.at) {
                continue;
            }
            let fetched = self.fetch(&url, &name).await;
            let mut problems = self.fetch_problem.lock().unwrap_or_else(|p| p.into_inner());
            let mut holds = self.fetch_held.lock().unwrap_or_else(|p| p.into_inner());
            match fetched {
                Ok(new) => {
                    holds.remove(&name);
                    let solved = problems.remove(&name).is_some();
                    changed |= new || solved;
                }
                Err(why) => {
                    let most = Duration::from_secs(self.fresh);
                    holds.insert(
                        name.clone(),
                        Held::after(before.as_ref(), &url, false, self.retry, most, Instant::now()),
                    );
                    let new = (url.clone(), why);
                    if problems.get(&name) != Some(&new) {
                        self.ctx.log().warn(&format!("{url}: {}", new.1));
                        problems.insert(name, new);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn refreshed_path(console: &str, part: &str) -> PluginResult<RelPath> {
        Ok(RelPath::new(format!("refreshed/{console}{part}.json"))?)
    }

    /// What `refreshed/<console>.meta.json` says, from any base copy.
    async fn refreshed_meta(&self, console: &str) -> Option<Refreshed> {
        let bytes = self.ctx.storage().read(&Self::refreshed_path(console, ".meta").ok()?).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// How long until the task has work with no ask, at least a second from now: for each
    /// console the core asked for, when its fetched copy gets too old or a fetch that failed
    /// tries again, and with the refresh on, when its refreshed copy gets too old or a refresh
    /// that failed or waits tries again (see [`Held`]). With the refresh on, a console with no
    /// refreshed copy tries again after the fresh time. `None` with no such work: then only an
    /// ask wakes the task.
    async fn next_wake(&self) -> Option<Duration> {
        let settings = self.settings().await;
        let asked = self.shown(&settings);
        let fresh = Duration::from_secs(self.fresh);
        let mut next = settings.refresh_from_forums.then_some(fresh);
        let mut soonest = |wait: Duration| next = Some(next.map_or(wait, |n| n.min(wait)));
        let until = |at: Instant| at.saturating_duration_since(Instant::now());
        let after = |time: u64| Duration::from_secs((time + self.fresh).saturating_sub(now()));
        for console in asked {
            let name = console.file_name();
            if let Some(url) = catalog_url(&settings, &name) {
                let held = self.fetch_held.lock().unwrap_or_else(|p| p.into_inner()).get(&name).cloned();
                match held.filter(|h| h.base == url) {
                    Some(held) => soonest(until(held.at)),
                    None => {
                        if let Some(kept) = self.kept_catalog(&name).await.filter(|k| k.url == url) {
                            soonest(after(kept.fetched));
                        }
                    }
                }
            }
            if !settings.refresh_from_forums {
                continue;
            }
            let held = self.held.lock().unwrap_or_else(|p| p.into_inner()).get(&name).cloned();
            match held {
                Some(held) => soonest(until(held.at)),
                None => soonest(after(self.refreshed_meta(&name).await.map_or(now(), |m| m.refreshed))),
            }
        }
        next.map(|n| n.max(Duration::from_secs(1)))
    }

    /// The refresh of each console the core asked for that is due: with no refreshed copy of
    /// its base copy, with one over [`FRESH`] old, or after **Refresh now**. A refresh of the
    /// same base copy that failed doesn't run again before its [`Held`] time, also on an ask;
    /// one that waits for the VPN asks the VPN gate again. **Refresh now** runs either.
    /// `true` when a catalog or the reason a refresh didn't run changed. A reason is logged
    /// once, when it changes.
    async fn refresh_due(&self) -> bool {
        let settings = self.settings().await;
        if !settings.refresh_from_forums {
            return false;
        }
        let forced = std::mem::take(&mut *self.forced.lock().unwrap_or_else(|p| p.into_inner()));
        let asked = self.shown(&settings);
        // A refresh that Refresh now asked for always tells the core when it ends, so the
        // Catalog stops saying "Refreshing…", also when it fails for the same reason again.
        let mut changed = !forced.is_empty();
        for console in asked {
            let name = console.file_name();
            let Some((_, copy)) = self.base(&name, &settings).await else { continue };
            let kept = self.refreshed_meta(&name).await.filter(|m| m.base == copy && !m.body.is_empty());
            let fresh = kept.is_some_and(|m| now().saturating_sub(m.refreshed) < self.fresh);
            let force = forced.contains(&console);
            if fresh && !force {
                continue;
            }
            let before = self.held.lock().unwrap_or_else(|p| p.into_inner()).get(&name).cloned();
            let held = before.as_ref().filter(|h| h.base == copy && Instant::now() < h.at);
            if held.is_some_and(|h| !h.waiting) && !force {
                continue;
            }
            let why = match self.vpn().await {
                Err(why) => Some((why, true)),
                Ok(vpn) => match self.refresh(&name, &copy, vpn).await {
                    Ok(_) => None,
                    Err(e) => Some((format!("Failed: {e}"), false)),
                },
            };
            {
                let mut holds = self.held.lock().unwrap_or_else(|p| p.into_inner());
                match &why {
                    None => {
                        holds.remove(&name);
                    }
                    // Still waiting before its time: the time stays.
                    Some((_, true)) if held.is_some_and(|h| h.waiting) && !force => {}
                    Some((_, waiting)) => {
                        let most = Duration::from_secs(self.fresh);
                        let next = Held::after(before.as_ref(), &copy, *waiting, self.retry, most, Instant::now());
                        holds.insert(name.clone(), next);
                    }
                }
            }
            let mut not_run = self.not_run.lock().unwrap_or_else(|p| p.into_inner());
            let same = match why {
                Some((why, _)) => {
                    let same = not_run.get(&name) == Some(&why);
                    if !same {
                        self.ctx.log().warn(&format!("the {name} catalog is not refreshed: {why}"));
                    }
                    not_run.insert(name, why);
                    same
                }
                None => {
                    // A refresh that ran changed the catalog.
                    not_run.remove(&name);
                    false
                }
            };
            changed |= !same;
        }
        // No other Refresh now came while this one ran.
        if !forced.is_empty() && self.forced.lock().unwrap_or_else(|p| p.into_inner()).is_empty() {
            self.refreshing.store(false, std::sync::atomic::Ordering::SeqCst);
        }
        changed
    }

    /// `Ok` when the refresh may run now under "Download only through a VPN", with why it runs
    /// without the VPN while the setting is on, if it does. Else why not. A host with no
    /// [`VpnGate`] has no such setting.
    async fn vpn(&self) -> Result<Option<String>, String> {
        let Some(gate) = self.ctx.services().get::<VpnGate>() else { return Ok(None) };
        match gate.check().await {
            Ok(VpnPass::Open) => Ok(None),
            Ok(VpnPass::Unbound { reason }) => Ok(Some(format!("Ran without the VPN: {reason}"))),
            Err(VpnBlock::Waiting { reason }) => Err(format!("Waiting for the VPN: {reason}")),
            Err(VpnBlock::NoVpn { reason }) => Err(format!("Skipped: {reason}, and Download only through a VPN is on")),
        }
    }

    /// **Refresh now**: the refresh task refreshes each console the core asked for, fresh or
    /// not. It doesn't wait for the refresh.
    async fn refresh_now(&self) -> PluginResult<()> {
        let settings = self.settings().await;
        if !settings.refresh_from_forums {
            return Err(PluginError::Precondition("Turn on Refresh from the forums first".into()));
        }
        let asked = self.shown(&settings);
        if !asked.is_empty() {
            self.refreshing.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.forced.lock().unwrap_or_else(|p| p.into_inner()).extend(asked);
        self.wake.notify_one();
        Ok(())
    }

    /// The Catalog screen's refresh button and, with the refresh on, what it says of the
    /// refresh: "Refreshing…" after **Refresh now** until that refresh ends, else why a
    /// console's refresh didn't run, else when the newest refresh ran. With the refresh off,
    /// the button says to turn it on, and opens Settings.
    async fn catalog_items(&self) -> Vec<SlotItem> {
        let button = |label: &str, action: &str| SlotItem {
            id: "catalog-refresh".into(),
            order: Order::default(),
            content: SlotContent::Action {
                label: label.into(),
                action: action.into(),
                args: Value::Null,
                style: ActionStyle::Secondary,
                icon: None,
            },
        };
        let settings = self.settings().await;
        if !settings.refresh_from_forums {
            return vec![button("Turn on Refresh from the forums in Settings", HOST_OPEN_SETTINGS)];
        }
        let asked = self.shown(&settings);
        let why = {
            let not_run = self.not_run.lock().unwrap_or_else(|p| p.into_inner());
            asked.iter().find_map(|c| not_run.get(&c.file_name()).map(|why| format!("{}: {why}", c.as_str())))
        };
        let mut newest = None;
        for console in &asked {
            if let Some(meta) = self.refreshed_meta(&console.file_name()).await {
                newest = newest.max(Some(meta.refreshed));
            }
        }
        let (label, tone) = if self.refreshing.load(std::sync::atomic::Ordering::SeqCst) {
            ("Refreshing…".to_owned(), Tone::Info)
        } else if let Some(why) = why {
            (why, Tone::Warning)
        } else if let Some(time) = newest {
            (format!("Refreshed {}", refresh::time_words(i64::try_from(time).unwrap_or(i64::MAX))), Tone::Neutral)
        } else {
            ("Not refreshed yet".to_owned(), Tone::Neutral)
        };
        let state = SlotItem {
            id: "catalog-refresh-state".into(),
            order: Order::default(),
            content: SlotContent::Badge { label, short: String::new(), tone, icon: None },
        };
        vec![button("Refresh", "refresh-now"), state]
    }

    /// The "Catalog refresh" section: for each console the core asked for, the last refresh
    /// (time, new, removed) or why it didn't run. One row "Off" with the setting off.
    async fn refresh_section(&self, on: bool) -> SlotItem {
        let row = |label: &str, value: String| SectionRow { label: label.into(), value };
        let mut rows = Vec::new();
        if !on {
            rows.push(row("Refresh from the forums", "Off".into()));
        }
        let asked = self.shown(&self.settings().await);
        for console in asked.iter().filter(|_| on) {
            let name = console.file_name();
            let last = match self.refreshed_meta(&name).await {
                Some(m) => {
                    let time = refresh::time_words(i64::try_from(m.refreshed).unwrap_or(i64::MAX));
                    let mut last = format!("Refreshed {time}: {} new, {} removed", m.new, m.removed);
                    for note in m.failed.iter().chain(&m.vpn) {
                        last.push_str(&format!(". {note}"));
                    }
                    last
                }
                None => "Not refreshed yet".into(),
            };
            let why = self.not_run.lock().unwrap_or_else(|p| p.into_inner()).get(&name).cloned();
            let value = match why {
                Some(why) => format!("{why}. {last}"),
                None => last,
            };
            rows.push(row(console.as_str(), value));
        }
        SlotItem {
            id: "refresh".into(),
            order: Order::default().after("sources"),
            content: SlotContent::Section { title: "Catalog refresh".into(), rows },
        }
    }

    /// Which refreshed copy this is, for [`Catalogs::read_once`].
    fn refreshed_copy(meta: &Refreshed) -> String {
        format!("refreshed:{}", meta.body)
    }

    /// The file of the copy `meta` names, when it is a copy of the catalog named `console`.
    fn body_path(console: &str, meta: &Refreshed) -> Result<RelPath, String> {
        copy_path("refreshed", console, &meta.body)
    }

    async fn read_refreshed(&self, console: &str, meta: &Refreshed) -> Result<ReleaseList, String> {
        // Not under `console`: the base copy is kept there.
        let slot = format!("{console}.refreshed");
        let copy = Self::refreshed_copy(meta);
        let cached = self.parsed.lock().unwrap_or_else(|p| p.into_inner()).get(&slot).is_some_and(|(c, _)| *c == copy);
        let bytes = if cached {
            None
        } else {
            let path = Self::body_path(console, meta)?;
            Some(self.ctx.storage().read(&path).await.map_err(|e| e.to_string())?)
        };
        self.read_once(&slot, &copy, || bytes)
    }

    /// The bytes of the base copy `copy` of the catalog named `console`.
    async fn base_bytes(&self, console: &str, copy: &str) -> Result<Vec<u8>, String> {
        if copy.starts_with("bundled:") {
            let name = format!("{console}.json");
            let (_, bytes) = BUNDLED.iter().find(|(n, _)| *n == name).ok_or("no bundled copy")?;
            return Ok(bytes.to_vec());
        }
        // The file the meta file named when the base copy was read: a later fetch writes a new one.
        let body = copy.rsplit_once(':').map_or("", |(_, body)| body);
        let path = copy_path("fetched", console, body)?;
        self.ctx.storage().read(&path).await.map_err(|e| e.to_string())
    }

    /// Fetches `url`, as bytes.
    async fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self.ctx.http().send(HttpRequest::get(url)).await.map_err(|e| e.to_string())?;
        if response.status != 200 {
            return Err(format!("HTTP {} from {url}", response.status));
        }
        Ok(response.body)
    }

    /// Removes the copies of the catalog named `console` in `folder` (`refreshed`, `fetched`)
    /// other than the ones in `keep`: the one the meta file names, and the one it named before,
    /// which a reader that read the meta file before may still read.
    async fn remove_old_copies(&self, folder: &str, console: &str, keep: &[&str]) {
        let Ok(path) = RelPath::new(folder) else { return };
        let names = self.ctx.storage().list(&path).await.unwrap_or_default();
        let meta_file = format!("{console}.meta.json");
        for name in names {
            let copy = name.starts_with(&format!("{console}.")) && name.ends_with(".json") && name != meta_file;
            if !copy || keep.contains(&name.as_str()) {
                continue;
            }
            let removed = match RelPath::new(format!("{folder}/{name}")) {
                Ok(path) => self.ctx.storage().remove(&path).await,
                Err(e) => Err(e.into()),
            };
            if let Err(e) = removed {
                self.ctx.log().warn(&format!("could not remove the old {folder} copy {name}: {e}"));
            }
        }
    }

    /// The feed and the forum list of `forum`, a forum of `catalog`, from the fixed addresses
    /// with its `f=` number. The forum's own address is never fetched.
    async fn forum_answers(
        &self,
        catalog: &sources::SourceCatalog,
        forum: &sources::Forum,
    ) -> Result<(refresh::ForumRef, Vec<refresh::Entry>, refresh::ForumList), String> {
        let number = refresh::forum_number(&forum.url).ok_or_else(|| format!("{}: no f= number", forum.url))?;
        let feed_url = refresh::feed_url(&number);
        let feed = self.get(&feed_url).await?;
        let feed = refresh::parse_feed(&String::from_utf8_lossy(&feed)).map_err(|e| format!("{feed_url}: {e}"))?;
        let list_url = refresh::list_url(&number);
        let list = refresh::parse_forum_list(&self.get(&list_url).await?).map_err(|e| format!("{list_url}: {e}"))?;
        let about = refresh::ForumRef {
            number,
            url: forum.url.clone(),
            console: catalog.console.clone(),
            system: forum.system.clone(),
            wine: forum.wine,
        };
        Ok((about, feed, list))
    }

    /// Refreshes the base copy `copy` of the catalog named `console` from its forums, and keeps
    /// the result when it reads. A forum that doesn't answer leaves its topics as they are, and
    /// the meta file names it. It fails when no forum answers, and when the copy or its meta
    /// file can't be written.
    async fn refresh(&self, console: &str, copy: &str, vpn: Option<String>) -> Result<ReleaseList, String> {
        let used = self.sources().await;
        let catalog = used.sources.catalogs.get(console).ok_or("the catalog sources name no forum for it")?;
        let mut answers = Vec::new();
        let mut failed = Vec::new();
        for forum in &catalog.forums {
            match self.forum_answers(catalog, forum).await {
                Ok(answer) => answers.push(answer),
                Err(e) => failed.push((forum.key.clone(), e)),
            }
        }
        if answers.is_empty() {
            let why: Vec<String> = failed.into_iter().map(|(_, e)| e).collect();
            return Err(why.join("; "));
        }
        for (key, e) in &failed {
            self.ctx.log().warn(&format!("the {console} catalog's forum {key} is not refreshed: {e}"));
        }
        let mut file: Value = serde_json::from_slice(&self.base_bytes(console, copy).await?)
            .map_err(|e| format!("not a catalog file: {e}"))?;
        let time = now();
        let counts =
            refresh::refresh(&mut file, &answers, catalog.forums.len(), i64::try_from(time).unwrap_or(i64::MAX))?;
        let body = serde_json::to_vec(&file).map_err(|e| e.to_string())?;
        let before = self.refreshed_meta(console).await;
        let meta = Refreshed {
            refreshed: time,
            base: copy.to_owned(),
            body: format!("{console}.{}.json", nanos()),
            new: counts.new,
            removed: counts.removed,
            borrowed: counts.borrowed,
            failed: failed.into_iter().map(|(key, e)| format!("{key} failed: {e}")).collect(),
            vpn,
        };
        let list = read(&body)?;
        let saved = async {
            let path = Self::body_path(console, &meta).map_err(PluginError::InvalidInput)?;
            self.ctx.storage().write(&path, &body).await?;
            let bytes = serde_json::to_vec(&meta).unwrap_or_default();
            self.ctx.storage().write(&Self::refreshed_path(console, ".meta")?, &bytes).await
        };
        // A copy with no meta file is no change: the next read would not find it.
        saved.await.map_err(|e| format!("could not keep the refreshed {console} catalog: {e}"))?;
        // Always this refresh's own reading: an earlier copy may have the same name.
        self.parsed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(format!("{console}.refreshed"), (Self::refreshed_copy(&meta), list.clone()));
        let keep = [meta.body.as_str(), before.as_ref().map_or("", |b| b.body.as_str())];
        self.remove_old_copies("refreshed", console, &keep).await;
        self.ctx.log().info(&format!(
            "refreshed the {console} catalog: {} new, {} removed, {} borrowed",
            counts.new, counts.removed, counts.borrowed
        ));
        Ok(list)
    }
}

#[async_trait]
impl Releases for Catalogs {
    async fn releases(&self, console: &Console) -> PluginResult<Option<ReleaseList>> {
        self.asked.lock().unwrap_or_else(|p| p.into_inner()).insert(console.clone());
        self.asking.lock().unwrap_or_else(|p| p.into_inner()).insert(console.clone());
        let settings = self.settings().await;
        if !settings.shows(console) {
            return Ok(None);
        }
        let list = self.list(&console.file_name(), &settings).await;
        // The task fetches the catalog address and refreshes, if either is due.
        self.wake.notify_one();
        Ok(list)
    }
}

#[async_trait]
impl Configurator for Catalogs {
    /// The settings, with a "Show <console> games" toggle for each console an emulator turned
    /// on plays (see [`Catalogs::played`]).
    async fn schema(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        let mut schema = serde_json::to_value(schemars::schema_for!(Settings))?;
        let played = self.played.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for console in played {
                let name = console.as_str();
                properties.insert(
                    shown_key(&console),
                    serde_json::json!({
                        "type": "boolean",
                        "default": true,
                        "title": format!("Show {name} games"),
                        "description": format!(
                            "{name} games in the Catalog, to download. Off, none can be downloaded; the {name} games \
                             you installed stay in your library and play."
                        ),
                    }),
                );
            }
        }
        Ok(schema)
    }

    async fn defaults(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        Ok(serde_json::to_value(Settings::default())?)
    }

    async fn validate(&self, _scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        let settings: Settings =
            serde_json::from_value(value.clone()).map_err(|e| PluginError::InvalidInput(e.to_string()))?;
        if let Some(key) = settings.shown.keys().find(|k| !k.starts_with("show_")) {
            return Err(PluginError::InvalidInput(format!("no setting {key}")));
        }
        for (what, url) in [("catalog address", &settings.catalog_url), ("sources address", &settings.sources_url)] {
            let url = url.trim();
            if url.is_empty() {
                continue;
            }
            let host = https_host(url)
                .ok_or_else(|| PluginError::InvalidInput(format!("the {what} must start with https://")))?;
            let allowed =
                PluginManifest::from_toml(MANIFEST).map(|m| m.permissions.allows_host(&host)).unwrap_or(false);
            if !allowed {
                return Err(PluginError::InvalidInput(format!(
                    "this plugin may fetch only from the hosts in its manifest, and {host} isn't one"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Slots for Catalogs {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        Ok(match request.slot {
            SlotId::SettingsPanel => vec![SlotItem {
                id: "settings".into(),
                order: Order::default(),
                content: SlotContent::SettingsPanel { title: "Game Catalog".into(), scope: ConfigScope::Emulator },
            }],
            SlotId::CatalogAction => {
                // The core's pass over the consoles ended.
                let pass = std::mem::take(&mut *self.asking.lock().unwrap_or_else(|p| p.into_inner()));
                *self.played.lock().unwrap_or_else(|p| p.into_inner()) = pass;
                self.catalog_items().await
            }
            // The plugin's page draws these.
            SlotId::EmulatorPanel => {
                let on = self.settings().await.refresh_from_forums;
                let mut items = vec![self.sources_section().await, self.refresh_section(on).await];
                if on {
                    items.push(SlotItem {
                        id: "refresh-now".into(),
                        order: Order::default(),
                        content: SlotContent::Action {
                            label: "Refresh now".into(),
                            action: "refresh-now".into(),
                            args: Value::Null,
                            style: ActionStyle::Secondary,
                            icon: None,
                        },
                    });
                }
                items
            }
            _ => Vec::new(),
        })
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        match action {
            "refresh-now" => self.refresh_now().await,
            _ => Err(PluginError::InvalidInput(format!("no action {action}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    #[test]
    fn every_bundled_catalog_reads() {
        let names: Vec<&str> = BUNDLED.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, BUNDLED_NAMES, "one bundled file for each console");
        for (name, bytes) in BUNDLED {
            read(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn the_bundled_sources_read() {
        let sources = sources::read(BUNDLED_SOURCES).unwrap();
        let (_, unusable) = sources.clone().usable();
        assert!(unusable.is_empty(), "forums with no f= number: {unusable:?}");
        let names: Vec<&str> = sources.catalogs.keys().map(String::as_str).collect();
        assert_eq!(names, ["pc", "ps4", "ps5"]);
    }

    #[test]
    fn the_refresh_s_hosts_are_in_the_manifest() {
        let manifest = PluginManifest::from_toml(MANIFEST).unwrap();
        for url in [refresh::API, refresh::FEED] {
            let host = https_host(url).unwrap_or_else(|| panic!("{url} is not an https:// address"));
            assert!(manifest.permissions.allows_host(&host), "{host} is not in the manifest's network list");
        }
    }

    #[test]
    fn a_topic_keeps_only_the_pictures_on_the_hosts_in_the_manifest() {
        let file = serde_json::json!({ "source": "test", "complete": true, "translation": { "language": "en" },
            "collected_at": "2026-10-01T00:00:00Z", "topic_count": 1, "topics": [{ "id": "1", "title": "[PS5] Astro",
              "magnet": "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "game_info": { "image_urls": ["https://ir.ozone.ru/cover.jpg", "https://i128.fastpic.org/big/a.jpg",
                "https://i1.imageban.ru/out/b.png"] } }] });
        let list = read(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(
            list.releases[0].image_urls,
            ["https://i128.fastpic.org/big/a.jpg", "https://i1.imageban.ru/out/b.png"],
            "a host the plugin may not fetch from gives no picture"
        );
    }

    #[test]
    fn every_sample_catalog_reads() {
        let samples = [
            ("ps5", include_bytes!("../samples/ps5.json").as_slice()),
            ("ps4", include_bytes!("../samples/ps4.json").as_slice()),
            ("pc", include_bytes!("../samples/pc.json").as_slice()),
        ];
        for (console, bytes) in samples {
            let list = read(bytes).unwrap_or_else(|e| panic!("{console}: {e}"));
            assert!(!list.releases.is_empty(), "{console}");
        }
    }

    #[test]
    fn a_release_is_named_by_its_info_hash_in_either_form() {
        assert_eq!(info_hash(&format!("magnet:?xt=urn:btih:{}&dn=x", HASH.to_uppercase())).as_deref(), Some(HASH));
        // The same hash in base32.
        assert_eq!(info_hash("magnet:?dn=x&xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK").as_deref(), Some(HASH));
        assert_eq!(info_hash("https://xyz.dev/a.torrent"), None);
        assert_eq!(info_hash("magnet:?xt=urn:btih:1234"), None);
    }

    #[test]
    fn a_meta_file_names_only_a_copy_of_its_own_catalog() {
        let meta = |body: &str| Refreshed {
            refreshed: 0,
            base: String::new(),
            body: body.into(),
            new: 0,
            removed: 0,
            borrowed: 0,
            failed: Vec::new(),
            vpn: None,
        };
        let path = Catalogs::body_path("ps5", &meta("ps5.1791400000000000000.json")).unwrap();
        assert_eq!(path.as_str(), "refreshed/ps5.1791400000000000000.json");
        for body in ["", "ps4.1.json", "ps5..json", "ps5.meta.json", "ps5.1/../../x.json"] {
            assert!(Catalogs::body_path("ps5", &meta(body)).is_err(), "{body}");
        }
    }

    #[test]
    fn a_held_refresh_waits_twice_as_long_each_try_at_most_the_fresh_time() {
        let (first, most, now) = (Duration::from_secs(300), Duration::from_secs(1000), Instant::now());
        let one = Held::after(None, "a", false, first, most, now);
        assert_eq!((one.delay, one.at), (first, now + first));
        let two = Held::after(Some(&one), "a", false, first, most, now);
        assert_eq!(two.delay, Duration::from_secs(600));
        let three = Held::after(Some(&two), "a", true, first, most, now);
        assert_eq!((three.delay, three.waiting), (most, true), "at most the fresh time");
        let other = Held::after(Some(&three), "b", false, first, most, now);
        assert_eq!(other.delay, first, "a new base copy starts again");
        assert_eq!(Held::after(None, "a", false, first, Duration::from_secs(1), now).delay, Duration::from_secs(1));
    }

    /// A runtime of one thread for a test.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap()
    }

    #[test]
    fn a_pass_that_panics_ends_with_its_words_and_not_the_task() {
        let rt = runtime();
        assert_eq!(rt.block_on(guarded(async { 1 })), Ok(1));
        let panicked = rt.block_on(guarded(async { panic!("the forum list is gone") }));
        assert_eq!(panicked, Err::<(), _>("the forum list is gone".to_owned()));
    }

    #[test]
    fn a_pass_ends_when_its_guard_is_dropped() {
        let rt = runtime();
        rt.block_on(async {
            let (alive, gone) = tokio::sync::oneshot::channel::<()>();
            let pass = tokio::spawn(guarded(async move {
                let _alive = alive;
                std::future::pending::<()>().await;
            }));
            tokio::task::yield_now().await;
            pass.abort();
            let ended = tokio::time::timeout(Duration::from_secs(1), gone).await;
            assert!(matches!(ended, Ok(Err(_))), "the pass ended, and its sender with it");
        });
    }

    #[test]
    fn only_an_https_address_has_a_host() {
        assert_eq!(https_host("https://XYZ.dev:8443/catalog").as_deref(), Some("xyz.dev"));
        assert_eq!(https_host("http://xyz.dev/catalog"), None);
    }
}
