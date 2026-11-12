//! The Switch firmware in Lichii's data folder, from the releases of THZoria/NX_Firmware,
//! with the keys of the same version from files.prodkeys.net.
//!
//! - The firmware is the firmware release the player set, not the newest one,
//!   and the keys are of the same version. Firmware newer than its keys can't be read: Lichii
//!   loops on "keys for gen … required".
//!
//! - The player turns on Automatic updates, or presses Update now, in the Firmware block on
//!   Lichii's panel. The block is the only place that shows or sets the firmware settings: the
//!   release and Automatic updates, in the plugin's storage ([`SETTINGS`]). With Automatic
//!   updates on, a check starts when the core reads the panel (at the start and at each update
//!   check), at most once every [`RELEASE_TTL`]. A failed update isn't tried again on
//!   its own until Nexus starts again.
//! - A release is one zip of `.nca` files, stored without compression. The plugin downloads it
//!   in pieces into Lichii's data folder, checks its SHA-256 against GitHub's, and unpacks the files
//!   into a folder beside the firmware. Two renames put it in place of the old firmware:
//!   [`CONTENTS`], where Lichii reads it at its next start.
//! - Anything that fails leaves the old firmware as it was. A run that stopped between the two
//!   renames is put back first, at the next update.
//! - The version Nexus installed last is kept in the plugin's storage ([`MARKER`]).

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use emulator_kit::console::Panel;
use emulator_kit::host::Host;
use emulator_kit::{cache, github};
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;
use sha2::{Digest, Sha256};
// Tokio's clock, not the system's: the host's test clock moves it, so the re-check can be tested.
use tokio::time::Instant;

use crate::DATA;

/// The actions of the Firmware block's rows: Update now, and the automatic updates' switch.
const UPDATE_NOW: &str = "update-firmware";
const TOGGLE_AUTO: &str = "toggle-firmware-updates";
/// The action of the Firmware release row: the typed line is the version to install.
const SET_RELEASE: &str = "set-firmware-release";

/// The firmware settings, in the plugin's storage. A backup holds the file.
pub const SETTINGS: &str = "firmware-settings.json";
/// The release installed until the player types another.
const DEFAULT_RELEASE: &str = "22.5.0";

/// What the player set in the Firmware block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct FirmwareSettings {
    /// Automatic updates: a read of the panel starts a check, at most every [`RELEASE_TTL`].
    automatic: bool,
    /// The Switch firmware release to install, like 22.5.0, with the keys of the same version:
    /// firmware newer than its keys can't be read.
    release: String,
}

impl Default for FirmwareSettings {
    fn default() -> Self {
        Self { automatic: false, release: DEFAULT_RELEASE.into() }
    }
}

/// The Firmware block on Lichii's panel, through the console emulator.
#[async_trait]
impl Panel for Firmware {
    /// With Automatic updates on, a read of the block starts a check, at most every
    /// [`RELEASE_TTL`]: the core reads the panel at the start and at each update check.
    async fn items(&self) -> Vec<SlotItem> {
        let read = self.read_settings().await;
        let settings = read.clone().unwrap_or_default();
        if settings.automatic {
            self.start_auto(settings.release.clone()).await;
        }
        self.panel(settings.automatic, &settings.release, read.err()).await
    }

    /// The core reads the panel again after an action: with Automatic updates turned on, a
    /// check starts then.
    async fn run_action(&self, action: &str, args: &Value) -> Option<PluginResult<()>> {
        Some(match action {
            UPDATE_NOW => {
                self.start(true, self.settings().await.release);
                Ok(())
            }
            TOGGLE_AUTO => {
                let mut settings = self.settings().await;
                settings.automatic = !settings.automatic;
                self.save(&settings).await
            }
            SET_RELEASE => self.set_release(args).await,
            _ => return None,
        })
    }
}

/// A file or folder in Lichii's data folder that an update replaces in two renames: `live` moves
/// to `old`, then `staged` into its place. Lichii never reads `staged` or `old`.
struct Swap {
    live: HostPath,
    staged: HostPath,
    old: HostPath,
}

impl Swap {
    /// Puts right what a run that stopped between the renames left: `old` moves back in place of
    /// a missing `live`, and is removed when `live` is there.
    async fn recover(&self, folders: &dyn Folders) -> PluginResult<()> {
        if folders.exists(&self.old).await? {
            if folders.exists(&self.live).await? {
                folders.remove(&self.old).await?;
            } else {
                folders.rename(&self.old, &self.live).await?;
            }
        }
        Ok(())
    }

