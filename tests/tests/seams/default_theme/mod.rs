//! Seam 3: the default theme, driven through its accessible labels and keys, one module per
//! screen. The setup and the steps more than one screen uses are here.

mod catalog;
mod downloads;
mod emulators;
mod game_hub;
mod home;
mod installs;
mod keyboard;
mod library;
mod notices;
mod play;
mod releases;
mod settings;
mod theme_files;
mod trailer;

use crate::common::*;
use nexus_core::{Area, Command, Pref, SessionCmd};
use slint_interpreter::ComponentHandle;

fn default_theme() -> Composed {
    default_theme_with(|h| h)
}

/// [`default_theme`], with more plugins.
fn default_theme_with(more: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    let mut host = composed(|h| {
        more(
            h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
                .plugin(FakeCatalog::new("emu-b").system("PS5").games(["Gamma"]).build()),
        )
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

/// Home's line for the first game, once its details are in.
fn home_line(host: &mut Composed) -> String {
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.until(Area::Library, |core| core.view().detail.is_some());
    host.ui.render().unwrap();
    host.settle();
    let shown = host.values("default-description");
    let [text] = shown.as_slice() else { panic!("one line, got {shown:?}") };
    text.clone()
}

/// The x of the first element labelled `label`.
fn x_of(host: &Composed, label: &str) -> f32 {
    host.first(label).absolute_position().x
}

/// An emulator with no games of its own that installs from the packages folder.
fn installing_emulator() -> nexus_testkit::StubPlugin {
    FakeCatalog::kyty().playable().build()
}

const ASTRO_HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

/// A catalog that offers one release, Astro, open on its Game Hub.
fn astro_on_its_game_hub(home: &std::path::Path) -> Composed {
    astro_with(home, installing_emulator())
}

/// [`astro_on_its_game_hub`] with `emulator` as KytyPS5.
fn astro_with(home: &std::path::Path, emulator: nexus_testkit::StubPlugin) -> Composed {
    let hash = ASTRO_HASH;
    let mut host = composed(|h| {
        h.home(home).plugin(emulator).built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{hash}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }] });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.test.torrents.has(hash, "Astro", &[("Astro/astro.pkg", b"package")]);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
    host.settle();
    host
}

/// Waits until the first download's state is `state`, then carries out what the theme asked
/// for when it saw it (the download sheet opens Downloads once the download is there).
fn until_torrent(host: &mut Composed, state: &str) {
    host.until(Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(|t| t.state == state));
    host.settle();
    host.pump();
    host.settle();
    assert_eq!(host.core().view().downloads.torrents.first().map(|t| t.state.clone()).as_deref(), Some(state));
}

/// A catalog with these topics, open on its first offer's Game Hub.
fn first_offer_with(home: &std::path::Path, topics: serde_json::Value) -> Composed {
    first_offer_on(home, installing_emulator(), |h| h, topics)
}

/// [`first_offer_with`], for `emulator`, on a host that `build` changes first.
fn first_offer_on(
    home: &std::path::Path,
    emulator: nexus_testkit::StubPlugin,
    build: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost,
    topics: serde_json::Value,
) -> Composed {
    let mut host = composed(|h| {
        build(h)
            .home(home)
            .plugin(emulator)
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let count = topics.as_array().unwrap().len();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": count,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    let offer = host.core().view().catalog.games.games.first().cloned().expect("the offer");
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    host.settle();
    host
}

/// Presses `key` and carries out what the theme asked for, as the app would, where the core
/// may refuse: gives the refusals.
fn key_refused(host: &mut Composed, key: slint::platform::Key) -> Vec<String> {
    host.send_key(key);
    let requests: Vec<nexus_ui_host::UiRequest> = host.requests.borrow_mut().drain(..).collect();
    let mut refused = Vec::new();
    for request in requests {
        if let nexus_ui_host::UiRequest::Core(command) = request
            && let Err(e) = host.rt.block_on(host.test.core.dispatch(command))
        {
            refused.push(e.to_string());
        }
    }
    host.ui.render().unwrap();
    host.settle();
    refused
}

/// Presses `key` and gives the core commands the theme asked for, without carrying them out.
fn key_asks(host: &mut Composed, key: slint::platform::Key) -> Vec<Command> {
    host.send_key(key);
    let requests: Vec<nexus_ui_host::UiRequest> = host.requests.borrow_mut().drain(..).collect();
    requests
        .into_iter()
        .filter_map(|r| if let nexus_ui_host::UiRequest::Core(command) = r { Some(command) } else { None })
        .collect()
}
