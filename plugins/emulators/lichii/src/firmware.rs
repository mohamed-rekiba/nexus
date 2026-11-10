//! The Switch firmware in Eden's folder, kept up to date from the releases of
//! THZoria/NX_Firmware, with configured keys from files.prodkeys.net.
//!
//! - The player turns it on (`firmware_updates`), or presses Update the firmware on Lichii's
//!   panel. With it on, a check starts when the core reads the panel (at the start and at each
//!   update check), at most once every [`RELEASE_TTL`]. A failed update isn't tried again on
//!   its own until Nexus starts again.
//! - A release is one zip of `.nca` files, stored without compression. The plugin downloads it
//!   in pieces into Eden's folder, checks its SHA-256 against GitHub's, and unpacks the files
//!   into a folder beside the firmware. Two renames put it in place of the old firmware:
//!   [`CONTENTS`], where Lichii reads it at its next start.
//! - Anything that fails leaves the old firmware as it was. A run that stopped between the two
//!   renames is put back first, at the next update.
//! - The version Nexus installed last is kept in the plugin's storage ([`MARKER`]).

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use emulator_kit::host::Host;
use emulator_kit::{cache, github};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;
use sha2::{Digest, Sha256};

use crate::DATA;

/// The newest firmware release.
pub const FIRMWARE_RELEASE: &str = "https://api.github.com/repos/THZoria/NX_Firmware/releases/latest";
/// The release, kept for hours: GitHub allows 60 requests an hour without a token.
const RELEASE_CACHE: &str = "cache/firmware-release.json";
const RELEASE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// The version Nexus installed last, in the plugin's storage.
const MARKER: &str = "firmware.json";
/// Where Lichii reads the firmware, in Eden's folder.
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
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Complete(String),
    Updating(String),
    Failed(String),
}

pub struct Firmware {
    host: Host,
    events: Arc<dyn EventSink>,
    phase: Mutex<Phase>,
    /// When a check last started on its own.
    last_auto: Mutex<Option<Instant>>,
}

impl Firmware {
    pub fn new(host: Host, events: Arc<dyn EventSink>) -> Self {
        Self { host, events, phase: Mutex::new(Phase::Idle), last_auto: Mutex::new(None) }
    }

    /// Starts a check on its own, bypassing the normal interval when firmware is missing.
    pub async fn start_auto(self: &Arc<Self>, keys_version: String) {
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
        self.start(false, keys_version);
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
    pub fn start(self: &Arc<Self>, asked: bool, keys_version: String) {
        {
            let mut phase = self.phase.lock().unwrap_or_else(|p| p.into_inner());
            match &*phase {
                Phase::Updating(_) => return,
                Phase::Failed(_) if !asked => return,
                _ => *phase = Phase::Updating("Checking for firmware updates…".into()),
            }
        }
        if let Err(e) = self.events.publish(SLOTS_CHANGED, Value::Null) {
            self.host.log.warn(&format!("could not tell the core the firmware check started: {e}"));
        }
        let this = self.clone();
        tokio::spawn(async move {
            let phase = match this.update(asked, &keys_version).await {
                Ok(message) => Phase::Complete(message),
                Err(e) => {
                    this.host.log.warn(&format!("the firmware update failed: {e}"));
                    Phase::Failed(e.to_string())
                }
            };
            this.set_phase(phase);
        });
    }

    /// The panel's status and action. While busy, the action shows progress; starting again is a no-op.
    pub async fn panel(&self) -> Vec<SlotItem> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        let badge = |label: String, tone| SlotContent::Badge { label, short: String::new(), tone, icon: None };
        let phase = self.phase();
        let content = match &phase {
            Phase::Updating(v) => badge(format!("Updating: {v}"), Tone::Info),
            Phase::Failed(e) => badge(format!("The firmware update failed: {e}"), Tone::Danger),
            Phase::Idle | Phase::Complete(_) => match self.installed().await {
                Some(v) if self.has_firmware().await => badge(format!("Firmware {v}"), Tone::Neutral),
                Some(_) => badge("No firmware".into(), Tone::Warning),
                None if self.has_firmware().await => badge("Firmware: not from Nexus".into(), Tone::Neutral),
                None => badge("No firmware".into(), Tone::Warning),
            },
        };
        let status = match &phase {
            Phase::Updating(message) => message.clone(),
            Phase::Complete(message) => message.clone(),
            Phase::Failed(error) => format!("The firmware update failed: {error}"),
            Phase::Idle => match self.installed().await {
                Some(version) => {
                    format!("Installed firmware: {version}. Check for updates to see if a newer version is available.")
                }
                None => "Check for updates to download firmware and the selected keys version.".into(),
            },
        };
        let mut items = vec![
            item("firmware", content),
            item(
                "firmware-status",
                SlotContent::Section {
                    title: "Firmware and keys".into(),
                    rows: vec![SectionRow { label: "Status".into(), value: status }],
                },
            ),
        ];
        items.push(item(
            "update-firmware",
            SlotContent::Action {
                label: match &phase {
                    Phase::Updating(message) => message.clone(),
                    _ => "Update the firmware".into(),
                },
                action: "update-firmware".into(),
                args: Value::Null,
                style: ActionStyle::Secondary,
                icon: None,
            },
        ));
        items
    }

