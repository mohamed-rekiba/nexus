//! Seam 3: Home's groups split the library by platform, and selecting a game loads its
//! details while the player stays on the home screen.

use crate::common::*;
use nexus_core::Command;
use nexus_ui_api::ScreenId;

/// Each group of Home as (platform, how many games, titles, the selected game's index).
fn groups(host: &Composed) -> Vec<(String, usize, Vec<String>, Option<usize>)> {
    let view = host.core().view();
    view.home
        .groups
        .iter()
        .map(|g| {
            let titles = g.games.iter().map(|c| c.title.clone()).collect();
            (g.platform.clone(), g.games.len(), titles, g.selected)
        })
        .collect()
}

fn two_platforms() -> Composed {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
            .plugin(FakeCatalog::new("emu-b").system("PS5").games(["Gamma"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host
}

#[test]
fn the_library_is_grouped_by_platform() {
    let host = two_platforms();
    let mut found = groups(&host);
    for g in &mut found {
        g.2.sort();
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        [
            // The first game by title is selected once the library loads.
            ("PS4".to_owned(), 2, vec!["Alpha".to_owned(), "Beta".to_owned()], Some(0)),
            ("PS5".to_owned(), 1, vec!["Gamma".to_owned()], None),
        ]
    );
}

#[test]
fn a_group_points_at_the_selected_game_when_it_holds_it() {
    let mut host = two_platforms();
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    let found = groups(&host);
    let ps4 = found.iter().find(|g| g.0 == "PS4").unwrap();
    assert_eq!(ps4.2[ps4.3.expect("PS4 holds the selected game")], "Beta");
    assert_eq!(found.iter().find(|g| g.0 == "PS5").unwrap().3, None);
}

#[test]
fn selecting_a_game_loads_its_details_and_stays_home() {
    let mut host = two_platforms();
    host.dispatch(Command::Select("emu-b:G0".parse().unwrap()));
    let view = host.core().view();
    assert_eq!(view.screen, ScreenId::Home);
    let detail = view.detail.expect("the game's details");
    assert_eq!(detail.title, "Gamma");
    assert_eq!(detail.description, "About Gamma");
}

fn notices(host: &Composed) -> Vec<String> {
    host.core().view().notices
}

#[test]
fn details_that_failed_to_load_are_not_asked_for_again() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").without_details().games(["Alpha", "Beta"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    for game in ["emu-a:G0", "emu-a:G1", "emu-a:G0"] {
        host.dispatch(Command::Select(game.parse().unwrap()));
    }
    let found = notices(&host);
    assert_eq!(found.len(), 2, "one notice per game, not per selection: {found:?}");
}

#[test]
fn of_quick_selections_only_the_last_loads_its_details() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").without_details().games(["Alpha", "Beta", "Gamma"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // The first game was selected, and its details failed, as the library loaded. The two
    // selections below are of the other games.
    host.dispatch(Command::Notice(nexus_core::NoticeCmd::DismissAll));
    let core = host.core().clone();
    host.rt
        .block_on(async {
            tokio::try_join!(
                core.dispatch(Command::Select("emu-a:G1".parse().unwrap())),
                core.dispatch(Command::Select("emu-a:G2".parse().unwrap())),
            )
        })
        .unwrap();
    host.ui.render().unwrap();
    let found = notices(&host);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("G2"), "{found:?}");
}
