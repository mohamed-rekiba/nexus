//! Seam 3, composed host: the core fetches the art the player is likely to see next. The fake
//! network records each address asked for; it serves none of them, so each fetch is a 404.

use std::time::Duration;

use crate::common::*;
use nexus_core::{Command, Grid};
use nexus_ui_host::UiRequest;
use slint_interpreter::ComponentHandle;

/// A store that gives every game a background, a logo and two screenshots at
/// `https://art.example/<title id>-<kind>.png`.
fn art_store() -> nexus_testkit::StubPlugin {
    struct Art;
    #[nexus_api::async_trait]
    impl nexus_api::Metadata for Art {
        async fn describe(
            &self,
            game: &nexus_api::GameRef,
        ) -> nexus_api::PluginResult<Option<nexus_api::GameMetadata>> {
            let url =
                |kind: &str| nexus_api::ImageSource::Url(format!("https://art.example/{}-{kind}.png", game.title_id));
            Ok(Some(nexus_api::GameMetadata {
                background: Some(url("bg")),
                logo: Some(url("logo")),
                screenshots: vec![url("shot1"), url("shot2")],
                ..Default::default()
            }))
        }
    }
    nexus_testkit::StubPlugin::from_toml(
        "id = \"store\"\nkind = \"service\"\nname = \"store\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"metadata\"]\n[permissions]\nnetwork = [\"art.example\"]",
    )
    .provides(|_| Ok(nexus_api::Provided { metadata: Some(std::sync::Arc::new(Art)), ..Default::default() }))
}

/// Twelve games, G00 to G11, whose title ids are CUSA0000<i>: by title, the row's order is
/// their number's.
fn twelve() -> Composed {
    let titles = vec!["G00", "G01", "G02", "G03", "G04", "G05", "G06", "G07", "G08", "G09", "G10", "G11"];
    let mut host = composed(|h| h.plugin(games("emu-a", titles)).plugin(art_store()));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host
}

/// The art addresses asked for so far.
fn asked(host: &Composed) -> Vec<String> {
    host.test.http.sent().into_iter().filter(|u| u.starts_with("https://art.example/CUSA")).collect()
}

fn art(i: usize, kind: &str) -> String {
    format!("https://art.example/CUSA0000{i}-{kind}.png")
}

/// Runs the core for `ms` milliseconds of its clock.
fn run(host: &mut Composed, ms: u64) {
    host.advance(Duration::from_millis(ms));
}

/// Runs the core until `art` was asked for, for at most five seconds of its clock.
fn until_asked(host: &mut Composed, art: &[String]) {
    for _ in 0..500 {
        if art.iter().all(|a| asked(host).contains(a)) {
            return;
        }
        run(host, 10);
    }
    panic!("never asked for {art:?}; asked for {:?}", asked(host));
}

#[test]
fn the_games_beside_the_selection_are_fetched_then_its_screenshots_once_it_rests() {
    let mut host = twelve();
    run(&mut host, 700);
    let before = asked(&host).len();

    host.dispatch(Command::Select("emu-a:G5".parse().unwrap()));
    until_asked(&mut host, &[art(4, "bg"), art(4, "logo"), art(6, "bg"), art(6, "logo")]);
    assert!(!asked(&host).contains(&art(5, "shot1")), "the screenshots wait for the selection to rest");
    assert!(!asked(&host).contains(&art(3, "bg")), "one game each side");

    until_asked(&mut host, &[art(5, "shot1"), art(5, "shot2")]);
    run(&mut host, 300);
    let new = asked(&host).len() - before;
    assert_eq!(new, 6, "each once: {:?}", asked(&host));
}

#[test]
fn a_fast_scroll_fetches_only_around_where_it_stops() {
    let mut host = twelve();
    run(&mut host, 700);
    let before = asked(&host).len();

    // A select every 20 ms, as a held key does.
    let (core, clock) = (host.core().clone(), host.test.clock.clone());
    host.rt.block_on(async {
        for i in 1..11 {
            let core = core.clone();
            tokio::spawn(async move { core.dispatch(Command::Select(format!("emu-a:G{i}").parse().unwrap())).await });
            clock.advance(Duration::from_millis(20)).await;
        }
    });
    run(&mut host, 1000);
    let new: Vec<String> = asked(&host).into_iter().skip(before).collect();
    // Around G10 only: G9's and G11's background and logo, and G10's screenshots.
    assert!(new.len() <= 6, "{new:?}");
    assert!(new.contains(&art(10, "shot1")), "{new:?}");
}

#[test]
fn in_the_library_grid_the_games_above_and_below_are_fetched_too() {
    let mut host = twelve();
    run(&mut host, 700);
    host.dispatch(Command::GridColumns { grid: Grid::Library, columns: 4 });

    host.dispatch(Command::Select("emu-a:G5".parse().unwrap()));
    until_asked(&mut host, &[art(1, "bg"), art(9, "bg"), art(4, "logo"), art(6, "logo")]);
}

#[test]
fn the_library_grid_tells_the_core_its_columns() {
    use slint::platform::Key;
    let mut host = twelve();
    host.resize(1600.0, 900.0);
    settle(&mut host);
    host.requests.borrow_mut().clear();
    // Up to the top bar, right to Library, Enter opens it.
    for key in [Key::UpArrow, Key::RightArrow, Key::Return] {
        let text: slint::SharedString = key.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    settle(&mut host);
    let told =
        host.requests.borrow().iter().any(
            |r| matches!(r, UiRequest::Core(Command::GridColumns { grid: Grid::Library, columns }) if *columns > 1),
        );
    assert!(told, "{:?}", host.requests.borrow());
}