    /// Whether Eden's folder has firmware: for the badge when Nexus didn't install one.
    async fn has_firmware(&self) -> bool {
        match self.place(CONTENTS).await {
            Ok(dir) => self.host.folders.list(&dir).await.is_ok_and(|e| !e.is_empty()),
            Err(_) => false,
        }
    }

    /// `path` in the first of Eden's folders.
    async fn place(&self, path: &str) -> PluginResult<HostPath> {
        let root =
            self.host.folders.roots(DATA).await?.first().map(|r| r.root).ok_or_else(|| {
                PluginError::Precondition("Lichii's data folder isn't set: set it in Settings".into())
            })?;
        Ok(HostPath::new(DATA, root, RelPath::new(path)?))
    }

    async fn update(&self, asked: bool, keys_version: &str) -> PluginResult<String> {
        let folders = self.host.folders.as_ref();
        let (contents, part, old, zip) =
            (self.place(CONTENTS).await?, self.place(PART).await?, self.place(OLD).await?, self.place(ZIP).await?);
        // A run that stopped between the renames: the old firmware goes back first.
        if folders.exists(&old).await? {
            if folders.exists(&contents).await? {
                folders.remove(&old).await?;
            } else {
                folders.rename(&old, &contents).await?;
            }
        }
        folders.remove(&part).await?;
        folders.remove(&zip).await?;

        let cache_path = RelPath::new(RELEASE_CACHE)?;
        if asked {
            cache::forget(self.host.storage.as_ref(), &cache_path).await?;
        }
        let storage = self.host.storage.as_ref();
        let json = cache::cached_json(
            storage,
            self.host.http.as_ref(),
            &self.host.log,
            FIRMWARE_RELEASE,
            &cache_path,
            RELEASE_TTL,
        )
        .await?;
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
            .is_some_and(|v| v == keys_version);
        let keys_exist = self.host.folders.exists(&self.place("keys/prod.keys").await?).await?;
        if current && keys_current && keys_exist {
            return Ok(format!(
                "Firmware {} is up to date. Keys {keys_version} are installed. No updates needed.",
                release.tag
            ));
        }
        self.set_phase(Phase::Updating(format!(
            "Firmware {} is {}. Downloading keys {keys_version}…",
            release.tag,
            if current { "up to date" } else { "available" }
        )));
        let keys = self.download_keys(keys_version).await?;
        if current {
            self.install_keys(keys).await?;
            self.host
                .storage
                .write(
                    &keys_marker,
                    &serde_json::to_vec(keys_version).map_err(|e| PluginError::InvalidInput(e.to_string()))?,
                )
                .await?;
            return Ok(format!("Firmware {} is up to date. Installed keys {keys_version}.", release.tag));
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
            self.set_phase(Phase::Updating(format!("Unpacking firmware {}…", release.tag)));
            unpack(folders, &zip, &part).await
        }
        .await;
        self.clean(&zip).await;
        if let Err(e) = unpacked {
            self.clean(&part).await;
            return Err(e);
        }
        self.set_phase(Phase::Updating(format!("Installing firmware {} and keys {keys_version}…", release.tag)));
        let had_old = folders.exists(&contents).await?;
        if had_old {
            folders.rename(&contents, &old).await?;
        }
        if let Err(e) = folders.rename(&part, &contents).await {
            if had_old && let Err(back) = folders.rename(&old, &contents).await {
                self.host.log.warn(&format!("could not put the old firmware back: {back}"));
            }
            self.clean(&part).await;
            return Err(e);
        }
        if let Err(e) = self.install_keys(keys).await {
            // Keep the previous firmware usable if the keys could not be installed.
            folders.remove(&contents).await?;
            if had_old {
                folders.rename(&old, &contents).await?;
            }
            return Err(e);
        }
        self.host
            .storage
            .write(
                &keys_marker,
                &serde_json::to_vec(keys_version).map_err(|e| PluginError::InvalidInput(e.to_string()))?,
            )
            .await?;
        let marker = serde_json::json!({ "version": release.tag });
        self.host.storage.write(&RelPath::new(MARKER)?, marker.to_string().as_bytes()).await?;
        // The new firmware is in place: an old one left behind goes at the next update.
        self.clean(&old).await;
        Ok(format!("Updated firmware to {}. Installed keys {keys_version}.", release.tag))
    }

