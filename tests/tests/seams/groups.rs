//! Seam 3: `Vm.home.groups` splits the library by platform, and selecting a game loads its
//! details while the player stays on the home screen.

use crate::common::*;
use nexus_core::Command;
use slint::Model;
use slint_interpreter::Value;

fn field(value: &Value, name: &str) -> Value {
    match value {
        Value::Struct(s) => s.get_field(name).cloned().unwrap_or_else(|| panic!("no field {name}")),
        other => panic!("{other:?} is not a struct"),
    }
}

fn rows(value: &Value) -> Vec<Value> {
    match value {
        Value::Model(m) => m.iter().collect(),
        other => panic!("{other:?} is not a list"),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        other => panic!("{other:?} is not text"),
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => *n,
        other => panic!("{other:?} is not a number"),
    }
}

/// Each group as (platform, count, titles, selected).
fn groups(host: &Composed) -> Vec<(String, f64, Vec<String>, f64)> {
    let home = host.instance().get_global_property("Vm", "home").unwrap();
    rows(&field(&home, "groups"))
        .iter()
        .map(|g| {
            let titles = rows(&field(g, "games")).iter().map(|c| text(&field(c, "title"))).collect();
            (text(&field(g, "platform")), number(&field(g, "count")), titles, number(&field(g, "selected")))
        })
        .collect()
}

fn two_platforms() -> Composed {
    let mut host = composed(|h| {
        h.plugin(games_on("emu-a", "PS4", vec!["Alpha", "Beta"])).plugin(games_on("emu-b", "PS5", vec!["Gamma"]))
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
            ("PS4".to_owned(), 2.0, vec!["Alpha".to_owned(), "Beta".to_owned()], 0.0),
            ("PS5".to_owned(), 1.0, vec!["Gamma".to_owned()], -1.0),
        ]
    );
}

#[test]
fn a_group_points_at_the_selected_game_when_it_holds_it() {
    let mut host = two_platforms();
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    let found = groups(&host);
    let ps4 = found.iter().find(|g| g.0 == "PS4").unwrap();
    assert_eq!(ps4.2[ps4.3 as usize], "Beta");
    assert_eq!(found.iter().find(|g| g.0 == "PS5").unwrap().3, -1.0);
}

#[test]
fn selecting_a_game_loads_its_details_and_stays_home() {
    let mut host = two_platforms();
    host.dispatch(Command::Select("emu-b:G0".parse().unwrap()));
    assert_eq!(host.instance().get_global_property("Vm", "screen").unwrap(), Value::String("home".into()));
    assert_eq!(host.instance().get_global_property("Vm", "has-detail").unwrap(), Value::Bool(true));
    assert_eq!(text(&vm_field(&host, "detail", "title")), "Gamma");
    assert_eq!(text(&vm_field(&host, "detail", "description")), "About Gamma");
}

fn notices(host: &Composed) -> Vec<String> {
    rows(&host.instance().get_global_property("Vm", "notices").unwrap()).iter().map(text).collect()
}

#[test]
fn details_that_failed_to_load_are_not_asked_for_again() {
    let mut host = composed(|h| h.plugin(games_without_details("emu-a", vec!["Alpha", "Beta"])));
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
    let mut host = composed(|h| h.plugin(games_without_details("emu-a", vec!["Alpha", "Beta", "Gamma"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // The first game was selected, and its details failed, as the library loaded. The two
    // selections below are of the other games.
    host.dispatch(Command::DismissNotices);
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
