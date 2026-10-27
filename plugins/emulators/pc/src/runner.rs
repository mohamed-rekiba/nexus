//! What runs a Windows game off Windows: umu-launcher, which runs it with Proton, or Wine. On
//! Linux the plugin installs umu-launcher into its data folder from the latest release on
//! GitHub, the release's zipapp (`umu/umu-run`, which needs only Python 3).

use std::io::Read;

use nexus_api::serde_json::Value;
use nexus_api::*;
use sha2::{Digest, Sha256};

use crate::Pc;

/// umu-launcher's latest release, on GitHub's API.
const LATEST: &str = "https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest";
/// Where the plugin keeps umu-run, in its data folder.
pub const PROGRAM: &str = "umu/umu-run";
/// The version of the umu-run kept, the release's tag.
const VERSION: &str = "umu/version";

/// Whether `name` is a release's zipapp: `umu-launcher-1.4.4-zipapp.tar`.
fn zipapp(name: &str) -> bool {
    name.starts_with("umu-launcher-") && name.ends_with("-zipapp.tar")
}

/// Whether the SHA-256 of `bytes` is `sha256` (lowercase hex); else why not.
fn checked(bytes: &[u8], sha256: &str) -> Result<(), String> {
    let got = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got == sha256 { Ok(()) } else { Err(format!("its checksum is {got}, and GitHub lists {sha256}")) }
}

/// umu-run, from a zipapp archive.
fn program(tar: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = tar::Archive::new(tar);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.path().map_err(|e| e.to_string())?.to_str() == Some(PROGRAM) {
            let mut out = Vec::new();
            entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
            return Ok(out);
        }
    }
    Err(format!("it has no {PROGRAM}"))
}

/// What runs a Windows game: umu-launcher (Proton) or Wine.
pub(crate) struct Runner {
    pub program: Location,
    pub umu: bool,
}

impl Pc {
    /// What runs a Windows game here: on Linux the umu-launcher this plugin installed, then one
    /// on the system, then Wine. `None` when there is none.
    pub(crate) async fn runner(&self) -> Option<Runner> {
        if self.platform == Platform::Linux && self.umu_version().await.is_some() {
            return Some(Runner { program: Location::Data(RelPath::new(PROGRAM).ok()?), umu: true });
        }
        let candidates: &[&'static str] =
            if self.platform == Platform::Linux { &["umu-run", "wine", "wine64"] } else { &["wine", "wine64"] };
        for name in candidates {
            if matches!(self.ctx.programs().find(&Location::System((*name).into())).await, Ok(Some(_))) {
                return Some(Runner { program: Location::System((*name).into()), umu: *name == "umu-run" });
            }
        }
        None
    }

    /// The version of the umu-launcher this plugin installed, if it did.
    async fn umu_version(&self) -> Option<String> {
        let storage = self.ctx.storage();
        if !storage.exists(&RelPath::new(PROGRAM).ok()?).await.unwrap_or(false) {
            return None;
        }
        let version = storage.read(&RelPath::new(VERSION).ok()?).await.ok()?;
        Some(String::from_utf8_lossy(&version).trim().to_owned()).filter(|v| !v.is_empty())
    }

    /// Installs umu-launcher's latest release into the data folder, in place of the one there:
    /// its zipapp, checked against the SHA-256 GitHub lists for it.
    pub(crate) async fn install_umu(&self) -> PluginResult<()> {
        let get = |url: String| async move {
            let answer = self.ctx.http().send(HttpRequest::get(url.clone())).await?;
            if answer.status != 200 {
                return Err(PluginError::Network(format!("{url} answered {}", answer.status)));
            }
            Ok(answer)
        };
        let release: Value = get(LATEST.into()).await?.json()?;
        let found = emulator_kit::github::builds(&release, false, zipapp);
        let (release, asset) = found
            .into_iter()
            .next()
            .ok_or_else(|| PluginError::NotFound("umu-launcher's latest release has no zipapp to install".into()))?;
        let sha256 = asset
            .sha256
            .ok_or_else(|| PluginError::Precondition(format!("GitHub has no checksum for {}", asset.name)))?;
        let tar = get(asset.url).await?.body;
        let fail =
            |why: String| PluginError::Precondition(format!("umu-launcher {} wasn't installed: {why}", release.tag));
        checked(&tar, &sha256).map_err(|why| fail(format!("the download isn't the one GitHub lists, {why}")))?;
        let program = program(&tar).map_err(fail)?;
        // Beside the one in use, then in its place: a failed install leaves it as it was.
        let storage = self.ctx.storage();
        let part = RelPath::new(format!("{}.part", PROGRAM))?;
        storage.write(&part, &program).await?;
        storage.set_executable(&part).await?;
        storage.rename(&part, &RelPath::new(PROGRAM)?).await?;
        storage.write(&RelPath::new(VERSION)?, release.tag.as_bytes()).await?;
        self.ctx.log().info(&format!("installed umu-launcher {}", release.tag));
        Ok(())
    }

    /// The plugin page's "Windows games" section: what runs them, and on Linux the button that
    /// installs or updates umu-launcher.
    pub(crate) async fn windows_panel(&self) -> Vec<SlotItem> {
        let runs = match (self.runner().await, self.umu_version().await) {
            (Some(Runner { umu: true, .. }), Some(version)) => format!("Proton, through umu-launcher {version}"),
            (Some(Runner { umu: true, .. }), None) => "Proton, through the system's umu-launcher".into(),
            (Some(_), _) => "Wine".into(),
            (None, _) if self.platform == Platform::Linux => {
                "Nothing yet: install umu-launcher (Proton) or Wine".into()
            }
            (None, _) => "Nothing yet: install Wine".into(),
        };
        let mut items = vec![SlotItem {
            id: "windows".into(),
            order: Order::default(),
            content: SlotContent::Section {
                title: "Windows games".into(),
                rows: vec![SectionRow { label: "Run with".into(), value: runs }],
            },
        }];
        if self.platform == Platform::Linux {
            let label = if self.umu_version().await.is_some() { "Update umu-launcher" } else { "Install umu-launcher" };
            items.push(SlotItem {
                id: "umu".into(),
                order: Order::default().after("windows"),
                content: SlotContent::Action {
                    label: label.into(),
                    action: "install-umu".into(),
                    args: Value::Null,
                    style: ActionStyle::Secondary,
                    icon: None,
                },
            });
        }
        items
    }

    pub(crate) fn runner_hint(&self) -> &'static str {
        if self.platform == Platform::Linux {
            "This is a Windows game: install umu-launcher (Emulators, then PC: it runs the game with Proton) or Wine to play it"
        } else {
            "This is a Windows game: install Wine to play it"
        }
    }
}
