//! Community compatibility lists, by title id.
//!
//! - KytyPS5 (PS5): `https://kytyps5.github.io/data/compatibility.json`,
//!   `{ "PPSA…": { "status": "InGame", "platforms": { "linux": { "status": … } } } }`.
//! - shadPS4 (PS4): the `compatibility_data.json` release asset of
//!   `shadps4-compatibility/shadps4-game-compatibility`,
//!   `{ "CUSA…": { "os-linux": { "status": "status-ingame", "last_tested": … } } }`.
//!
//! The list's own top-level status is the best of any system, which would let a Windows
//! "in game" hide a Linux "doesn't boot", so the result for this system wins when there is one.

use std::collections::HashMap;

use nexus_api::serde_json::Value;
use nexus_api::{CompatInfo, CompatLevel, Platform};

fn kyty_level(s: &str) -> Option<CompatLevel> {
    Some(match s {
        "InGame" => CompatLevel::Ingame,
        "MainMenu" => CompatLevel::Menus,
        "Logo" => CompatLevel::Boots,
        "DoesntBoot" => CompatLevel::Nothing,
        _ => return None,
    })
}

fn shad_level(s: &str) -> Option<CompatLevel> {
    Some(match s {
        "status-playable" => CompatLevel::Playable,
        "status-ingame" => CompatLevel::Ingame,
        "status-menus" => CompatLevel::Menus,
        "status-boots" => CompatLevel::Boots,
        "status-nothing" => CompatLevel::Nothing,
        _ => return None,
    })
}

pub fn os_name(platform: Option<Platform>) -> &'static str {
    match platform {
        Some(Platform::Macos) => "macOS",
        Some(Platform::Windows) => "Windows",
        _ => "Linux",
    }
}

/// What the notes say about where a result comes from. `from` is the system the level was
/// tested on, when the list says; `version` the build it was tested with.
fn info(
    level: CompatLevel,
    from: Option<String>,
    platform: Option<Platform>,
    tested_on: &[String],
    version: &str,
    tested: Option<String>,
) -> CompatInfo {
    let here = from.as_deref() == Some(os_name(platform));
    let source = if here {
        format!("Tested on {}", os_name(platform))
    } else if tested_on.is_empty() {
        "Tested on another system".to_owned()
    } else {
        format!("Tested on {}, not yet on {}", tested_on.join(", "), os_name(platform))
    };
    let notes = if version.is_empty() { source } else { format!("{source} ({version})") };
    CompatInfo { level, notes: Some(notes), source: from, page: None, tested }
}

/// The `YYYY-MM-DD` at the start of `stamp`, such as `2026-10-01T22:30:38Z`.
fn day(stamp: &str) -> Option<String> {
    let day = stamp.get(..10)?;
    let digits = day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    digits.then(|| day.to_owned())
}

/// KytyPS5's list for `platform`.
pub fn kyty(json: &Value, platform: Option<Platform>) -> HashMap<String, CompatInfo> {
    let key = match platform {
        Some(Platform::Macos) => "macos",
        Some(Platform::Windows) => "windows",
        _ => "linux",
    };
    let mut out = HashMap::new();
    let Some(games) = json.as_object() else { return out };
    for (tid, e) in games {
        let Some(best) = e["status"].as_str().and_then(kyty_level) else { continue };
        let per_os = e["platforms"].as_object();
        let here = per_os.and_then(|p| p.get(key)).and_then(|l| l["status"].as_str()).and_then(kyty_level);
        let mut tested_on: Vec<String> = per_os.map(|p| p.keys().map(|k| kyty_os(k)).collect()).unwrap_or_default();
        tested_on.sort();
        let version = per_os
            .and_then(|p| p.get(key).or_else(|| p.values().next()))
            .and_then(|l| l["version"].as_str())
            .unwrap_or_default();
        // Where the best result was tested, when it isn't this system's.
        let from = if here.is_some() {
            Some(os_name(platform).to_owned())
        } else {
            per_os.and_then(|p| {
                let mut best_on: Vec<_> = p
                    .iter()
                    .filter(|(_, l)| l["status"].as_str().and_then(kyty_level) == Some(best))
                    .map(|(k, _)| kyty_os(k))
                    .collect();
                best_on.sort();
                best_on.into_iter().next()
            })
        };
        out.insert(tid.to_uppercase(), info(here.unwrap_or(best), from, platform, &tested_on, version, None));
    }
    out
}

fn kyty_os(key: &str) -> String {
    match key {
        "linux" => "Linux".into(),
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        other => other.to_owned(),
    }
}

fn shad_os(key: &str) -> Option<String> {
    match key {
        "os-linux" => Some("Linux".to_owned()),
        "os-windows" => Some("Windows".to_owned()),
        "os-macOS" => Some("macOS".to_owned()),
        _ => None,
    }
}