    /// Moves `live` to `old`, then `staged` into its place, and says whether there was a `live`.
    /// When the second rename fails, `old` moves back; a failure to move it back is logged, and
    /// the next [`Swap::recover`] moves it back.
    async fn swap(&self, folders: &dyn Folders, log: &Logger) -> PluginResult<bool> {
        let had_live = folders.exists(&self.live).await?;
        if had_live {
            folders.rename(&self.live, &self.old).await?;
        }
        if let Err(e) = folders.rename(&self.staged, &self.live).await {
            if had_live && let Err(back) = folders.rename(&self.old, &self.live).await {
                log.warn(&format!("could not put {} back: {back}", self.live.path));
            }
            return Err(e);
        }
        Ok(had_live)
    }

    /// Undoes a swap that worked: the new `live` is removed, and `old` moves back when the swap
    /// moved one there (`had_live`).
    async fn undo(&self, folders: &dyn Folders, had_live: bool) -> PluginResult<()> {
        folders.remove(&self.live).await?;
        if had_live {
            folders.rename(&self.old, &self.live).await?;
        }
        Ok(())
    }
}

/// Checks a typed firmware release before it is saved, and returns it cleaned (no leading "v",
/// trimmed): it must be a version like 22.5.0.
fn check_version(typed: &str) -> PluginResult<String> {
    clean_version(typed).map(str::to_owned)
}
/// The GitHub API address of THZoria/NX_Firmware's release `version`.
pub fn release_url(version: &str) -> String {
    format!("https://api.github.com/repos/THZoria/NX_Firmware/releases/tags/{version}")
}
/// The release of a version, kept for hours: GitHub allows 60 requests an hour without a token.
fn release_cache(version: &str) -> String {
    format!("cache/firmware-release-{version}.json")
}

/// The Switch version the player typed, without a leading "v", as it goes into URLs and a file
/// name. It must be dot-separated groups of digits, like 22.5.0. Anything else is refused, so a
/// path like "../latest" or ".." can't turn the release address into the list of all releases.
fn clean_version(typed: &str) -> PluginResult<&str> {
    let trimmed = typed.trim();
    let version = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let is_version = !version.is_empty()
        && version.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    if !is_version {
        return Err(PluginError::InvalidInput(format!("the firmware release \"{typed}\" isn't a version like 22.5.0")));
    }
    Ok(version)
}
const RELEASE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// The version Nexus installed last, in the plugin's storage.
const MARKER: &str = "firmware.json";
/// Where Lichii reads the firmware, in Lichii's data folder.
const CONTENTS: &str = "nand/system/Contents/registered";
/// The new firmware while it is unpacked, the old one while the new one moves in, and the
/// download: beside `Contents`, so Lichii never reads them.
const PART: &str = "nand/system/registered.part";
const OLD: &str = "nand/system/registered.old";
const ZIP: &str = "nand/system/firmware.zip.part";
/// The bytes asked for in one request, and read or written in one go.
const PIECE: u64 = 8 * 1024 * 1024;
/// More files than any firmware has: a zip with more isn't one.
const MAX_FILES: usize = 4096;

/// What the update is doing.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    Complete(String),
    Updating(Step),
    Failed(String),
}

/// A step of an update that runs: what it does, and for a download how far it is.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    what: String,
    /// From 0 to 1.
    progress: Option<f32>,
    /// A line under `what`, such as the megabytes downloaded.
    detail: String,
}

impl Step {
    fn new(what: impl Into<String>) -> Self {
        Self { what: what.into(), progress: None, detail: String::new() }
    }

    /// The download's step, at `at` of `size` bytes.
    fn download(at: u64, size: u64) -> Self {
        let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
        Self {
            what: "Downloading the firmware".into(),
            progress: Some(at as f32 / size.max(1) as f32),
            detail: format!("{} of {}", mb(at), mb(size)),
        }
    }
}

pub struct Firmware {
    /// Itself, for the update it runs in the background.
    me: Weak<Firmware>,
    host: Host,
    events: Arc<dyn EventSink>,
    phase: Mutex<Phase>,
    /// When a check last started on its own.
    last_auto: Mutex<Option<Instant>>,
}

