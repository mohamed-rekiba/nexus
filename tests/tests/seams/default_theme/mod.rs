//! Seam 3: the Default theme, driven through its accessible labels and keys. Its Home is the
//! list of systems; the screens it takes from the Classic theme have the Classic theme's
//! tests.

mod home;
mod library;
mod theme_files;

use crate::common::*;
use nexus_core::Command;
use slint::platform::Key;

/// The Default theme on a library of two emulators: PS4 with Alpha and Beta, PS5 with Gamma.
fn default_theme() -> Composed {
    default_theme_with(|h| h)
}

/// [`default_theme`], with more plugins.
fn default_theme_with(more: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    default_theme_of(|h| {
        more(
            h.plugin(FakeCatalog::new("emu-a").playable().system("PS4").games(["Alpha", "Beta"]).build())
                .plugin(FakeCatalog::new("emu-b").playable().system("PS5").games(["Gamma"]).build()),
        )
    })
}

/// The Default theme on the plugins `build` adds, its library loaded.
fn default_theme_of(build: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    let mut host = composed(build);
    host.ui.start().unwrap();
    host.ui.switch(&nexus_api::PluginId::new(theme_default::ID).unwrap()).unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

/// The titles of the Library's games, as the core has them.
fn library_titles(host: &Composed) -> Vec<String> {
    host.core().view().home.library.games.iter().map(|g| g.title.clone()).collect()
}

/// Lets the cover follow the focus: it waits a moment after the last move.
fn cover_follows(host: &mut Composed) {
    host.wait(std::time::Duration::from_millis(200));
    host.settle();
}

/// Down the list until the system named `name` has the focus. The list follows the core's
/// order of the emulators.
fn focus_on(host: &mut Composed, name: &str) {
    for _ in 0..host.count("systems-tile") {
        if host.focused("systems-tile") == [name] {
            return;
        }
        host.navigate(Key::DownArrow);
    }
    assert_eq!(host.focused("systems-tile"), [name]);
}