/// shadPS4's list for `platform`.
pub fn shad(json: &Value, platform: Option<Platform>) -> HashMap<String, CompatInfo> {
    let key = match platform {
        Some(Platform::Macos) => "os-macOS",
        Some(Platform::Windows) => "os-windows",
        _ => "os-linux",
    };
    let mut out = HashMap::new();
    let Some(games) = json.as_object() else { return out };
    for (tid, per_os) in games {
        let Some(per_os) = per_os.as_object() else { continue };
        let level = |e: &Value| e["status"].as_str().and_then(shad_level);
        let Some(best) = per_os.values().filter_map(level).max() else { continue };
        let here = per_os.get(key).and_then(level);
        let mut tested_on: Vec<String> = per_os.keys().filter_map(|k| shad_os(k)).collect();
        tested_on.sort();
        let tested = per_os
            .get(key)
            .and_then(|e| e["last_tested"].as_str())
            .or_else(|| per_os.values().filter_map(|e| e["last_tested"].as_str()).max())
            .and_then(day);
        let from = if here.is_some() {
            Some(os_name(platform).to_owned())
        } else {
            let mut best_on: Vec<_> =
                per_os.iter().filter(|(_, e)| level(e) == Some(best)).filter_map(|(k, _)| shad_os(k)).collect();
            best_on.sort();
            best_on.into_iter().next()
        };
        out.insert(tid.to_uppercase(), info(here.unwrap_or(best), from, platform, &tested_on, "", tested));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_api::serde_json::json;

    #[test]
    fn this_system_s_result_wins_over_the_best_elsewhere() {
        let list = json!({
            "ppsa01": { "status": "InGame", "platforms": { "windows": { "status": "InGame" }, "linux": { "status": "DoesntBoot", "version": "KytyPS5-2026-10-01" } } },
            "PPSA02": { "status": "MainMenu", "platforms": { "windows": { "status": "MainMenu" } } },
            "PPSA03": { "status": "Unknown" },
        });
        let db = kyty(&list, Some(Platform::Linux));
        assert_eq!(db["PPSA01"].level, CompatLevel::Nothing);
        assert_eq!(db["PPSA01"].notes.as_deref(), Some("Tested on Linux (KytyPS5-2026-10-01)"));
        assert_eq!(db["PPSA02"].level, CompatLevel::Menus);
        assert_eq!(db["PPSA02"].notes.as_deref(), Some("Tested on Windows, not yet on Linux"));
        assert!(!db.contains_key("PPSA03"));

        let shad_list = json!({ "CUSA04118": {
            "os-windows": { "status": "status-playable", "last_tested": "2026-09-01" },
            "os-linux": { "status": "status-menus", "last_tested": "2026-08-01" } } });
        let db = shad(&shad_list, Some(Platform::Linux));
        assert_eq!(db["CUSA04118"].level, CompatLevel::Menus);
        assert_eq!(shad(&shad_list, Some(Platform::Macos))["CUSA04118"].level, CompatLevel::Playable);
        let stamped =
            json!({ "CUSA00001": { "os-linux": { "status": "status-boots", "last_tested": "2026-10-01T22:30:38Z" } } });
        assert_eq!(shad(&stamped, Some(Platform::Linux))["CUSA00001"].tested.as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn the_badge_names_the_system_the_result_was_tested_on() {
        let list = json!({
            "PPSA01": { "status": "InGame", "platforms": { "macos": { "status": "InGame" } } },
            "PPSA02": { "status": "InGame", "platforms": { "windows": { "status": "MainMenu" }, "linux": { "status": "InGame" } } },
        });
        let db = kyty(&list, Some(Platform::Macos));
        let badge = |tid: &str, platform| match crate::games::compat_badge(&db[tid], platform) {
            nexus_api::SlotContent::Badge { label, short, .. } => (label, short),
            _ => unreachable!("a badge"),
        };
        assert_eq!(badge("PPSA01", Some(Platform::Macos)), ("In-game on macOS".into(), "In-game".into()));
        assert_eq!(
            badge("PPSA02", Some(Platform::Macos)),
            ("In-game on Linux · untested on macOS".into(), "Linux · In-game".into())
        );

        let shad_list = json!({ "CUSA04118": { "os-windows": { "status": "status-playable" } } });
        let shad_db = shad(&shad_list, Some(Platform::Linux));
        let nexus_api::SlotContent::Badge { label, short, .. } =
            crate::games::compat_badge(&shad_db["CUSA04118"], Some(Platform::Linux))
        else {
            unreachable!("a badge")
        };
        assert_eq!((label.as_str(), short.as_str()), ("Playable on Windows · untested on Linux", "Win · Playable"));
    }
}