impl Firmware {
    pub fn new(host: Host, events: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            host,
            events,
            phase: Mutex::new(Phase::Idle),
            last_auto: Mutex::new(None),
        })
    }

    /// Takes over the firmware settings that Nexus 0.16.4 and older saved among Lichii's
    /// settings, once: not when the firmware settings are there already. A saved release that
    /// isn't a version stays behind, and the default release applies.
    pub async fn adopt(&self, automatic: Option<bool>, release: Option<String>) -> PluginResult<()> {
        let path = RelPath::new(SETTINGS)?;
        if (automatic.is_none() && release.is_none()) || self.host.storage.exists(&path).await? {
            return Ok(());
        }
        let release = match release.map(|r| check_version(&r)) {
            Some(Ok(release)) => release,
            Some(Err(e)) => {
                self.host.log.warn(&format!("the saved firmware release stays behind: {e}"));
                DEFAULT_RELEASE.into()
            }
            None => DEFAULT_RELEASE.into(),
        };
        self.save(&FirmwareSettings { automatic: automatic.unwrap_or(false), release }).await
    }

    /// The firmware settings: the defaults until the player sets one. A file that doesn't read
    /// gives the defaults too, and a warning in the log.
    async fn settings(&self) -> FirmwareSettings {
        self.read_settings().await.unwrap_or_else(|e| {
            self.host.log.warn(&format!("{e}. The defaults apply."));
            FirmwareSettings::default()
        })
    }

    /// The firmware settings, or why their file doesn't read. No file gives the defaults.
    async fn read_settings(&self) -> Result<FirmwareSettings, String> {
        let read = async {
            let path = RelPath::new(SETTINGS)?;
            if !self.host.storage.exists(&path).await? {
                return Ok(FirmwareSettings::default());
            }
            let bytes = self.host.storage.read(&path).await?;
            Ok(serde_json::from_slice(&bytes)?)
        };
        read.await.map_err(|e: PluginError| format!("The firmware settings file ({SETTINGS}) doesn't read: {e}"))
    }

    async fn save(&self, settings: &FirmwareSettings) -> PluginResult<()> {
        self.host.storage.write(&RelPath::new(SETTINGS)?, &serde_json::to_vec(settings)?).await
    }

    /// The player typed `typed` into the Firmware release row: the next update installs it.
    async fn set_release(&self, typed: &Value) -> PluginResult<()> {
        let typed =
            typed.as_str().ok_or_else(|| PluginError::InvalidInput("the firmware release must be text".into()))?;
        let mut settings = self.settings().await;
        settings.release = check_version(typed)?;
        self.save(&settings).await
    }

    /// Starts a check on its own, bypassing the normal interval when firmware is missing.
    async fn start_auto(&self, version: String) {
        if matches!(self.phase(), Phase::Updating(_)) {
            return;
        }
        let missing = !self.has_firmware().await;
        {
            let mut last = self.last_auto.lock().unwrap_or_else(|p| p.into_inner());
            if !missing && last.is_some_and(|t| t.elapsed() < RELEASE_TTL) {
                return;
            }
            *last = Some(Instant::now());
        }
        self.start(false, version);
    }

    fn phase(&self) -> Phase {
        self.phase.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner()) = phase;
        if let Err(e) = self.events.publish(SLOTS_CHANGED, Value::Null) {
            self.host.log.warn(&format!("could not tell the core the firmware changed: {e}"));
        }
    }

    /// The version Nexus installed last.
    async fn installed(&self) -> Option<String> {
        let bytes = self.host.storage.read(&RelPath::new(MARKER).ok()?).await.ok()?;
        let marker: Value = serde_json::from_slice(&bytes).ok()?;
        marker["version"].as_str().map(str::to_owned)
    }

    /// Starts an update in the background, unless one is running. `asked`: the player pressed
    /// the button, so the release is asked for again, and an update that failed runs again.
    fn start(&self, asked: bool, version: String) {
        {
            let mut phase = self.phase.lock().unwrap_or_else(|p| p.into_inner());
            match &*phase {
                Phase::Updating(_) => return,
                Phase::Failed(_) if !asked => return,
                _ => *phase = Phase::Updating(Step::new("Checking for firmware updates…")),
            }
        }
        if let Err(e) = self.events.publish(SLOTS_CHANGED, Value::Null) {
            self.host.log.warn(&format!("could not tell the core the firmware check started: {e}"));
        }
        let Some(this) = self.me.upgrade() else { return };
        tokio::spawn(async move {
            let phase = match this.update(asked, &version).await {
                Ok(message) => Phase::Complete(message),
                Err(e) => {
                    this.host.log.warn(&format!("the firmware update failed: {e}"));
                    Phase::Failed(e.to_string())
                }
            };
            this.set_phase(phase);
        });
    }

    /// The Firmware block on Lichii's page: the firmware in Lichii's data folder, the update that
    /// runs or the last one's result, the firmware release to install (`release`), the automatic
    /// updates' switch (`auto`), and Update now.
    /// `unread`: why the firmware settings file doesn't read, shown in a row of its own.
    async fn panel(&self, auto: bool, release: &str, unread: Option<String>) -> Vec<SlotItem> {
        let phase = self.phase();
        let row = |label: &str, value: String, hint: String, tone| BlockRow {
            label: label.into(),
            value,
            hint,
            tone,
            ..BlockRow::default()
        };
        let installed = match (self.installed().await, self.has_firmware().await) {
            (Some(version), true) => row("Installed", version, "Nexus installed it".into(), Tone::Neutral),
            (None, true) => row(
                "Installed",
                "Not from Nexus".into(),
                "Update now puts the set firmware release in its place".into(),
                Tone::Neutral,
            ),
            (_, false) => row("Installed", "None".into(), "Some games need the firmware".into(), Tone::Warning),
        };
        let mut rows = vec![installed];
        if let Some(why) = unread {
            let hint = format!("{why}. The defaults apply until you change a setting here.");
            rows.push(row("Firmware settings", String::new(), hint, Tone::Warning));
        }
        match &phase {
            Phase::Updating(step) => rows.push(BlockRow {
                progress: step.progress,
                value: step.progress.map(|p| format!("{}%", (p * 100.0) as u32)).unwrap_or_default(),
                ..row(&step.what, String::new(), step.detail.clone(), Tone::Info)
            }),
            Phase::Failed(error) => {
                rows.push(row("The last update failed", String::new(), error.clone(), Tone::Danger))
            }
            Phase::Complete(message) => rows.push(row("Last check", String::new(), message.clone(), Tone::Neutral)),
            Phase::Idle => {}
        }
        rows.push(BlockRow {
            action: Some(SET_RELEASE.into()),
            input: true,
            ..row(
                "Firmware release",
                release.to_owned(),
                "The Switch version to install, with its keys. Type one like 22.5.0.".into(),
                Tone::Neutral,
            )
        });
        rows.push(BlockRow {
            action: Some(TOGGLE_AUTO.into()),
            switch: Some(auto),
            ..row(
                "Automatic updates",
                String::new(),
                "Nexus keeps the set release installed, checked every 6 hours. Nintendo's firmware: check that \
                 you may use it where you live."
                    .into(),
                Tone::Neutral,
            )
        });
        if !matches!(phase, Phase::Updating(_)) {
            rows.push(BlockRow {
                action: Some(UPDATE_NOW.into()),
                ..row(
                    "Update now",
                    String::new(),
                    "Installs the set firmware release and its keys".into(),
                    Tone::Neutral,
                )
            });
        }
        let block = SlotContent::Block { title: "Firmware".into(), rows };
        vec![SlotItem { id: "firmware".into(), order: Order::default(), content: block }]
    }

    /// Whether Lichii's data folder has firmware: for the badge when Nexus didn't install one.
    async fn has_firmware(&self) -> bool {
        match self.place(CONTENTS).await {
            Ok(dir) => self.host.folders.list(&dir).await.is_ok_and(|e| !e.is_empty()),
            Err(_) => false,
        }
    }

    /// `path` in the first of Lichii's data folders.
    async fn place(&self, path: &str) -> PluginResult<HostPath> {
        let root =
            self.host.folders.roots(DATA).await?.first().map(|r| r.root).ok_or_else(|| {
                PluginError::Precondition("Lichii's data folder isn't set: set it in Settings".into())
            })?;
        Ok(HostPath::new(DATA, root, RelPath::new(path)?))
    }

    /// Installs the firmware `version` and the keys of the same version.
    async fn update(&self, asked: bool, version: &str) -> PluginResult<String> {
        let version = clean_version(version)?;
        let folders = self.host.folders.as_ref();
        let firmware =
            Swap { live: self.place(CONTENTS).await?, staged: self.place(PART).await?, old: self.place(OLD).await? };
        let zip = self.place(ZIP).await?;
        firmware.recover(folders).await?;
        folders.remove(&firmware.staged).await?;
        folders.remove(&zip).await?;

        let cache_path = RelPath::new(release_cache(version))?;
        if asked {
            cache::forget(self.host.storage.as_ref(), &cache_path).await?;
        }
        let storage = self.host.storage.as_ref();
        let json = cache::cached_json(
            storage,
            self.host.http.as_ref(),
            &self.host.log,
            &release_url(version),
            &cache_path,
            RELEASE_TTL,
        )
        .await
        .map_err(|e| PluginError::NotFound(format!("couldn't get firmware {version} from THZoria/NX_Firmware: {e}")))?;
        let release = github::releases(&json, false)
            .into_iter()
            .next()
            .ok_or_else(|| PluginError::NotFound("no firmware release".into()))?;
        let current = self.installed().await.as_deref() == Some(release.tag.as_str()) && self.has_firmware().await;
        let keys_marker = RelPath::new("keys-version.json")?;
        let keys_current = self
            .host
            .storage
            .read(&keys_marker)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<String>(&b).ok())
            .is_some_and(|v| v == version);
        let keys_exist = self.host.folders.exists(&self.place("keys/prod.keys").await?).await?;
        if current && keys_current && keys_exist {
            return Ok(format!(
                "Firmware {} is up to date. Keys {version} are installed. No updates needed.",
                release.tag
            ));
        }
        self.set_phase(Phase::Updating(Step::new(format!(
            "Firmware {} is {}. Downloading keys {version}…",
            release.tag,
            if current { "up to date" } else { "available" }
        ))));
        let keys = self.download_keys(version).await?;
        if current {
            self.install_keys(keys).await?;
            self.host
                .storage
                .write(
                    &keys_marker,
                    &serde_json::to_vec(version).map_err(|e| PluginError::InvalidInput(e.to_string()))?,
                )
                .await?;
            return Ok(format!("Firmware {} is up to date. Installed keys {version}.", release.tag));
        }
        let asset = release
            .assets
            .iter()
            .find(|a| a.name.starts_with("Firmware.") && a.name.ends_with(".zip"))
            .ok_or_else(|| PluginError::NotFound(format!("firmware {} has no zip", release.tag)))?;
        let sha256 = asset
            .sha256
            .clone()
            .ok_or_else(|| PluginError::Precondition(format!("GitHub gives no SHA-256 for {}", asset.name)))?;
        let unpacked = async {
            self.download(&asset.url, asset.size, &sha256, &zip).await?;
            self.set_phase(Phase::Updating(Step::new(format!("Unpacking firmware {}…", release.tag))));
            unpack(folders, &zip, &firmware.staged).await
        }
        .await;
        self.clean(&zip).await;
        if let Err(e) = unpacked {
            self.clean(&firmware.staged).await;
            return Err(e);
        }
        self.set_phase(Phase::Updating(Step::new(format!("Installing firmware {} and keys {version}…", release.tag))));
        let had_live = match firmware.swap(folders, &self.host.log).await {
            Ok(had_live) => had_live,
            Err(e) => {
                self.clean(&firmware.staged).await;
                return Err(e);
            }
        };
        if let Err(e) = self.install_keys(keys).await {
            // Keep the previous firmware usable if the keys could not be installed.
            firmware.undo(folders, had_live).await?;
            return Err(e);
        }
        self.host
            .storage
            .write(&keys_marker, &serde_json::to_vec(version).map_err(|e| PluginError::InvalidInput(e.to_string()))?)
            .await?;
        let marker = serde_json::json!({ "version": release.tag });
        self.host.storage.write(&RelPath::new(MARKER)?, marker.to_string().as_bytes()).await?;
        // The new firmware is in place: an old one left behind goes at the next update.
        self.clean(&firmware.old).await;
        Ok(format!("Updated firmware to {}. Installed keys {version}.", release.tag))
    }

    /// Reads only known key files, without trusting paths supplied by the archive.
    async fn download_keys(&self, version: &str) -> PluginResult<Vec<(String, Vec<u8>)>> {
        let url = format!("https://files.prodkeys.net/ProdKeys.NET-v{version}.zip");
        let response = self.host.http.send(HttpRequest::get(&url)).await?;
        if response.status == 404 {
            return Err(PluginError::NotFound(format!(
                "keys archive version {version} is unavailable: {url} (HTTP 404); the firmware was not changed"
            )));
        }
        if response.status != 200 {
            return Err(PluginError::Network(format!("keys download from {url} returned status {}", response.status)));
        }
        let invalid = |e| PluginError::InvalidInput(format!("invalid keys zip: {e}"));
        let mut archive = zip::ZipArchive::new(Cursor::new(response.body)).map_err(|e| invalid(e.to_string()))?;
        let mut keys = Vec::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| invalid(e.to_string()))?;
            let name = file.name().map_err(|e| invalid(e.to_string()))?.rsplit('/').next().unwrap_or("").to_owned();
            if !matches!(name.as_str(), "prod.keys" | "title.keys") {
                continue;
            }
            if keys.iter().any(|(n, _)| n == &name) {
                return Err(invalid(format!("duplicate {name}")));
            }
            let mut bytes = Vec::new();
            file.by_ref().take(1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| invalid(e.to_string()))?;
            if bytes.is_empty() || bytes.len() > 1024 * 1024 {
                return Err(invalid(format!("invalid size for {name}")));
            }
            keys.push((name, bytes));
        }
        if !keys.iter().any(|(n, _)| n == "prod.keys") {
            return Err(invalid("missing prod.keys".into()));
        }
        Ok(keys)
    }

    async fn install_keys(&self, keys: Vec<(String, Vec<u8>)>) -> PluginResult<()> {
        let folders = self.host.folders.as_ref();
        for (name, bytes) in keys {
            let file = Swap {
                live: self.place(&format!("keys/{name}")).await?,
                staged: self.place(&format!("keys/{name}.part")).await?,
                old: self.place(&format!("keys/{name}.old")).await?,
            };
            file.recover(folders).await?;
            folders.write(&file.staged, &bytes).await?;
            file.swap(folders, &self.host.log).await?;
            self.clean(&file.old).await;
        }
        Ok(())
    }

    /// Removes a leftover of an update. A failure is logged: the next update removes it.
    async fn clean(&self, path: &HostPath) {
        if let Err(e) = self.host.folders.remove(path).await {
            self.host.log.warn(&format!("could not remove {}: {e}", path.path));
        }
    }

    /// Downloads `url` (`size` bytes) into `into`, in pieces, and checks its SHA-256.
    async fn download(&self, url: &str, size: u64, sha256: &str, into: &HostPath) -> PluginResult<()> {
        if size == 0 {
            return Err(PluginError::InvalidInput("the firmware's size isn't known".into()));
        }
        self.set_phase(Phase::Updating(Step::download(0, size)));
        let mut hash = Sha256::new();
        let mut at = 0;
        while at < size {
            let end = (at + PIECE).min(size) - 1;
            let mut request = HttpRequest::get(url);
            request.headers.push(("Range".into(), format!("bytes={at}-{end}")));
            let response = self.host.http.send(request).await?;
            // Only a piece of the asked size: a whole firmware is too big to hold in memory.
            if response.status != 206 || response.body.len() as u64 != end + 1 - at {
                return Err(PluginError::Network(format!(
                    "the server didn't send the firmware in pieces (status {}, {} bytes)",
                    response.status,
                    response.body.len()
                )));
            }
            hash.update(&response.body);
            self.host.folders.write_at(into, at, &response.body).await?;
            at += response.body.len() as u64;
            self.set_phase(Phase::Updating(Step::download(at, size)));
        }
        let got = hex(&hash.finalize());
        if got != sha256 {
            return Err(PluginError::InvalidInput(
                "the firmware download doesn't match the SHA-256 GitHub gives".into(),
            ));
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// The plugin reads the zip itself: the host's download and unpack step (`DownloadPlan`) writes
// only into the plugin's own folder, and the firmware goes into Lichii's data folder.

/// One file of a zip: its name, and where its bytes are.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    /// The local header's offset; the bytes come after it, its name and its extra field.
    header: u64,
    size: u64,
}

fn u16_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?).into())
}

