//! JSON from the web, kept in the plugin's storage for a while.
//!
//! GitHub's API allows 60 requests an hour without a token, and the host asks for versions
//! every time the player opens a game, so release lists are fetched at most once per `ttl`.
//! When the web is down, the last copy is used however old it is.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nexus_api::serde_json::{self, Value};
use nexus_api::{HttpRequest, PluginContext, PluginError, PluginResult, RelPath};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The JSON at `url`, from `cache` when it is younger than `ttl`.
pub async fn cached_json(ctx: &PluginContext, url: &str, cache: &RelPath, ttl: Duration) -> PluginResult<Value> {
    let stored: Option<Value> = ctx.storage().read(cache).await.ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh =
        stored.as_ref().and_then(|s| s["fetched"].as_u64()).is_some_and(|t| now().saturating_sub(t) < ttl.as_secs());
    if fresh && let Some(stored) = &stored {
        return Ok(stored["body"].clone());
    }
    match fetch(ctx, url).await {
        Ok(body) => {
            let entry = serde_json::json!({ "fetched": now(), "body": body });
            ctx.storage().write(cache, entry.to_string().as_bytes()).await?;
            Ok(body)
        }
        Err(error) => match stored {
            Some(stored) => {
                ctx.log().warn(&format!("{url}: {error}; using the copy from before"));
                Ok(stored["body"].clone())
            }
            None => Err(error),
        },
    }
}

/// Forgets a cached copy, so the next call fetches.
pub async fn forget(ctx: &PluginContext, cache: &RelPath) -> PluginResult<()> {
    ctx.storage().remove(cache).await
}

async fn fetch(ctx: &PluginContext, url: &str) -> PluginResult<Value> {
    let mut request = HttpRequest::get(url);
    // GitHub's API asks for this.
    request.headers.push(("Accept".into(), "application/vnd.github+json".into()));
    let response = ctx.http().send(request).await?;
    if response.status != 200 {
        return Err(PluginError::Network(format!("HTTP {} from {url}", response.status)));
    }
    serde_json::from_slice(&response.body).map_err(|e| PluginError::Network(format!("{url}: not JSON: {e}")))
}