    /// Reads only known key files, without trusting paths supplied by the archive.
    async fn download_keys(&self, version: &str) -> PluginResult<Vec<(String, Vec<u8>)>> {
        let version = version.strip_prefix('v').unwrap_or(version);
        if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(PluginError::InvalidInput("invalid keys version".into()));
        }
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
        for (name, bytes) in keys {
            let destination = self.place(&format!("keys/{name}")).await?;
            let staged = self.place(&format!("keys/{name}.part")).await?;
            let backup = self.place(&format!("keys/{name}.old")).await?;
            let folders = self.host.folders.as_ref();
            if folders.exists(&backup).await? {
                if folders.exists(&destination).await? {
                    folders.remove(&backup).await?;
                } else {
                    folders.rename(&backup, &destination).await?;
                }
            }
            folders.write(&staged, &bytes).await?;
            let had_old = folders.exists(&destination).await?;
            if had_old {
                folders.rename(&destination, &backup).await?;
            }
            if let Err(e) = folders.rename(&staged, &destination).await {
                if had_old && let Err(back) = folders.rename(&backup, &destination).await {
                    self.host.log.warn(&format!("could not restore {name}: {back}"));
                }
                return Err(e);
            }
            self.clean(&backup).await;
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
        self.set_phase(Phase::Updating("Downloading firmware: 0%…".into()));
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
            self.set_phase(Phase::Updating(progress(at, size)));
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

/// The download's progress for the badge, in the words of the Downloads screen:
/// `Downloading firmware: 46% (159.4 MB of 340.5 MB)`.
fn progress(at: u64, size: u64) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    format!("Downloading firmware: {}% ({} of {})", at * 100 / size.max(1), mb(at), mb(size))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// The plugin reads the zip itself: the host's download and unpack step (`DownloadPlan`) writes
// only into the plugin's own folder, and the firmware goes into Eden's.

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
    fn the_download_s_progress_reads_in_megabytes() {
        assert_eq!(progress(159_383_552, 340_474_472), "Downloading firmware: 46% (159.4 MB of 340.5 MB)");
        assert_eq!(progress(0, 340_474_472), "Downloading firmware: 0% (0.0 MB of 340.5 MB)");
    }
}
