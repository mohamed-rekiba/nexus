//! umu-launcher, which runs Windows games with Proton: the plugin installs it into its data
//! folder from the latest release on GitHub, the release's zipapp (`umu/umu-run`, which needs
//! only Python 3).

use std::io::Read;

use sha2::{Digest, Sha256};

/// umu-launcher's latest release, on GitHub's API.
pub const LATEST: &str = "https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest";
/// Where the plugin keeps umu-run, in its data folder.
pub const PROGRAM: &str = "umu/umu-run";
/// The version of the umu-run kept, the release's tag.
pub const VERSION: &str = "umu/version";

/// Whether `name` is a release's zipapp: `umu-launcher-1.4.4-zipapp.tar`.
pub fn zipapp(name: &str) -> bool {
    name.starts_with("umu-launcher-") && name.ends_with("-zipapp.tar")
}

/// `bytes` when their SHA-256 is `sha256` (lowercase hex), else why not.
pub fn checked(bytes: &[u8], sha256: &str) -> Result<(), String> {
    let got = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got == sha256 { Ok(()) } else { Err(format!("its checksum is {got}, and GitHub lists {sha256}")) }
}

/// umu-run, from a zipapp archive.
pub fn program(tar: &[u8]) -> Result<Vec<u8>, String> {
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