fn u32_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?).into())
}

fn bad(why: &str) -> PluginError {
    PluginError::InvalidInput(format!("the firmware zip {why}"))
}

/// The files of the zip at `zip`, `len` bytes long, from its central directory (its index at
/// the end): each must be stored as it is, at the top, and named `*.nca`. The whole zip's
/// SHA-256 was checked already, so the files' own CRCs aren't.
async fn entries(folders: &dyn Folders, zip: &HostPath, len: u64) -> PluginResult<Vec<Entry>> {
    // The end record: 22 bytes, after a comment of up to 64 KiB.
    let tail_len = len.min(22 + 65535);
    let tail = folders.read_range(zip, len - tail_len, tail_len).await?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| bad("has no end record"))?;
    let count = u16_at(&tail, end + 10).ok_or_else(|| bad("is cut short"))? as usize;
    let index_len = u32_at(&tail, end + 12).ok_or_else(|| bad("is cut short"))?;
    let index_at = u32_at(&tail, end + 16).ok_or_else(|| bad("is cut short"))?;
    if count > MAX_FILES || index_len > PIECE || index_at == 0xFFFF_FFFF {
        return Err(bad("is too big"));
    }
    let index = folders.read_range(zip, index_at, index_len).await?;
    let mut out = Vec::with_capacity(count);
    let mut at = 0;
    for _ in 0..count {
        let field = |off| u32_at(&index, at + off).ok_or_else(|| bad("has a broken index"));
        let short = |off| u16_at(&index, at + off).ok_or_else(|| bad("has a broken index"));
        if field(0)? != 0x0201_4b50 {
            return Err(bad("has a broken index"));
        }
        let (method, size, stored) = (short(10)?, field(24)?, field(20)?);
        let (name_len, extra_len, comment_len) = (short(28)? as usize, short(30)? as usize, short(32)? as usize);
        let header = field(42)?;
        let name = index.get(at + 46..at + 46 + name_len).ok_or_else(|| bad("has a broken index"))?;
        let name = String::from_utf8(name.to_vec()).map_err(|_| bad("has a name that isn't text"))?;
        if method != 0 || stored != size {
            return Err(bad(&format!("compresses {name}: only stored files are read")));
        }
        if [size, header].contains(&0xFFFF_FFFF) {
            return Err(bad("is a zip64 zip"));
        }
        let plain = name.ends_with(".nca") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        if !plain {
            return Err(bad(&format!("has {name}, which isn't a firmware file")));
        }
        out.push(Entry { name, header, size });
        at += 46 + name_len + extra_len + comment_len;
    }
    if out.is_empty() {
        return Err(bad("is empty"));
    }
    Ok(out)
}

