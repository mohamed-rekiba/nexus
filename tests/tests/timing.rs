//! Opt-in, `NEXUS_TIMING=1`: how long the Catalog's view takes to build and bind with the
//! bundled catalogs, which are as big as the project's `assets/catalog/` makes them. Prints
//! the times; checks only that the Catalog has games.

mod common;

use std::time::Instant;

use common::*;
use nexus_core::{Command, Route};

#[test]
fn the_catalog_s_view_builds_and_binds_in_time() {
    if std::env::var_os("NEXUS_TIMING").is_none() {
        return;
    }
    let mut host = composed(|h| {
        h.plugin(games_on("emu-kyty", "PS5", vec!["Alpha"]).console("PS5"))
            .plugin(games_on("emu-shadps4", "PS4", vec!["Beta"]).console("PS4"))
            .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    let started = Instant::now();
    host.dispatch(Command::Navigate(Route::Catalog));
    eprintln!("first Catalog open, with reading the catalogs: {:?}", started.elapsed());
    let started = Instant::now();
    host.dispatch(Command::Navigate(Route::Home));
    host.dispatch(Command::Navigate(Route::Catalog));
    eprintln!("Catalog open again: {:?}", started.elapsed());

    let total = host.core().view().catalog.games.total;
    assert!(total > 0, "the bundled catalogs have games");
    let runs = 20;
    let started = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(host.core().view());
    }
    eprintln!("{total} games: core.view() {:?} each", started.elapsed() / runs);
    let started = Instant::now();
    for _ in 0..runs {
        host.ui.render().unwrap();
    }
    eprintln!("ui.render() (view and bind) {:?} each", started.elapsed() / runs);
    let started = Instant::now();
    host.dispatch(Command::CatalogSort("rating".into()));
    host.dispatch(Command::CatalogSearch("the".into()));
    eprintln!("a sort and a search: {:?}", started.elapsed());
}
