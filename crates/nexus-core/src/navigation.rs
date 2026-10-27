//! Where the player is and what they look at: the navigation stack, the selections, and the
//! Library's and the Catalog's grids. The state lives in [`crate::app_state::AppState`]; these
//! flows change it and load what a screen needs before it opens.

use nexus_api::{Capability, GameKey, PluginKind};

use crate::app::{Core, Grid, Route};
use crate::changes::Area;
use crate::grid_query::{GRID_PAGE, GridQuery};
use crate::library::DetailState;
use crate::{CoreError, CoreResult, Kernel};

/// How long a selection must stay before its details load.
const SELECT_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// The Catalog shows its "Hidden" filter: unless a plugin that gives catalogs turned it off
/// (its `show_hidden_filter` setting).
pub(crate) fn hidden_filter_on(kernel: &Kernel) -> bool {
    kernel.providers(Capability::Releases).iter().all(|plugin| {
        kernel.config().get(&["plugins", plugin.as_str(), "emulator", "show_hidden_filter"]).as_bool() != Some(false)
    })
}

/// A turned-on plugin gives release catalogs, so there is a Catalog.
pub(crate) fn catalog_available(kernel: &Kernel) -> bool {
    !kernel.providers(Capability::Releases).is_empty()
}

pub(crate) async fn navigate(core: &Core, route: Route) -> CoreResult<()> {
    if route == Route::Catalog && !catalog_available(core.kernel()) {
        return Err(CoreError::Invalid("no game catalog is turned on".into()));
    }
    match &route {
        Route::GameDetail(game) => return open_game(core, game.clone()).await,
        Route::Emulators => crate::emulator_builds::refresh_emulators(core).await,
        Route::Plugin(plugin) => {
            let manifest = core.kernel().manifest(plugin)?;
            // A service's blocks show on its page too.
            if manifest.kind != PluginKind::Theme {
                crate::emulator_builds::refresh_emulator(core, plugin).await;
            }
            crate::plugin_settings::refresh_settings(core).await;
        }
        Route::Settings => {
            core.vpn().refresh().await;
            crate::plugin_settings::refresh_settings(core).await;
        }
        // Read once; a new catalog address reads them again. The Catalog opens at once and
        // says it is loading until the read ends.
        Route::Catalog if !core.library().has_catalogs() => {
            push(core, route);
            crate::catalog_reads::refresh_releases(core).await;
            return Ok(());
        }
        Route::GameSettings(game) if core.library().snapshot().detail(game).is_none() => {
            crate::library_reads::load_detail(core, game).await?
        }
        _ => {}
    }
    push(core, route);
    Ok(())
}

pub(crate) fn push(core: &Core, route: Route) {
    {
        let mut state = core.state();
        // One plugin's page at a time: opening another replaces it.
        if matches!(route, Route::Plugin(_)) && matches!(state.nav.last(), Some(Route::Plugin(_))) {
            state.nav.pop();
        }
        if state.nav.last() != Some(&route) {
            state.nav.push(route);
        }
    }
    core.changed(Area::Navigation);
}

/// Searches the Catalog, and shows it.
pub(crate) async fn search(core: &Core, text: String) -> CoreResult<()> {
    if !core.library().has_catalogs() {
        crate::catalog_reads::refresh_releases(core).await;
    }
    set_grid(core, Grid::Catalog, |v| v.query = text)?;
    push(core, Route::Catalog);
    Ok(())
}

/// Changes the Library's or the Catalog's view, and goes back to its first page.
pub(crate) fn set_grid(core: &Core, grid: Grid, change: impl FnOnce(&mut GridQuery)) -> CoreResult<()> {
    {
        let mut state = core.state();
        let view = state.grid_view(grid);
        change(view);
        view.shown = GRID_PAGE;
    }
    core.changed(Area::Navigation);
    if grid == Grid::Catalog {
        crate::catalog_reads::describe_catalog_page(core);
    }
    Ok(())
}

/// The next page of the Library or the Catalog.
pub(crate) fn more(core: &Core, grid: Grid) -> CoreResult<()> {
    core.state().grid_view(grid).shown += GRID_PAGE;
    core.changed(Area::Navigation);
    if grid == Grid::Catalog {
        crate::catalog_reads::describe_catalog_page(core);
    }
    Ok(())
}

/// Selects a game: a game the catalogs offer in the Catalog, any other on Home.
fn select(core: &Core, game: &GameKey) {
    let offered = core.library().snapshot().offer(game).is_some();
    let mut state = core.state();
    if offered {
        state.catalog_selected = Some(game.clone());
    } else {
        state.selected = Some(game.clone());
    }
}

pub(crate) async fn open_game(core: &Core, game: GameKey) -> CoreResult<()> {
    // A game a catalog offers: its download sheet names the VPN in use.
    if core.vpn().only() && core.library().snapshot().offer(&game).is_some() {
        core.vpn().refresh().await;
    }
    crate::library_reads::load_detail(core, &game).await?;
    select(core, &game);
    crate::disks::refresh(core);
    push(core, Route::GameDetail(game));
    Ok(())
}

/// Back to the screen before, unless this is the first.
pub(crate) fn back(core: &Core) -> CoreResult<()> {
    {
        let mut state = core.state();
        if state.nav.len() > 1 {
            state.nav.pop();
        }
    }
    core.changed(Area::Navigation);
    Ok(())
}

/// Selects a game, fetches the art around it, and once the selection rests, loads its
/// details.
pub(crate) async fn select_game(core: &Core, game: GameKey) -> CoreResult<()> {
    select(core, &game);
    core.changed(Area::Navigation);
    crate::prefetch::prefetch_around(core, &game);
    // Themes show the selected game's details on the home screen, so load them once. Wait
    // first: a player who scrolls past a game doesn't need its details. A plugin that fails
    // here leaves one notice; the selection stands.
    let wanted = matches!(core.library().snapshot().detail_state(&game), DetailState::Unread);
    if !wanted {
        return Ok(());
    }
    core.ports().clock.sleep(SELECT_SETTLE).await;
    let still = {
        let state = core.state();
        state.selected.as_ref() == Some(&game) || state.catalog_selected.as_ref() == Some(&game)
    };
    if !still {
        return Ok(());
    }
    match crate::library_reads::load_detail(core, &game).await {
        Ok(()) => core.changed(Area::Library),
        Err(e) => {
            core.library().detail_failed(&game);
            core.notices().problem(e.to_string());
        }
    }
    Ok(())
}

/// Opens a game's page, with no download error left from before.
pub(crate) async fn open_game_anew(core: &Core, game: GameKey) -> CoreResult<()> {
    core.torrents().set_download_error(None);
    open_game(core, game).await
}

/// How many columns a grid shows.
pub(crate) fn set_columns(core: &Core, grid: Grid, columns: usize) -> CoreResult<()> {
    core.state().grid_columns.insert(grid, columns);
    Ok(())
}

/// Closes the notice `id`, and opens what it is about.
pub(crate) async fn open_notice(core: &Core, id: u64) -> CoreResult<()> {
    match core.notices().take(id).and_then(|n| n.route) {
        Some(route) => navigate(core, route).await,
        None => Ok(()),
    }
}
