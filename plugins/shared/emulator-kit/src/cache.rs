//! JSON from the web, kept in the plugin's storage for a while.
//!
//! GitHub's API allows 60 requests an hour without a token, and the host asks for versions
//! every time the player opens a game, so release lists are fetched at most once per `ttl`.
//! When the web is down, the last copy is used however old it is.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nexus_api::serde_json::{self, Value};
use nexus_api::{HttpClient, HttpRequest, Logger, PluginError, PluginResult, RelPath, Storage};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The JSON at `url`, from `cache` when it is younger than `ttl`.
pub async fn cached_json(
    storage: &dyn Storage,
    http: &dyn HttpClient,
    log: &Logger,
    url: &str,
    cache: &RelPath,
    ttl: Duration,
) -> PluginResult<Value> {
    let stored: Option<Value> = storage.read(cache).await.ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh =
        stored.as_ref().and_then(|s| s["fetched"].as_u64()).is_some_and(|t| now().saturating_sub(t) < ttl.as_secs());
    if fresh && let Some(stored) = &stored {
        return Ok(stored["body"].clone());
    }
    match fetch(http, url).await {
        Ok(body) => {
            let entry = serde_json::json!({ "fetched": now(), "body": body });
            storage.write(cache, entry.to_string().as_bytes()).await?;
            Ok(body)
        }
        Err(error) => match stored {
            Some(stored) => {
                log.warn(&format!("{url}: {error}; using the copy from before"));
                Ok(stored["body"].clone())
            }
            None => Err(error),
        },
    }
}

/// Forgets a cached copy, so the next call fetches.
pub async fn forget(storage: &dyn Storage, cache: &RelPath) -> PluginResult<()> {
    storage.remove(cache).await
}

/// Whether `url` is on GitHub's REST API.
fn github_api(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.eq_ignore_ascii_case("api.github.com")
}

async fn fetch(http: &dyn HttpClient, url: &str) -> PluginResult<Value> {
    let mut request = HttpRequest::get(url);
    if github_api(url) {
        // GitHub's API asks for this. Other servers need not know the type.
        request.headers.push(("Accept".into(), "application/vnd.github+json".into()));
    }
    let response = http.send(request).await?;
    if response.status != 200 {
        return Err(PluginError::Network(format!("HTTP {} from {url}", response.status)));
    }
    serde_json::from_slice(&response.body).map_err(|e| PluginError::Network(format!("{url}: not JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestHost;

    fn accept(request: &HttpRequest) -> Option<&str> {
        request.headers.iter().find(|(k, _)| k == "Accept").map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn only_github_s_api_gets_github_s_accept_header() {
        let test = TestHost::default();
        let host = test.host();
        let api = "https://api.github.com/repos/o/r/releases?per_page=10";
        let asset = "https://github.com/o/r/releases/latest/download/list.json";
        let other = "https://example.org/list.json";
        for (n, url) in [api, asset, other].into_iter().enumerate() {
            test.http.page(url, "{}");
            let cache = RelPath::new(format!("cache/{n}.json")).unwrap();
            cached_json(host.storage.as_ref(), host.http.as_ref(), &host.log, url, &cache, Duration::from_secs(60))
                .await
                .unwrap();
        }
        let requests = test.http.requests();
        assert_eq!(accept(&requests[0]), Some("application/vnd.github+json"));
        assert_eq!(accept(&requests[1]), None, "a release asset is a plain download");
        assert_eq!(accept(&requests[2]), None);
    }

    #[tokio::test]
    async fn a_fresh_copy_is_not_fetched_again_and_a_stale_one_serves_when_the_web_fails() {
        let test = TestHost::default();
        let host = test.host();
        let url = "https://example.org/list.json";
        let cache = RelPath::new("cache/list.json").unwrap();
        let get = |ttl| cached_json(host.storage.as_ref(), host.http.as_ref(), &host.log, url, &cache, ttl);
        test.http.page(url, r#"{"a":1}"#);
        assert_eq!(get(Duration::from_secs(60)).await.unwrap()["a"], 1);
        assert_eq!(get(Duration::from_secs(60)).await.unwrap()["a"], 1);
        assert_eq!(test.http.requests().len(), 1, "the second call reads the copy");
        // With no time to live the copy is stale; the page now isn't JSON, so the copy serves.
        test.http.page(url, "not json");
        assert_eq!(get(Duration::ZERO).await.unwrap()["a"], 1);
        assert_eq!(test.http.requests().len(), 2);
    }
}
