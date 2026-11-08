//! Seam 3: the Systems theme, driven through its accessible labels and keys. Its Home is the
//! list of systems; the screens it takes from the default theme have the default theme's
//! tests.

mod home;
mod theme_files;

use crate::common::*;
use nexus_core::Command;
use slint::platform::Key;

/// The Systems theme on a library of two emulators: PS4 with Alpha and Beta, PS5 with Gamma.
fn systems_theme() -> Composed {
    systems_theme_with(|h| h)
}

/// [`systems_theme`], with more plugins.
fn systems_theme_with(more: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    let mut host = composed(|h| {
        more(
            h.built_in(nexus_api::PluginBox::theme(theme_systems::SystemsTheme))
                .plugin(FakeCatalog::new("emu-a").playable().system("PS4").games(["Alpha", "Beta"]).build())
                .plugin(FakeCatalog::new("emu-b").playable().system("PS5").games(["Gamma"]).build()),
        )
    });
    host.ui.start().unwrap();
    host.ui.switch(&nexus_api::PluginId::new(theme_systems::ID).unwrap()).unwrap();
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
