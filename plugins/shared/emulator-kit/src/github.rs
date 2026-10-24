//! GitHub releases, as the REST API returns them.

use nexus_api::serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// From the asset's `digest` (`sha256:…`). GitHub has it for assets uploaded since 2025.
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub published_at: String,
    pub assets: Vec<Asset>,
}

/// Published releases (not drafts, and pre-releases only when `prereleases`), newest first.
/// Takes the list from `/releases` or one release from `/releases/latest`.
pub fn releases(json: &Value, prereleases: bool) -> Vec<Release> {
    let list: Vec<&Value> = match json.as_array() {
        Some(list) => list.iter().collect(),
        None if json.is_object() => vec![json],
        None => Vec::new(),
    };
    let mut out: Vec<Release> = list
        .into_iter()
        .filter(|r| r["draft"].as_bool() != Some(true) && (prereleases || r["prerelease"].as_bool() != Some(true)))
        .filter_map(|r| {
            let assets = r["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_owned(),
                        url: a["browser_download_url"].as_str()?.to_owned(),
                        size: a["size"].as_u64().unwrap_or(0),
                        sha256: a["digest"]
                            .as_str()
                            .and_then(|d| d.strip_prefix("sha256:"))
                            .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
                            .map(str::to_ascii_lowercase),
                    })
                })
                .collect();
            Some(Release {
                tag: r["tag_name"].as_str()?.to_owned(),
                published_at: r["published_at"].as_str().unwrap_or_default().to_owned(),
                assets,
            })
        })
        .collect();
    out.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    out
}

/// Each release that has an asset `pick` accepts, with that asset, newest first. A release
/// is published before its builds finish uploading, and one system's build can fail, so the
/// newest release may have nothing for this system yet.
pub fn builds(json: &Value, prereleases: bool, pick: impl Fn(&str) -> bool) -> Vec<(Release, Asset)> {
    releases(json, prereleases)
        .into_iter()
        .filter_map(|r| {
            let asset = r.assets.iter().find(|a| pick(&a.name))?.clone();
            Some((r, asset))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_api::serde_json::json;

    #[test]
    fn newest_published_release_with_a_matching_asset_first() {
        let asset = |n: &str| json!({ "name": n, "browser_download_url": format!("https://x/{n}"), "size": 1, "digest": format!("sha256:{}", "AB".repeat(32)) });
        let list = json!([
            { "tag_name": "old", "published_at": "2026-10-12T00:00:00Z", "assets": [asset("emu-Linux-x86_64.tar.gz")] },
            { "tag_name": "mac-only", "published_at": "2026-10-13T00:00:00Z", "assets": [asset("emu-macOS.zip")] },
            { "tag_name": "beta", "published_at": "2026-10-14T00:00:00Z", "prerelease": true, "assets": [asset("emu-Linux-x86_64.tar.gz")] },
        ]);
        let found = builds(&list, false, |n| n.contains("Linux"));
        assert_eq!(found.iter().map(|(r, _)| r.tag.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(found[0].1.sha256.as_deref(), Some("ab".repeat(32).as_str()));
        let with_beta = builds(&list, true, |n| n.contains("Linux"));
        assert_eq!(with_beta.iter().map(|(r, _)| r.tag.as_str()).collect::<Vec<_>>(), ["beta", "old"]);
        let one =
            json!({ "tag_name": "v1", "assets": [{ "name": "a.zip", "browser_download_url": "https://x/a.zip" }] });
        assert_eq!(builds(&one, false, |n| n.ends_with(".zip"))[0].1.sha256, None);
    }
}