/// Copies the files of the zip at `zip` into the folder `into`.
async fn unpack(folders: &dyn Folders, zip: &HostPath, into: &HostPath) -> PluginResult<()> {
    let len = folders.size(zip).await?;
    for entry in entries(folders, zip, len).await? {
        let local = folders.read_range(zip, entry.header, 30).await?;
        if u32_at(&local, 0) != Some(0x0403_4b50) {
            return Err(bad(&format!("has no header for {}", entry.name)));
        }
        let skip = 30 + u16_at(&local, 26).unwrap_or(0) + u16_at(&local, 28).unwrap_or(0);
        let start = entry.header + skip;
        if start + entry.size > len {
            return Err(bad(&format!("is cut short in {}", entry.name)));
        }
        let file = into.join(&entry.name)?;
        if entry.size == 0 {
            folders.write(&file, &[]).await?;
        }
        let mut done = 0;
        while done < entry.size {
            let piece = folders.read_range(zip, start + done, (entry.size - done).min(PIECE)).await?;
            if piece.is_empty() {
                return Err(bad(&format!("is cut short in {}", entry.name)));
            }
            folders.write_at(&file, done, &piece).await?;
            done += piece.len() as u64;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_download_s_step_has_its_share_and_its_megabytes() {
        let step = Step::download(159_383_552, 340_474_472);
        assert_eq!(step.detail, "159.4 MB of 340.5 MB");
        assert_eq!((step.progress.unwrap() * 100.0) as u32, 46);
        assert_eq!(Step::download(0, 340_474_472).progress, Some(0.0));
    }
}
