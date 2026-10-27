//! Uninstalling an installed game: delete its folders through its plugin's folder grants, or
//! ask its store to remove it.

use nexus_api::GameKey;

use crate::app::{Core, Route};
use crate::changes::Area;
use crate::{CoreError, CoreResult};

pub(crate) async fn uninstall_game(core: &Core, game: &GameKey) -> CoreResult<()> {
    let also;
    let link;
    if core.sessions().runs(game) {
        return Err(CoreError::Invalid("Stop the game before you uninstall it".into()));
    }
    let installs = core.installs().snapshot();
    let (folder, title) = {
        let library = core.library().snapshot();
        if installs.installing(&game.plugin) {
            return Err(CoreError::Invalid("Wait until the install ends".into()));
        }
        let details = library.detail(game).map(|d| &d.details);
        let folder = details.and_then(|d| d.folder.clone());
        also = details.map(|d| d.also.clone()).unwrap_or_default();
        // A store's game: the store removes it (Steam asks in its own dialog).
        link = details.and_then(|d| Some((d.uninstall_link.clone()?, d.summary.platform.clone())));
        (folder, library.title(game))
    };
    if let Some((link, platform)) = link {
        let allowed = core.kernel().manifest(&game.plugin)?.permissions.allows_link(&link);
        if !allowed {
            return Err(CoreError::PermissionDenied { plugin: game.plugin.clone(), action: format!("open {link}") });
        }
        let opening = link.clone();
        crate::running::open_link(core, opening).await.map_err(CoreError::Invalid)?;
        let store = crate::snapshot::Plugins::new(core.kernel()).system(&platform).name;
        tracing::info!(%game, %link, "asked the store to uninstall");
        core.notices().info(format!("{store} is uninstalling {title}: it leaves your Library when it's done"));
        core.changed(Area::Library);
        return Ok(());
    }
    let folder = folder.ok_or_else(|| CoreError::Invalid(format!("{title} has no folder to delete")))?;
    let folders = crate::app::game_folders(core.kernel(), game)
        .ok_or_else(|| CoreError::Invalid(format!("{title} has no folder to delete")))?;
    folders.remove(&folder).await.map_err(|e| CoreError::Plugin { plugin: game.plugin.clone(), error: e })?;
    // Its update and add-ons, in their own folders.
    for other in &also {
        if let Err(e) = folders.remove(other).await {
            tracing::warn!(error = %e, folder = %other.path, "a folder of an uninstalled game stays");
        }
    }
    core.library().forget_detail(game);
    {
        let mut state = core.state();
        state.nav.retain(|r| r != &Route::GameDetail(game.clone()));
        if state.nav.is_empty() {
            state.nav.push(Route::Home);
        }
    }
    crate::library_reads::refresh_library(core).await?;
    core.notices().info(format!("Uninstalled {title}"));
    core.changed(Area::Library);
    Ok(())
}
