//! Emulator versions, one folder each under `builds/<tag>`, with the one games start (the
//! current one) and the one before it, for a rollback. Saved in the plugin's `versions.json`.

use nexus_api::serde_json::{self, json};
use nexus_api::{PluginContext, PluginError, PluginResult, RelPath};

const STATE: &str = "versions.json";
pub const BUILDS: &str = "builds";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub current: Option<String>,
    pub previous: Option<String>,
}

fn path(p: &str) -> PluginResult<RelPath> {
    Ok(RelPath::new(p)?)
}

/// The folder of a version.
pub fn folder(tag: &str) -> PluginResult<RelPath> {
    path(BUILDS)?.join(tag).map_err(Into::into)
}

pub async fn state(ctx: &PluginContext) -> State {
    let Ok(bytes) = ctx.storage().read(&RelPath::new(STATE).expect("a plain name")).await else {
        return State::default();
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    State { current: text("current"), previous: text("previous") }
}

async fn save(ctx: &PluginContext, state: &State) -> PluginResult<()> {
    let body = json!({ "current": state.current, "previous": state.previous });
    ctx.storage().write(&path(STATE)?, body.to_string().as_bytes()).await
}

/// Installed versions, the current one last (the host shows the last one as installed).
pub async fn installed(ctx: &PluginContext) -> PluginResult<Vec<String>> {
    let mut tags: Vec<String> =
        ctx.storage().list(&path(BUILDS)?).await?.into_iter().filter(|t| !t.ends_with(".unpacking")).collect();
    let current = state(ctx).await.current;
    tags.sort_by_key(|t| Some(t) == current.as_ref());
    Ok(tags)
}

/// Makes `tag` current; the old current becomes the previous one.
pub async fn activate(ctx: &PluginContext, tag: &str) -> PluginResult<()> {
    let mut s = state(ctx).await;
    if s.current.as_deref() != Some(tag) {
        s.previous = s.current.take();
    }
    s.current = Some(tag.to_owned());
    save(ctx, &s).await
}

/// Goes back to the previous version. Returns it.
pub async fn rollback(ctx: &PluginContext) -> PluginResult<String> {
    let mut s = state(ctx).await;
    let previous =
        s.previous.clone().ok_or_else(|| PluginError::Precondition("no earlier version to go back to".into()))?;
    if !ctx.storage().exists(&folder(&previous)?).await? {
        return Err(PluginError::Precondition(format!("{previous} is no longer installed")));
    }
    s.previous = s.current.replace(previous.clone());
    save(ctx, &s).await?;
    Ok(previous)
}

/// Removes every version but the current and the previous one.
pub async fn prune(ctx: &PluginContext) -> PluginResult<()> {
    let s = state(ctx).await;
    for tag in installed(ctx).await? {
        if Some(&tag) != s.current.as_ref() && Some(&tag) != s.previous.as_ref() {
            ctx.storage().remove(&folder(&tag)?).await?;
        }
    }
    Ok(())
}

/// Removes a version's folder and forgets it.
pub async fn remove(ctx: &PluginContext, tag: &str) -> PluginResult<()> {
    ctx.storage().remove(&folder(tag)?).await?;
    let mut s = state(ctx).await;
    if s.previous.as_deref() == Some(tag) {
        s.previous = None;
    }
    if s.current.as_deref() == Some(tag) {
        s.current = s.previous.take();
    }
    save(ctx, &s).await
}

/// Removes `tag` after its install failed, so a broken build is never listed. The current
/// build stays: games start from it.
pub async fn discard(ctx: &PluginContext, tag: &str) -> PluginResult<()> {
    if state(ctx).await.current.as_deref() == Some(tag) {
        return Ok(());
    }
    remove(ctx, tag).await
}
